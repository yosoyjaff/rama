//! `rama <path>` from a terminal, as `code <path>` works for VS Code.
//!
//! The path opens as a tab. If Rama is already running, the path goes to that
//! instance, which opens it and comes to the front, and this process ends: one
//! window owns the saved tabs, not two racing to write them. Otherwise this
//! process becomes the app — on Linux and macOS first letting go of the
//! terminal, so the prompt comes back and closing the terminal leaves the
//! window open. (A Windows build is a GUI program; the shell never waits.)
//!
//! The running instance listens on a loopback port it writes, with a random
//! token, to [`crate::settings::instance_path`]. A message is the token and a
//! path, one line each; anything else is dropped.
//!
//! Only a launch WITH a path is handed over. A bare `rama` starts as it always
//! did — which is also what the updater's restart relies on, starting the new
//! version while the old one is still quitting.

use std::hash::{BuildHasher as _, Hasher as _};

use gpui_kit::AppContext as _;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a launch waits on an instance that may no longer exist.
const HANDOFF_TIMEOUT: Duration = Duration::from_millis(500);

/// Set on the copy of this process that stays behind as the app, so it does
/// not detach again.
#[cfg(unix)]
const DETACHED_ENV: &str = "RAMA_DETACHED";

/// What the command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// The repository to open: the root of the checkout holding the path.
    pub repo: Option<String>,
}

/// What the process does after reading its arguments.
#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Run {
        path: Option<PathBuf>,
        foreground: bool,
    },
    Help,
    Version,
    Error(String),
}

const USAGE: &str = "\
Usage: rama [path] [--foreground]

Open the Git repository that holds <path> in Rama.
If Rama is already running, the repository opens there as a new tab.

  -f, --foreground  Stay attached to the terminal
  -h, --help        Show this help
  -v, --version     Show the version";

fn parse(args: impl IntoIterator<Item = String>) -> Parsed {
    let mut path = None;
    let mut foreground = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "-v" | "--version" => return Parsed::Version,
            "-f" | "--foreground" => foreground = true,
            flag if flag.starts_with('-') && flag.len() > 1 => {
                return Parsed::Error(format!("unknown option: {flag}"));
            }
            _ if path.is_some() => return Parsed::Error("only one path can be opened".into()),
            _ => path = Some(PathBuf::from(arg)),
        }
    }
    Parsed::Run { path, foreground }
}

/// Read the command line. Returns what the app should open, or `None` when
/// this process has nothing left to do: help printed, an error reported, or
/// the path handed to the running instance or to a detached copy.
pub fn start() -> Option<Launch> {
    let (path, foreground) = match parse(std::env::args().skip(1)) {
        Parsed::Run { path, foreground } => (path, foreground),
        Parsed::Help => {
            println!("{USAGE}");
            return None;
        }
        Parsed::Version => {
            println!("rama {}", env!("CARGO_PKG_VERSION"));
            return None;
        }
        Parsed::Error(message) => {
            eprintln!("rama: {message}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let repo = match path {
        None => None,
        Some(path) => match crate::git::open::repository_at(&path) {
            Some(repo) => Some(repo),
            None => {
                eprintln!("rama: {} is not inside a Git repository", path.display());
                std::process::exit(1);
            }
        },
    };
    if let Some(repo) = &repo
        && hand_over(&crate::settings::instance_path(), repo)
    {
        return None;
    }
    if !foreground && repo.is_some() && detach() {
        return None;
    }
    Some(Launch { repo })
}

/// Give `repo` to the running instance. `false` when there is none, or it
/// did not answer.
fn hand_over(instance: &Path, repo: &str) -> bool {
    let Some((port, token)) = std::fs::read_to_string(instance)
        .ok()
        .and_then(|text| read_instance(&text))
    else {
        return false;
    };
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, HANDOFF_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(HANDOFF_TIMEOUT));
    if writeln!(stream, "{token}\n{repo}").is_err() {
        return false;
    }
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).is_ok() && answer.trim() == "ok"
}

/// `<port> <token>`, as [`serve`] writes it.
fn read_instance(text: &str) -> Option<(u16, String)> {
    let (port, token) = text.trim().split_once(' ')?;
    Some((port.parse().ok()?, token.to_owned()))
}

