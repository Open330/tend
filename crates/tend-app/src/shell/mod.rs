//! The Windows shell: one instance per user, the notification-area icon
//! and its menu, the Home window (`WebView2` served by the app's scoped
//! proxy), toasts, and the start-with-Windows entry.

mod autostart;
mod toast;

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use wry::http::{header, Response as HttpResponse};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

use crate::client::Client;
use crate::identity;
use crate::mark::{self, Rooms};
use crate::poller::{Poller, Report};
use crate::proxy;
use crate::queue::Toast;
use crate::settings;
use crate::Command;

/// Light ink for the dimmed rooms and the working room.
const INK: [u8; 3] = [0xE8, 0xE6, 0xE3];
const OPEN_REQUEST: &str = "open-request";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Open(Option<String>),
    Poll(Report),
}

pub fn main(command: Command) -> ExitCode {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("tend-app.exe"));
    let app_id = identity::app_id();
    match command {
        Command::Install => {
            if let Err(error) = toast::register(&app_id).and_then(|()| autostart::set(&exe, true)) {
                log(&format!("install failed: {error}"));
                return ExitCode::FAILURE;
            }
            log("installed: starts with Windows");
            run(None, &exe, app_id)
        }
        Command::Uninstall => {
            let result = autostart::set(&exe, false).and_then(|()| toast::unregister(&app_id));
            log(&format!("uninstalled: {result:?}"));
            if result.is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Command::Run { open } => run(open, &exe, app_id),
        Command::Version | Command::Help => ExitCode::SUCCESS,
    }
}

/// Append one line to the app's log (`app.log` in the state directory).
/// The end-to-end test reads it; it never holds the token or agent text
/// beyond an item's key.
pub(crate) fn log(line: &str) {
    let dir = identity::state_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("app.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let _ = writeln!(file, "{now} {line}");
    }
}

/// Hold the per-user lock, or hand `open` to the instance that does.
fn single_instance(dir: &Path, open: Option<&Option<String>>) -> Option<File> {
    let _ = std::fs::create_dir_all(dir);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("app.lock"))
        .ok()?;
    if lock.try_lock().is_ok() {
        return Some(lock);
    }
    if let Some(item) = open {
        let _ = std::fs::write(dir.join(OPEN_REQUEST), item.clone().unwrap_or_default());
    }
    None
}

fn home_url(item: Option<&str>) -> String {
    match item {
        Some(item) if !item.is_empty() => {
            let encoded: String = url::form_urlencoded::byte_serialize(item.as_bytes()).collect();
            format!("{}/home#item={encoded}", proxy::ORIGIN)
        }
        _ => format!("{}/home", proxy::ORIGIN),
    }
}

fn open_in_browser(url: &str) {
    // Validated http(s) only (proxy::outside_target); explorer hands it to
    // the default browser without a shell parsing it.
    let _ = std::process::Command::new("explorer.exe").arg(url).spawn();
}

fn tray_icon(rooms: Rooms) -> Option<tray_icon::Icon> {
    tray_icon::Icon::from_rgba(mark::rgba(32, rooms, INK), 32, 32).ok()
}

