//! Running `git`.
//!
//! Every read in this module is the git CLI, spawned and waited on. That is
//! the whole data layer: worktrees, submodules, sparse-checkout, LFS and
//! signatures behave like git here because they *are* git.
//!
//! These calls block. The caller is a window, so nothing calls them on the
//! foreground: [`crate::git::history`] and its neighbours are meant to run on
//! `cx.background_executor()`, with the result applied back on the UI thread.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

/// How long a git command may run before it is stopped, in milliseconds;
/// zero for as long as it takes. The app sets it from the settings: this
/// layer does not read them.
static TIMEOUT_MS: AtomicU64 = AtomicU64::new(0);

/// Stop every git command that runs longer than `limit` from now on; `None`
/// lets them run to the end.
pub fn set_timeout(limit: Option<Duration>) {
    let ms = limit.map_or(0, |limit| limit.as_millis().max(1) as u64);
    TIMEOUT_MS.store(ms, Ordering::Relaxed);
}

fn timeout() -> Option<Duration> {
    match TIMEOUT_MS.load(Ordering::Relaxed) {
        0 => None,
        ms => Some(Duration::from_millis(ms)),
    }
}

/// Run `command` (`git <args>`) to the end, or kill it once the timeout set
/// with [`set_timeout`] has passed.
pub(crate) fn output(command: &mut Command, args: &[&str]) -> Result<Output> {
    output_within(command, args, timeout())
}

/// The longest a wait between two looks at a running command gets: what a
/// timed command can lose on top of its own time.
const MAX_NAP: Duration = Duration::from_millis(50);

fn output_within(command: &mut Command, args: &[&str], limit: Option<Duration>) -> Result<Output> {
    let Some(limit) = limit else {
        return command
            .output()
            .map_err(|e| anyhow!("git spawn failed: {e}"));
    };
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| anyhow!("git spawn failed: {e}"))?;
    // Drained as it comes, or a chatty command fills the pipe and stalls.
    fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + limit;
    // Short at first, so quick commands lose nothing, then longer.
    let mut nap = Duration::from_millis(1);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let now = Instant::now();
        if now >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            // The readers end when whatever git started lets go of the
            // pipes; nothing waits for them.
            bail!(
                "git {} timed out after {}",
                args.first().unwrap_or(&"?"),
                describe(limit)
            );
        }
        std::thread::sleep(nap.min(deadline - now));
        nap = (nap * 2).min(MAX_NAP);
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// `30 s`, `5 min`.
fn describe(limit: Duration) -> String {
    match limit.as_secs() {
        secs if secs >= 60 && secs % 60 == 0 => format!("{} min", secs / 60),
        secs => format!("{secs} s"),
    }
}

/// Run `git <args>` inside `cwd` and return its trimmed stdout.
///
/// A non-zero exit is an error carrying git's own stderr: the message the user
/// needs is the one git wrote, not a paraphrase of it.
pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = spawn(cwd, args)?;
    Ok(String::from_utf8_lossy(&output).trim().to_string())
}

/// Raw stdout, undecoded.
///
/// The porcelain formats are NUL-separated and a path is bytes, not text: the
/// split has to happen before any lossy decoding, or a path with invalid UTF-8
/// takes its record's separator with it.
pub fn git_bytes(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    spawn(cwd, args)
}

/// Stdout with a ceiling on it.
///
/// A patch has no natural size limit — a vendored directory landing in one
/// commit is megabytes of it — and past a point no one reads it anyway. The
/// cap is what keeps one pathological commit from being held in memory in
/// full, and `truncated` is what lets the view say the tail is missing instead
/// of quietly showing half a diff.
pub struct Capture {
    pub stdout: Vec<u8>,
    pub truncated: bool,
}

pub fn git_capture(cwd: &Path, args: &[&str], max_bytes: usize) -> Result<Capture> {
    let mut stdout = spawn(cwd, args)?;
    let truncated = stdout.len() > max_bytes;
    if truncated {
        stdout.truncate(max_bytes);
    }
    Ok(Capture { stdout, truncated })
}

/// Run `git <args>` for its effect, discarding stdout.
pub fn git_ok(cwd: &Path, args: &[&str]) -> Result<()> {
    spawn(cwd, args).map(|_| ())
}

fn spawn(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command.args(args);
    command.current_dir(cwd);
    // No terminal is attached to a window: git must never wait for a password
    // prompt, an editor or a pager it has nowhere to draw.
    command.stdin(Stdio::null());
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_OPTIONAL_LOCKS", "0");
    // git is translated, and a parser that reads its prose reads it in the
    // user's language: on a non-English system `stash push` answers "No local
    // changes to save" in that language and an English substring check is
    // simply wrong. Pin the messages to C so what is parsed is what was
    // written against.
    command.env("LC_ALL", "C");
    no_console_window(&mut command);

    let output = output(&mut command, args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim();
        return Err(if message.is_empty() {
            anyhow!(
                "git {} failed ({})",
                args.first().unwrap_or(&"?"),
                output.status
            )
        } else {
            anyhow!("git: {message}")
        });
    }
    Ok(output.stdout)
}

/// Windows spawns a console for a child process of a GUI application, which
/// reads as a black window blinking on every read. `CREATE_NO_WINDOW` is what
/// suppresses it; everywhere else there is nothing to suppress.
#[cfg(windows)]
pub(crate) fn no_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(crate) fn no_console_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;

    #[test]
    fn stdout_comes_back_trimmed() {
        let repo = TestRepo::new();
        repo.commit("first");
        let sha = git(repo.path(), &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(sha.len(), 40, "{sha:?} is not a full sha");
        assert!(!sha.ends_with('\n'));
    }

    #[test]
    fn a_timed_command_runs_to_the_end_inside_its_limit() {
        let output = output_within(
            Command::new("git").arg("--version"),
            &["--version"],
            Some(Duration::from_secs(30)),
        )
        .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("git version"));
    }

    #[cfg(unix)]
    #[test]
    fn a_command_past_its_limit_is_stopped() {
        let started = Instant::now();
        let error = output_within(
            Command::new("sleep").arg("5"),
            &["fetch"],
            Some(Duration::from_millis(100)),
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, "git fetch timed out after 0 s");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "it waited it out"
        );
    }

    #[test]
    fn a_limit_reads_in_seconds_or_minutes() {
        assert_eq!(describe(Duration::from_secs(30)), "30 s");
        assert_eq!(describe(Duration::from_secs(300)), "5 min");
    }

    #[test]
    fn a_failing_command_carries_gits_own_message() {
        let repo = TestRepo::new();
        let error = git(repo.path(), &["rev-parse", "--verify", "nope"])
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("git:"), "{error:?}");
    }
}
