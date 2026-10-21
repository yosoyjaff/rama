//! The repositories this machine has opened, newest first.
//!
//! Kept beside the settings but in a file of its own: it is per-machine state
//! — a path from another machine resolves to nothing here — and it is written
//! on every open, which is not a rhythm the preferences file should follow.

use std::path::{Path, PathBuf};

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "recent-repositories.json";

/// How many the list keeps. A "recent" list longer than a screen is an
/// archive, and the welcome screen is not one.
pub const MAX_RECENT: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    pub path: String,
    /// The checkout's folder name, which is what the row reads as.
    pub name: String,
    /// Unix seconds. Only the order is used, but the stamp is what makes the
    /// order survive a hand-edited file.
    pub opened_at: i64,
}

/// The list, in memory, owned by the process that writes it.
#[derive(Default)]
pub struct Recents {
    entries: Vec<RecentEntry>,
    path: PathBuf,
}

impl Global for Recents {}

pub fn init(cx: &mut App) {
    let path = crate::settings::settings_path().with_file_name(FILE_NAME);
    let entries = read(&path);
    cx.set_global(Recents { entries, path });
}

/// The recent repositories, newest first.
pub fn all(cx: &App) -> Vec<RecentEntry> {
    cx.try_global::<Recents>()
        .map(|recents| recents.entries.clone())
        .unwrap_or_default()
}

/// Record a repository as opened: it moves to the front, and a repeat opening
/// moves rather than duplicates it.
pub fn add(path: &str, cx: &mut App) {
    if !cx.has_global::<Recents>() {
        return;
    }
    let entry = RecentEntry {
        name: repository_name(path),
        path: path.to_string(),
        opened_at: now_seconds(),
    };
    let recents = cx.global_mut::<Recents>();
    recents
        .entries
        .retain(|existing| existing.path != entry.path);
    recents.entries.insert(0, entry);
    recents.entries.truncate(MAX_RECENT);
    flush(cx);
}

pub fn remove(path: &str, cx: &mut App) {
    if !cx.has_global::<Recents>() {
        return;
    }
    let recents = cx.global_mut::<Recents>();
    recents.entries.retain(|entry| entry.path != path);
    flush(cx);
}

fn flush(cx: &mut App) {
    let Some(recents) = cx.try_global::<Recents>() else {
        return;
    };
    let (entries, path) = (recents.entries.clone(), recents.path.clone());
    cx.background_executor()
        .spawn(async move {
            let _ = write(&path, &entries);
        })
        .detach();
}

fn read(path: &Path) -> Vec<RecentEntry> {
    // An unreadable list is an empty list: a stray comma in it must not be
    // the reason the window comes up without a welcome screen.
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Vec<RecentEntry>>(&contents).ok())
        .map(|mut entries| {
            entries.truncate(MAX_RECENT);
            entries
        })
        .unwrap_or_default()
}

fn write(path: &Path, entries: &[RecentEntry]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(entries)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

/// The checkout's own folder name.
pub fn repository_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_string()
}

/// The repositories GitKraken had open in its tabs, read from GitKraken's
/// own state on disk — the same state GitKraken restores on its own launch.
/// Each entry is `(repo path, was the tab on screen)`.
pub fn gitkraken_repos() -> Vec<(String, bool)> {
    for base in gitkraken_state_dirs() {
        let repos = gitkraken_repos_under(base);
        if !repos.is_empty() {
            return repos;
        }
    }
    Vec::new()
}

/// Where GitKraken keeps its state, per platform: `~/.gitkraken` on Linux,
/// `~/Library/Application Support/GitKraken` on macOS, `%APPDATA%\GitKraken`
/// on Windows.
fn gitkraken_state_dirs() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        out.push(home.join(".gitkraken"));
        out.push(home.join("Library/Application Support/GitKraken"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        out.push(std::path::PathBuf::from(appdata).join("GitKraken"));
    }
    out
}

