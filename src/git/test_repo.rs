//! A real repository, for the tests of this module.
//!
//! What breaks a git layer is git behaving differently from the assumption
//! written into it, and only a real `git init` says so. Every test here runs
//! against a throwaway repo in a temporary directory.

#![cfg(test)]

use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::TempDir;

pub struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    /// An initialized repository with a deterministic identity and no
    /// dependency on the developer's own git configuration.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        let repo = Self { dir };
        repo.git(&["init", "--initial-branch=main"]);
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Wrap a directory the test prepared itself, such as a fresh clone,
    /// with the same deterministic identity as [`Self::new`].
    pub fn from(dir: TempDir) -> Self {
        let repo = Self { dir };
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    /// Run a git command that is expected to succeed, returning its stdout.
    pub fn git(&self, args: &[&str]) -> String {
        self.git_env(args, &[])
    }

    /// Run a git command that may fail — a conflicted merge exits 1 by
    /// design — answering whether it succeeded.
    pub fn git_allow_failure(&self, args: &[&str]) -> bool {
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(self.path())
            .stdin(Stdio::null())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null");
        command.status().expect("git runs").success()
    }

    /// [`Self::git`] with extra environment — how a test pins a commit's date,
    /// which is the only way to make an ordering assertion deterministic when
    /// two commits would otherwise land in the same second.
    pub fn git_env(&self, args: &[&str], envs: &[(&str, &str)]) -> String {
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(self.path())
            .stdin(Stdio::null())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null");
        for (key, value) in envs {
            command.env(key, value);
        }
        let output = command.output().expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Write a file and commit it, returning the new commit's sha.
    pub fn commit(&self, message: &str) -> String {
        let file = self.path().join("log.txt");
        let previous = std::fs::read_to_string(&file).unwrap_or_default();
        std::fs::write(&file, format!("{previous}{message}\n")).expect("write file");
        self.git(&["add", "log.txt"]);
        self.git(&["commit", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Write a file without committing it, for the working-tree reads.
    pub fn write(&self, path: &str, contents: &str) {
        let target = self.path().join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(target, contents).expect("write file");
    }
}