fn start_tendd(exe: &Path) {
    use std::os::windows::process::CommandExt;
    let sibling = exe.with_file_name("tend.exe");
    let tend = if sibling.is_file() {
        sibling
    } else {
        PathBuf::from("tend")
    };
    let result = std::process::Command::new(tend)
        .args(["daemon", "start"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    log(&format!("start tendd: {result:?}"));
}

struct Home {
    window: Window,
    webview: WebView,
    _context: WebContext,
}

#[allow(clippy::too_many_lines)] // one builder chain
fn open_home(
    target: &EventLoopWindowTarget<UserEvent>,
    home: &mut Option<Home>,
    item: Option<&str>,
) {
    let url = home_url(item);
    if let Some(open) = home {
        let _ = open.webview.load_url(&url);
        open.window.set_minimized(false);
        open.window.set_focus();
        log(&format!("home focused {}", item.unwrap_or("")));
        return;
    }
    let dashboard = match settings::load() {
        Ok(dashboard) => dashboard,
        Err(error) => {
            log(&format!("home not opened: {error}"));
            return;
        }
    };
    let Ok(window) = WindowBuilder::new()
        .with_title(identity::DISPLAY_NAME)
        .with_inner_size(tao::dpi::LogicalSize::new(520.0, 860.0))
        .with_window_icon(
            tao::window::Icon::from_rgba(
                mark::rgba(64, Rooms::default(), [0x5F, 0x63, 0x68]),
                64,
                64,
            )
            .ok(),
        )
        .build(target)
    else {
        return;
    };
    let addr = dashboard.addr;
    let client = Arc::new(Client::new(dashboard));
    let mut context = WebContext::new(Some(identity::state_dir().join("webview2")));
    let built = WebViewBuilder::new_with_web_context(&mut context)
        .with_asynchronous_custom_protocol(proxy::SCHEME.into(), move |_id, request, responder| {
            let client = client.clone();
            std::thread::spawn(move || {
                let method = request.method().as_str().to_owned();
                let path = request
                    .uri()
                    .path_and_query()
                    .map_or_else(|| "/".to_owned(), |p| p.as_str().to_owned());
                let content_type = request
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let reply = proxy::handle(
                    &client,
                    &method,
                    &path,
                    content_type.as_deref(),
                    request.body(),
                );
                let mut response = HttpResponse::builder().status(reply.status);
                if let Some(value) = reply.content_type {
                    response = response.header(header::CONTENT_TYPE, value);
                }
                response = response.header(
                    header::CACHE_CONTROL,
                    reply.cache_control.unwrap_or_else(|| "no-store".into()),
                );
                if let Ok(response) = response.body(reply.body) {
                    responder.respond(response);
                }
            });
        })
        .with_navigation_handler(move |url| {
            if proxy::stays_in_window(&url) {
                return true;
            }
            if let Some(outside) = proxy::outside_target(&url, addr) {
                open_in_browser(&outside);
            }
            false
        })
        .with_new_window_req_handler(move |url, _features| {
            if let Some(outside) = proxy::outside_target(&url, addr) {
                open_in_browser(&outside);
            }
            NewWindowResponse::Deny
        })
        .with_devtools(false)
        .with_browser_accelerator_keys(false)
        .with_default_context_menus(false)
        .with_url(&url)
        .build(&window);
    match built {
        Ok(webview) => {
            log(&format!("home opened {}", item.unwrap_or("")));
            *home = Some(Home {
                window,
                webview,
                _context: context,
            });
        }
        Err(error) => {
            log(&format!(
                "WebView2 failed ({error}); opening Home in the browser"
            ));
            open_in_browser(&format!("http://{addr}/home"));
        }
    }
}

#[allow(clippy::too_many_lines, clippy::option_option)] // one event loop
fn run(open: Option<Option<String>>, exe: &Path, app_id: String) -> ExitCode {
    let state = identity::state_dir();
    let Some(_lock) = single_instance(&state, open.as_ref()) else {
        // Another instance runs; it was handed the open request.
        return ExitCode::SUCCESS;
    };
    log(&format!("started pid {}", std::process::id()));
    // Toasts need the AppUserModelID registered; doing it on every start
    // keeps the display name right after a move or rename.
    if let Err(error) = toast::register(&app_id) {
        log(&format!("toast registration failed: {error}"));
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy_events = event_loop.create_proxy();
    {
        let events = proxy_events.clone();
        MenuEvent::set_event_handler(Some(move |event| {
            let _ = events.send_event(UserEvent::Menu(event));
        }));
        let events = proxy_events.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            let _ = events.send_event(UserEvent::Tray(event));
        }));
    }

    let menu = Menu::new();
    let open_item = MenuItem::new("Open Home", true, None);
    let notify_item = CheckMenuItem::new("Notifications", true, true, None);
    let autostart_item =
        CheckMenuItem::new("Start with Windows", true, autostart::enabled(exe), None);
    let start_item = MenuItem::new("Start tendd", false, None);
    let quit_item = MenuItem::new("Quit Tend", true, None);
    let _ = menu.append_items(&[
        &open_item,
        &PredefinedMenuItem::separator(),
        &notify_item,
        &autostart_item,
        &start_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ]);
    let mut tray: Option<TrayIcon> = None;

    spawn_poller(
        proxy_events.clone(),
        state.join("app-state.json"),
        state.join(OPEN_REQUEST),
    );

    let mut home: Option<Home> = None;
    let mut toasts = toast::Toasts::new(app_id);
    let mut pending_open = open;
    let exe = exe.to_path_buf();
    let mut tried_daemon = false;

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                tray = TrayIconBuilder::new()
                    .with_menu(Box::new(menu.clone()))
                    .with_menu_on_left_click(false)
                    .with_tooltip("Tend")
                    .with_icon(tray_icon(Rooms::default()).expect("the mark renders"))
                    .build()
                    .ok();
                if let Some(item) = pending_open.take() {
                    open_home(target, &mut home, item.as_deref());
                }
            }
            Event::UserEvent(UserEvent::Tray(TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            })) => open_home(target, &mut home, None),
            Event::UserEvent(UserEvent::Menu(event)) => {
                if event.id == open_item.id() {
                    open_home(target, &mut home, None);
                } else if event.id == autostart_item.id() {
                    let on = autostart_item.is_checked();
                    if let Err(error) = autostart::set(&exe, on) {
                        log(&format!("start with Windows: {error}"));
                    }
                    autostart_item.set_checked(autostart::enabled(&exe));
                } else if event.id == start_item.id() {
                    start_tendd(&exe);
                } else if event.id == quit_item.id() {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::Open(item)) => {
                open_home(target, &mut home, item.as_deref());
            }
            Event::UserEvent(UserEvent::Poll(report)) => {
                if let Some(tray) = &tray {
                    let _ = tray.set_icon(tray_icon(report.rooms()));
                    let _ = tray.set_tooltip(Some(report.tooltip()));
                }
                let down = report
                    .problem
                    .as_deref()
                    .is_some_and(|p| p.starts_with("tendd is not running"));
                start_item.set_enabled(down);
                if down && !tried_daemon {
                    // Once per run: the session just started, or tendd
                    // stopped. Later starts are the person's choice.
                    tried_daemon = true;
                    start_tendd(&exe);
                }
                for key in &report.changes.gone {
                    toasts.withdraw(&identity::toast_tag(key));
                }
                if notify_item.is_checked() {
                    for toast in report.changes.toasts {
                        show_toast(&mut toasts, &proxy_events, toast);
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                // Closing Home frees WebView2; the app stays in the tray.
                home = None;
                log("home closed");
            }
            _ => {}
        }
    });
}

