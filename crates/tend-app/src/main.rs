//! Tend for Windows: a tray app around a local tendd's Home.
//!
//! Design and its review: `docs/proposals/WINDOWS_APP.md`. The modules
//! outside `windows` are platform-neutral and tested on every CI host; the
//! shell itself (event loop, `WebView2`, tray, toasts, registry) builds only
//! on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]
// Off Windows only the tests use the core; the shell that drives it is
// Windows-only.
#![cfg_attr(not(windows), allow(dead_code))]

mod client;
mod dispatch;
mod identity;
mod mark;
mod poller;
mod proxy;
mod queue;
mod settings;
#[cfg(windows)]
mod shell;

// The exe icon resource build.rs links in, and the PNG the toast
// registration points at; tested here, where tests run on every CI host.
#[cfg(any(test, windows))]
#[cfg_attr(not(test), allow(dead_code))]
mod icon_res;

#[cfg(test)]
mod icon_res_tests {
    use super::icon_res::{largest_png, res_from_ico};

    #[test]
    fn the_toast_icon_is_the_largest_png_in_the_checked_in_icon() {
        let ico = include_bytes!("../assets/tend.ico");
        let png = largest_png(ico).expect("tend.ico holds PNG images");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        // IHDR width, big-endian, right after the signature and chunk header.
        assert_eq!(
            u32::from_be_bytes([png[16], png[17], png[18], png[19]]),
            256
        );
        assert_eq!(largest_png(b"not an icon"), None);
        assert_eq!(largest_png(&[0, 0, 1, 0, 1, 0]), None);
    }

    fn u16_at(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }

    fn u32_at(b: &[u8], at: usize) -> usize {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
    }

    /// (type, name, data) of every entry in a `.res` built with numeric ids.
    fn entries(res: &[u8]) -> Vec<(u16, u16, &[u8])> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < res.len() {
            let size = u32_at(res, at);
            let header = u32_at(res, at + 4);
            assert_eq!(header, 32);
            assert_eq!(u16_at(res, at + 8), 0xFFFF);
            assert_eq!(u16_at(res, at + 12), 0xFFFF);
            let data = &res[at + header..at + header + size];
            out.push((u16_at(res, at + 10), u16_at(res, at + 14), data));
            at += header + size.div_ceil(4) * 4;
        }
        out
    }

    #[test]
    fn the_checked_in_icon_becomes_one_group_of_all_its_images() {
        let ico = include_bytes!("../assets/tend.ico");
        let res = res_from_ico(ico).expect("tend.ico parses");
        let all = entries(&res);
        assert_eq!(
            (all[0].0, all[0].1, all[0].2.len()),
            (0, 0, 0),
            "leading empty entry"
        );
        let count = usize::from(u16_at(ico, 4));
        let icons: Vec<_> = all.iter().filter(|e| e.0 == 3).collect();
        assert_eq!(icons.len(), count);
        let groups: Vec<_> = all.iter().filter(|e| e.0 == 14).collect();
        assert_eq!(groups.len(), 1);
        let group = groups[0].2;
        assert_eq!((u16_at(group, 0), u16_at(group, 2)), (0, 1));
        assert_eq!(usize::from(u16_at(group, 4)), count);
        for (i, icon) in icons.iter().enumerate() {
            let dir = 6 + 16 * i;
            let image = &ico[u32_at(ico, dir + 12)..][..u32_at(ico, dir + 8)];
            assert_eq!(icon.2, image, "image {i} copied unchanged");
            let entry = 6 + 14 * i;
            assert_eq!(&group[entry..entry + 12], &ico[dir..dir + 12]);
            assert_eq!(usize::from(u16_at(group, entry + 12)), i + 1);
            assert_eq!(usize::from(icon.1), i + 1);
        }
        // 256 px is written as 0 in both directories.
        assert!(ico[6..].chunks(16).take(count).any(|d| d[0] == 0));
    }

    #[test]
    fn a_file_that_is_not_an_icon_is_refused() {
        assert!(res_from_ico(b"").is_none());
        assert!(res_from_ico(&[0, 0, 2, 0, 1, 0]).is_none(), "a cursor file");
        assert!(
            res_from_ico(&[0, 0, 1, 0, 1, 0]).is_none(),
            "an entry past the end"
        );
    }
}

const USAGE: &str = "\
usage: tend-app [--background] [--open [ITEM]] [--install | --uninstall] [--version]

  (no flag)      start in the tray and open Home
  --background   start in the tray only (what the start-with-Windows entry runs)
  --open ITEM    open Home on one card (a running app takes it over)
  --install      start with Windows and register toasts, then start in the tray
  --uninstall    stop starting with Windows and unregister toasts";

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Command {
    Run { open: Option<Option<String>> },
    Install,
    Uninstall,
    Version,
    Help,
}

fn parse(args: &[String]) -> Result<Command, String> {
    let mut open = Some(None);
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--background" => open = None,
            "--open" => {
                let item = iter.next_if(|next| !next.starts_with("--")).cloned();
                open = Some(item);
            }
            "--install" => return Ok(Command::Install),
            "--uninstall" => return Ok(Command::Uninstall),
            "--version" | "-V" => return Ok(Command::Version),
            "--help" | "-h" => return Ok(Command::Help),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Command::Run { open })
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse(&args) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("tend-app: {error}\n{USAGE}");
            return std::process::ExitCode::from(2);
        }
    };
    match command {
        Command::Version => {
            println!("tend-app {}", env!("CARGO_PKG_VERSION"));
            std::process::ExitCode::SUCCESS
        }
        Command::Help => {
            println!("{USAGE}");
            std::process::ExitCode::SUCCESS
        }
        #[cfg(windows)]
        other => shell::main(other),
        #[cfg(not(windows))]
        _ => {
            eprintln!(
                "tend-app runs on Windows; on macOS use Tend.app, elsewhere the dashboard's /home"
            );
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_command_line() {
        assert_eq!(parse(&args(&[])), Ok(Command::Run { open: Some(None) }));
        assert_eq!(
            parse(&args(&["--background"])),
            Ok(Command::Run { open: None })
        );
        assert_eq!(
            parse(&args(&["--open", "rq_1"])),
            Ok(Command::Run {
                open: Some(Some("rq_1".into()))
            })
        );
        assert_eq!(
            parse(&args(&["--open", "--background"])),
            Ok(Command::Run { open: None })
        );
        assert_eq!(parse(&args(&["--install"])), Ok(Command::Install));
        assert!(parse(&args(&["--nope"])).is_err());
    }
}