/// Start a copy of this process cut loose from the terminal, and report
/// whether it started. That copy is the app; this one exits.
#[cfg(unix)]
fn detach() -> bool {
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};
    if std::env::var_os(DETACHED_ENV).is_some() {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(DETACHED_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own process group: Ctrl+C and the terminal closing reach the
        // shell's jobs, and this is not one of them.
        .process_group(0)
        .spawn()
        .is_ok()
}

#[cfg(not(unix))]
fn detach() -> bool {
    false
}

/// Listen for paths from later launches, and hand each to `open` on the main
/// thread. Best effort: without the port, a launch just opens a second window.
pub fn serve(cx: &mut gpui_kit::App, open: impl Fn(String, &mut gpui_kit::App) + 'static) {
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else {
        return;
    };
    let Ok(port) = listener.local_addr().map(|address| address.port()) else {
        return;
    };
    let token = new_token();
    if write_instance(&crate::settings::instance_path(), port, &token).is_err() {
        return;
    }
    let listener = std::sync::Arc::new(listener);
    cx.spawn(async move |cx| {
        loop {
            let listener = listener.clone();
            let token = token.clone();
            let received = cx
                .background_spawn(async move { receive(&listener, &token) })
                .await;
            match received {
                Ok(Some(repo)) => cx.update(|cx| open(repo, cx)),
                Ok(None) => {}
                // The listener itself failed: stop rather than spin.
                Err(_) => return,
            }
        }
    })
    .detach();
}

/// Take one connection: its path, when it carries the token.
fn receive(listener: &TcpListener, token: &str) -> std::io::Result<Option<String>> {
    let (stream, _) = listener.accept()?;
    let _ = stream.set_read_timeout(Some(HANDOFF_TIMEOUT));
    let mut reader = BufReader::new(&stream);
    let (mut sent_token, mut repo) = (String::new(), String::new());
    if reader.read_line(&mut sent_token).is_err() || sent_token.trim_end() != token {
        return Ok(None);
    }
    if reader.read_line(&mut repo).is_err() {
        return Ok(None);
    }
    let repo = repo.trim_end_matches(['\r', '\n']).to_owned();
    if repo.is_empty() {
        return Ok(None);
    }
    let _ = (&stream).write_all(b"ok\n");
    Ok(Some(repo))
}

/// Write `<port> <token>` where only this user can read it.
fn write_instance(path: &Path, port: u16, token: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    write!(file, "{port} {token}")
}

/// 128 random bits, from the standard library's randomly keyed hasher.
fn new_token() -> String {
    let half = || {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u32(std::process::id());
        hasher.finish()
    };
    format!("{:016x}{:016x}", half(), half())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn a_path_and_flags_are_read() {
        assert_eq!(
            parse(args(&[".", "--foreground"])),
            Parsed::Run {
                path: Some(PathBuf::from(".")),
                foreground: true
            }
        );
        assert_eq!(
            parse(args(&[])),
            Parsed::Run {
                path: None,
                foreground: false
            }
        );
        assert_eq!(parse(args(&["-h"])), Parsed::Help);
        assert_eq!(parse(args(&["--version"])), Parsed::Version);
        assert!(matches!(parse(args(&["--nope"])), Parsed::Error(_)));
        assert!(matches!(parse(args(&["a", "b"])), Parsed::Error(_)));
    }

    #[test]
    fn a_path_reaches_the_running_instance() {
        let dir = tempfile::tempdir().unwrap();
        let instance = dir.path().join("instance");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        write_instance(&instance, port, "secret").unwrap();
        let server = std::thread::spawn(move || receive(&listener, "secret").unwrap());
        assert!(hand_over(&instance, "/work/repo"));
        assert_eq!(server.join().unwrap().as_deref(), Some("/work/repo"));
    }

    #[test]
    fn a_wrong_token_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let instance = dir.path().join("instance");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        write_instance(&instance, port, "forged").unwrap();
        let server = std::thread::spawn(move || receive(&listener, "secret").unwrap());
        assert!(!hand_over(&instance, "/work/repo"));
        assert_eq!(server.join().unwrap(), None);
    }

    #[test]
    fn no_instance_means_no_hand_over() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!hand_over(&dir.path().join("instance"), "/work/repo"));
        // A stale file pointing at a closed port.
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let instance = dir.path().join("stale");
        write_instance(&instance, port, "secret").unwrap();
        assert!(!hand_over(&instance, "/work/repo"));
    }

    #[test]
    fn tokens_differ() {
        assert_ne!(new_token(), new_token());
        assert_eq!(new_token().len(), 32);
    }
}