fn show_toast(toasts: &mut toast::Toasts, events: &EventLoopProxy<UserEvent>, toast: Toast) {
    let (tag, title, body, attribution, item, key) = match toast {
        Toast::One(item) => (
            identity::toast_tag(&item.key),
            item.heading().to_owned(),
            item.title.clone(),
            item.host.clone(),
            Some(item.id.clone()),
            item.key.clone(),
        ),
        Toast::Summary(count) => (
            "summary".to_owned(),
            format!("{count} things need you"),
            "Open Home to answer and decide.".to_owned(),
            identity::DISPLAY_NAME.to_owned(),
            None,
            format!("summary:{count}"),
        ),
    };
    let events = events.clone();
    let clicked = item.clone();
    match toasts.show(&tag, &title, &body, &attribution, move || {
        log("toast clicked");
        let _ = events.send_event(UserEvent::Open(clicked.clone()));
    }) {
        Ok(()) => log(&format!("toast {key}")),
        Err(error) => log(&format!("toast failed {key}: {error}")),
    }
}

fn spawn_poller(events: EventLoopProxy<UserEvent>, seen_path: PathBuf, open_request: PathBuf) {
    std::thread::spawn(move || {
        let reports = events.clone();
        Poller { seen_path }.run(
            move |report| reports.send_event(UserEvent::Poll(report)).is_ok(),
            move || {
                if let Ok(item) = std::fs::read_to_string(&open_request) {
                    let _ = std::fs::remove_file(&open_request);
                    let item = Some(item.trim().to_owned()).filter(|i| !i.is_empty());
                    let _ = events.send_event(UserEvent::Open(item));
                }
            },
        );
    });
}