fn gitkraken_repos_under(base: std::path::PathBuf) -> Vec<(String, bool)> {
    let profiles = base.join("profiles");
    let Ok(entries) = std::fs::read_dir(&profiles) else {
        return Vec::new();
    };
    let mut repos: Vec<(String, bool)> = Vec::new();
    for profile_dir in entries.flatten() {
        let profile_file = profile_dir.path().join("profile");
        let Ok(text) = std::fs::read_to_string(&profile_file) else {
            continue;
        };
        let Ok(profile) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(tabs) = profile
            .get("tabInfo")
            .and_then(|info| info.get("tabs"))
            .and_then(|tabs| tabs.as_array())
        else {
            continue;
        };
        let selected = profile
            .get("tabInfo")
            .and_then(|info| info.get("selectedTabId"))
            .and_then(|id| id.as_str())
            .map(str::to_string);
        for tab in tabs {
            if tab.get("type").and_then(|kind| kind.as_str()) != Some("REPO") {
                continue;
            }
            let Some(raw_path) = tab.get("repoPath").and_then(|path| path.as_str()) else {
                continue;
            };
            let path = raw_path.trim_end_matches(['/', '\\']).to_string();
            if path.is_empty() {
                continue;
            }
            let was_active = tab
                .get("id")
                .and_then(|id| id.as_str())
                .is_some_and(|id| selected.as_deref() == Some(id));
            match repos.iter_mut().find(|(open, _)| *open == path) {
                // The same checkout in two profiles: the tab on screen in
                // either one makes it the active one.
                Some((_, active)) => *active = *active || was_active,
                None => repos.push((path, was_active)),
            }
        }
    }
    repos
}

/// Where a checkout lives, as a row shows it under its name: the folder
/// that holds it, with the home folder written `~`.
///
/// Names collide in practice — a `dev/api` and a `prod/api` — so the row has
/// to show the folders that tell them apart. A deep path keeps its start and
/// its last two folders and elides the middle, rather than letting the row's
/// truncation cut off the end that differs.
pub fn display_path(path: &str) -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    display_path_from(path, &home)
}

fn display_path_from(path: &str, home: &str) -> String {
    let path = path.trim_end_matches(['/', '\\']);
    let parent = match path.rfind(['/', '\\']) {
        Some(0) => "/",
        Some(index) => &path[..index],
        None => return String::new(),
    };
    let home = home.trim_end_matches(['/', '\\']);
    let (root, rest) = if !home.is_empty()
        && parent.starts_with(home)
        && matches!(parent[home.len()..].chars().next(), None | Some('/' | '\\'))
    {
        ("~", &parent[home.len()..])
    } else {
        ("", parent)
    };
    let separator = if parent.contains('\\') { '\\' } else { '/' };
    let parts: Vec<&str> = rest.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let leading = if root.is_empty() && rest.starts_with(['/', '\\']) {
        separator.to_string()
    } else {
        String::new()
    };
    let joined = if parts.len() > 3 {
        format!(
            "{}{sep}…{sep}{}{sep}{}",
            parts[0],
            parts[parts.len() - 2],
            parts[parts.len() - 1],
            sep = separator
        )
    } else {
        parts.join(&separator.to_string())
    };
    match (root, joined.is_empty()) {
        ("~", true) => "~".into(),
        ("~", false) => format!("~{separator}{joined}"),
        (_, true) => parent.to_string(),
        _ => format!("{leading}{joined}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, opened_at: i64) -> RecentEntry {
        RecentEntry {
            name: repository_name(path),
            path: path.to_string(),
            opened_at,
        }
    }

    #[test]
    fn a_repository_is_named_by_its_folder() {
        assert_eq!(repository_name("/home/user/projects/rama"), "rama");
        assert_eq!(repository_name("/home/user/projects/rama/"), "rama");
        assert_eq!(repository_name("C:\\code\\rama"), "rama");
    }

    #[test]
    fn the_location_shows_the_folders_that_tell_two_checkouts_apart() {
        let home = "/home/user";
        assert_eq!(display_path_from("/home/user/dev/api", home), "~/dev");
        assert_eq!(display_path_from("/home/user/api", home), "~");
        assert_eq!(
            display_path_from("/home/username/api", home),
            "/home/username"
        );
        assert_eq!(display_path_from("/srv/api", home), "/srv");
        assert_eq!(display_path_from("/api", home), "/");
        assert_eq!(
            display_path_from("/home/user/work/clients/acme/prod/api", home),
            "~/work/…/acme/prod"
        );
        assert_eq!(
            display_path_from("C:\\code\\rama", "C:\\Users\\me"),
            "C:\\code"
        );
    }

    #[test]
    fn the_list_round_trips_and_is_capped() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join(FILE_NAME);
        let entries: Vec<RecentEntry> = (0..MAX_RECENT + 5)
            .map(|index| entry(&format!("/repos/{index}"), index as i64))
            .collect();

        write(&path, &entries).expect("write");
        let read_back = read(&path);
        assert_eq!(read_back.len(), MAX_RECENT);
        assert_eq!(read_back[0].path, "/repos/0");
    }

    #[test]
    fn an_unreadable_list_reads_as_empty() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join(FILE_NAME);
        assert!(read(&path).is_empty(), "a missing file");
        std::fs::write(&path, "{ not json").expect("write");
        assert!(read(&path).is_empty(), "a corrupt file");
    }
}
