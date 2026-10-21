//! Finding the repository to open.

use std::path::{Path, PathBuf};

use super::command::git;

/// The root of the repository the process was started in, if it is inside one.
///
/// `--show-toplevel` is git's own answer, which is what makes this work from a
/// subdirectory, inside a linked worktree, and through a symlinked path — the
/// three cases a "walk up looking for `.git`" loop gets wrong.
pub fn repository_at_cwd() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    repository_at(&cwd)
}

/// The root of the repository containing `path`, if any.
pub fn repository_at(path: &Path) -> Option<String> {
    let root = git(path, &["rev-parse", "--show-toplevel"]).ok()?;
    (!root.is_empty()).then(|| PathBuf::from(root).to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;

    #[test]
    fn a_checkout_is_found_from_one_of_its_subdirectories() {
        let repo = TestRepo::new();
        repo.commit("first");
        let nested = repo.path().join("deep").join("nested");
        std::fs::create_dir_all(&nested).expect("create dirs");

        let found = repository_at(&nested).expect("the repository");
        // Compare canonically: a temporary directory is a symlink on macOS.
        assert_eq!(
            std::fs::canonicalize(found).unwrap(),
            std::fs::canonicalize(repo.path()).unwrap()
        );
    }

    #[test]
    fn a_folder_that_is_not_a_checkout_is_not_one() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert_eq!(repository_at(dir.path()), None);
    }
}
