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
