//! Getting a repository in the first place: cloning one, or starting one.
//!
//! Both answer with the path of the checkout they produced, because that is
//! what the caller opens next.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::command::git;

/// Clone `url` into `parent`, returning the checkout git created.
///
/// The name comes from the URL, the way `git clone` picks it, and the path is
/// checked before the network is touched: an hour of cloning that ends in
/// "directory exists" is an hour wasted.
pub fn clone(url: &str, parent: &Path) -> Result<PathBuf> {
    let url = url.trim();
    if url.is_empty() {
        bail!("no repository url");
    }
    if !parent.is_dir() {
        bail!("{} is not a folder", parent.display());
    }
    let name = clone_directory_name(url);
    if name.is_empty() {
        bail!("`{url}` does not name a repository");
    }
    let target = parent.join(&name);
    if target.exists() {
        bail!("{} already exists", target.display());
    }
    // Run from the parent with an explicit target: `git clone <url> <name>`
    // resolves a relative name against the working directory, and this way
    // there is no doubt which one that is.
    git(parent, &["clone", "--", url, &name])?;
    Ok(target)
}

/// Start a repository at `path`, creating the folder if it is not there.
///
/// An existing checkout is left alone and returned: `git init` on one is a
/// no-op that reports success, and the user asking to create what they already
/// have means they want it open.
pub fn init(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("no folder given");
    }
    std::fs::create_dir_all(path)?;
    git(path, &["init"])?;
    Ok(path.to_path_buf())
}

/// The folder `git clone` would create for this URL: the last path segment,
/// without `.git`.
fn clone_directory_name(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    // `git@host:owner/repo.git` — the segment after the colon, then the last
    // path element of it.
    let tail = trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git");
    tail.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;

    #[test]
    fn the_clone_directory_is_the_one_git_would_pick() {
        assert_eq!(clone_directory_name("https://host/owner/repo.git"), "repo");
        assert_eq!(clone_directory_name("https://host/owner/repo"), "repo");
        assert_eq!(clone_directory_name("https://host/owner/repo/"), "repo");
        assert_eq!(clone_directory_name("git@host:owner/repo.git"), "repo");
        assert_eq!(clone_directory_name("/srv/git/repo.git"), "repo");
        assert_eq!(clone_directory_name(""), "");
    }

    #[test]
    fn cloning_a_local_repository_produces_a_checkout_of_it() {
        let source = TestRepo::new();
        let sha = source.commit("first");
        let into = tempfile::tempdir().expect("temporary directory");

        let clone_url = source.path().to_string_lossy().into_owned();
        let target = clone(&clone_url, into.path()).expect("clone");
        assert!(target.join(".git").exists(), "{target:?} is not a checkout");
        assert_eq!(
            crate::git::history::history(&target, 0, 10)
                .unwrap()
                .commits[0]
                .sha,
            sha
        );
    }

    #[test]
    fn cloning_onto_an_existing_folder_is_refused_before_the_network() {
        let source = TestRepo::new();
        source.commit("first");
        let into = tempfile::tempdir().expect("temporary directory");
        let clone_url = source.path().to_string_lossy().into_owned();
        let name = clone_directory_name(&clone_url);
        std::fs::create_dir_all(into.path().join(&name)).expect("create dir");

        let error = clone(&clone_url, into.path()).unwrap_err().to_string();
        assert!(error.contains("already exists"), "{error:?}");
    }

    #[test]
    fn creating_a_repository_makes_the_folder_and_the_checkout() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("brand").join("new");
        let created = init(&path).expect("init");
        assert_eq!(created, path);
        assert!(path.join(".git").exists());
        // Opening it finds the checkout that was just made.
        assert!(crate::git::open::repository_at(&path).is_some());
    }

    #[test]
    fn creating_over_an_existing_checkout_leaves_it_alone() {
        let repo = TestRepo::new();
        let sha = repo.commit("first");
        init(repo.path()).expect("init on an existing checkout");
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), sha);
    }
}
