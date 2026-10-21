//! Where the settings file lives, and how it is written.
//!
//! The write is atomic: the new contents go to a temporary file beside the
//! target and are then renamed over it. A crash, a full disk or two windows
//! writing at once can lose the *new* settings, but never leave a truncated
//! file that the next launch cannot read.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::Settings;

const FILE_NAME: &str = "settings.json";

/// The settings file for this user, on this platform.
///
/// The directory is the platform's own configuration home — no crate, because
/// the rule per platform is three lines and one environment variable.
pub fn settings_path() -> PathBuf {
    config_dir().join(FILE_NAME)
}

/// The folder author photographs are cached in, for this user.
pub fn avatars_dir() -> PathBuf {
    config_dir().join("avatars")
}

fn config_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "windows") {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join(crate::APP_NAME);
        }
    } else if cfg!(target_os = "macos") {
        if let Some(home) = home.clone() {
            return home
                .join("Library")
                .join("Application Support")
                .join(crate::APP_NAME);
        }
    } else if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config).join(crate::APP_NAME.to_lowercase());
    } else if let Some(home) = home.clone() {
        return home.join(".config").join(crate::APP_NAME.to_lowercase());
    }
    // Nothing to anchor to: keep the file beside the process rather than
    // losing the user's preferences entirely.
    PathBuf::from(".")
}

/// Read the settings, or `None` when there is no readable file yet.
///
/// A file that does not parse is treated as absent rather than as a failure:
/// the alternative is an application that will not start because of a stray
/// comma in a preferences file.
pub fn read(path: &Path) -> Option<Settings> {
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Write the settings atomically.
pub fn write(path: &Path, settings: &Settings) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(settings)?;
    let temporary = path.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(contents.as_bytes())?;
        // The rename below only publishes what actually reached the disk.
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_reads_back_the_same() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("deeper").join(FILE_NAME);
        let mut settings = Settings::default();
        settings.history.badges = false;
        settings.history.columns.author.width = 130.0;
        settings.tabs.open = vec!["/code/shop".into()];
        write(&path, &settings).expect("write");
        assert_eq!(read(&path), Some(settings));
    }

    #[test]
    fn a_missing_or_broken_file_reads_as_nothing() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join(FILE_NAME);
        assert_eq!(read(&path), None);
        std::fs::write(&path, "{ half").expect("write");
        assert_eq!(read(&path), None);
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let dir = tempfile::tempdir().expect("temporary directory");
        write(&dir.path().join(FILE_NAME), &Settings::default()).expect("write");
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [FILE_NAME]);
    }
}
