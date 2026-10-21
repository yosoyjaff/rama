//! Running `git`.
//!
//! Every read in this module is the git CLI, spawned and waited on. That is
//! the whole data layer: worktrees, submodules, sparse-checkout, LFS and
//! signatures behave like git here because they *are* git.
//!
//! These calls block. The caller is a window, so nothing calls them on the
//! foreground: [`crate::git::history`] and its neighbours are meant to run on
//! `cx.background_executor()`, with the result applied back on the UI thread.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Result, anyhow};

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

    let output = command
        .output()
        .map_err(|e| anyhow!("git spawn failed: {e}"))?;
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
    fn a_failing_command_carries_gits_own_message() {
        let repo = TestRepo::new();
        let error = git(repo.path(), &["rev-parse", "--verify", "nope"])
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("git:"), "{error:?}");
    }
}
