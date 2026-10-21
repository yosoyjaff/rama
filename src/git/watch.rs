//! Noticing changes made to a repository from outside: another client, an
//! editor, a terminal. Every tool writes the same files in the git dir, so
//! watching it is how a commit, merge or checkout made elsewhere shows up
//! here without a manual refresh.
//!
//! Only HEAD, the index, the other top-level state files and `refs/` are
//! watched: they tell the whole story, while `objects/` churns on every
//! fetch. The watcher only raises a flag; the waiting side lets a burst
//! settle (a checkout rewrites dozens of files) before it reloads once.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::command::git;

/// How long the waiting side lets a burst of writes go on before it reloads.
pub const SETTLE: Duration = Duration::from_millis(200);

/// Whether an event says anything happened. Reads (git opens HEAD and refs
/// on every command, ours included) would otherwise wake a reload that
/// reads them again.
fn is_change(event: &Event) -> bool {
    !matches!(event.kind, EventKind::Access(_))
        && event.paths.iter().any(|path| !says_nothing(path))
}

/// Files written by operations that say nothing new about refs, the index or
/// HEAD: `FETCH_HEAD` on every fetch (one that brings something also writes
/// under `refs/remotes/`), the message of a commit being written, and the
/// `.lock` files that precede each real write.
fn says_nothing(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    name.ends_with(".lock") || matches!(name, "COMMIT_EDITMSG" | "FETCH_HEAD")
}

/// The git dirs of the checkout at `root`: its own and, for a linked
/// worktree, the common one where the refs live. Empty if `root` is not a
/// checkout.
fn git_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(out) = git(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
    ) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    for line in out.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let dir = PathBuf::from(line);
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// What to register for each git dir: the dir itself flat (HEAD, index,
/// packed-refs and MERGE_HEAD sit at its top), and `refs/` deep, since
/// branch names nest.
fn targets(dirs: &[PathBuf]) -> Vec<(PathBuf, RecursiveMode)> {
    dirs.iter()
        .flat_map(|dir| {
            let refs = dir.join("refs");
            let deep = refs.is_dir().then_some((refs, RecursiveMode::Recursive));
            std::iter::once((dir.clone(), RecursiveMode::NonRecursive)).chain(deep)
        })
        .collect()
}

/// A flag the watcher raises and the UI side waits on.
#[derive(Default)]
struct Flag {
    raised: bool,
    waiting: Option<Waker>,
}

/// The UI side's handle: each [`Self::next`] resolves after the next change.
#[derive(Clone)]
pub struct Changes(Arc<Mutex<Flag>>);

impl Changes {
    fn raise(&self) {
        let mut flag = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        flag.raised = true;
        if let Some(waker) = flag.waiting.take() {
            waker.wake();
        }
    }

    /// Forget what changed so far: the burst just waited out is being
    /// handled.
    pub fn clear(&self) {
        let mut flag = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        flag.raised = false;
    }

    /// Resolves once something changed since the last time it resolved.
    pub async fn next(&self) {
        std::future::poll_fn(|cx| {
            let mut flag = self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if std::mem::take(&mut flag.raised) {
                return Poll::Ready(());
            }
            flag.waiting = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }
}

/// A live watch over one checkout. Dropping it stops watching.
pub struct RepoWatch {
    changes: Changes,
    _watcher: Option<RecommendedWatcher>,
}

impl RepoWatch {
    /// Start watching the checkout at `root`. A checkout that cannot be
    /// watched (no git dir, no watcher on this system) never reports a change.
    pub fn spawn(root: &Path) -> Self {
        let changes = Changes(Arc::default());
        let raise = changes.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            if event.is_ok_and(|event| is_change(&event)) {
                raise.raise();
            }
        });
        let watcher = watcher.ok().map(|mut watcher| {
            for (path, mode) in targets(&git_dirs(root)) {
                // A dir gone since it was listed is no reason to drop the rest.
                if let Err(error) = watcher.watch(&path, mode) {
                    eprintln!("git watch {}: {error}", path.display());
                }
            }
            watcher
        });
        Self {
            changes,
            _watcher: watcher,
        }
    }

    pub fn changes(&self) -> Changes {
        self.changes.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;
    use std::time::Instant;

    #[test]
    fn writes_that_change_nothing_are_ignored() {
        let dir = Path::new("/repo/.git");
        for quiet in [
            "COMMIT_EDITMSG",
            "FETCH_HEAD",
            "index.lock",
            "refs/heads/x.lock",
        ] {
            assert!(says_nothing(&dir.join(quiet)), "{quiet}");
        }
        for loud in [
            "HEAD",
            "index",
            "MERGE_HEAD",
            "packed-refs",
            "refs/heads/main",
        ] {
            assert!(!says_nothing(&dir.join(loud)), "{loud}");
        }
    }

    #[test]
    fn reading_the_git_dir_is_not_a_change() {
        use notify::event::{AccessKind, ModifyKind};
        let head = PathBuf::from("/repo/.git/HEAD");
        let read = Event::new(EventKind::Access(AccessKind::Any)).add_path(head.clone());
        let write = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(head);
        assert!(!is_change(&read));
        assert!(is_change(&write));
    }

    #[test]
    fn a_linked_worktree_also_watches_the_common_dir() {
        let repo = TestRepo::new();
        repo.commit("first");
        let plain = git_dirs(repo.path());
        assert_eq!(plain.len(), 1);
        assert_eq!(
            targets(&plain),
            [
                (plain[0].clone(), RecursiveMode::NonRecursive),
                (plain[0].join("refs"), RecursiveMode::Recursive),
            ]
        );

        let outside = tempfile::tempdir().expect("temporary directory");
        let linked = outside.path().join("linked");
        repo.git(&[
            "worktree",
            "add",
            "-q",
            linked.to_str().unwrap(),
            "-b",
            "side",
        ]);
        let dirs = git_dirs(&linked);
        assert_eq!(dirs.len(), 2, "{dirs:?}");
        assert_eq!(dirs[1], plain[0], "the common dir is the main one");
    }

    #[test]
    fn a_folder_that_is_no_checkout_watches_nothing() {
        let empty = tempfile::tempdir().expect("temporary directory");
        assert!(git_dirs(empty.path()).is_empty());
    }

    /// Against a real repository: a commit made by another git process
    /// reaches the handle the pane awaits.
    #[test]
    fn a_commit_made_elsewhere_is_noticed() {
        let repo = TestRepo::new();
        repo.commit("first");
        let watch = RepoWatch::spawn(repo.path());
        let changes = watch.changes();
        // Registration is quick but not instant.
        std::thread::sleep(Duration::from_millis(200));
        repo.commit("second");

        let deadline = Instant::now() + Duration::from_secs(5);
        let waker = Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let mut next = Box::pin(changes.next());
        while next.as_mut().poll(&mut cx).is_pending() {
            assert!(Instant::now() < deadline, "the commit was never noticed");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
