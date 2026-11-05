//! The `rama` terminal command on macOS: `rama .` opens a repository.
//!
//! The `.deb`, `.rpm` and `.msi` put `rama` on the PATH when they install.
//! A `.dmg` installs nothing — it copies `Rama.app` — so on macOS the first
//! launch does it: a link at [`LINK`] to the binary inside the bundle. It is
//! made quietly when the folder allows it, and otherwise behind macOS's own
//! administrator prompt. That first launch also covers anyone updating from a
//! version without it. Asked once; if it was cancelled or failed, Settings →
//! About offers it again ([`InstallCommandLine`]).

use std::path::{Path, PathBuf};

use gpui_kit::{App, AppContext as _, Global, SharedString};

use crate::settings::{self, SavePolicy};

/// Where the command goes: on macOS's default PATH, for every shell.
pub const LINK: &str = "/usr/local/bin/rama";

gpui_kit::actions!(
    command_line,
    [
        /// Install the `rama` terminal command. Settings → About.
        InstallCommandLine,
    ]
);

/// Where the command stands, for Settings → About.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandStatus {
    Ready,
    Missing,
    Installing,
    /// Rama runs from somewhere a link would not survive.
    Unavailable(SharedString),
    Failed(SharedString),
}

/// `None` where there is nothing to install: not macOS, or not a bundle (a
/// development build).
struct CommandLine(Option<CommandStatus>);

impl Global for CommandLine {}

/// The command's status, or `None` where Settings should not mention it.
pub fn status(cx: &App) -> Option<CommandStatus> {
    cx.try_global::<CommandLine>()
        .and_then(|state| state.0.clone())
}

fn set(status: CommandStatus, cx: &mut App) {
    cx.set_global(CommandLine(Some(status)));
    cx.refresh_windows();
}

/// Work out where the command stands and, on the first launch that finds it
/// missing, install it.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &InstallCommandLine, cx| install(cx));
    let initial = if cfg!(target_os = "macos") {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.canonicalize().ok())
            .and_then(|exe| initial_status(&exe, Path::new(LINK)))
    } else {
        None
    };
    let offer =
        initial == Some(CommandStatus::Missing) && !settings::current(cx).command_line_offered;
    cx.set_global(CommandLine(initial));
    if offer {
        settings::update(SavePolicy::Immediate, cx, |settings| {
            settings.command_line_offered = true;
        });
        install(cx);
    }
}

/// `None` outside an app bundle.
fn initial_status(exe: &Path, link: &Path) -> Option<CommandStatus> {
    let text = exe.to_string_lossy();
    if !text.contains(".app/Contents/MacOS/") {
        return None;
    }
    // Opened from the disk image, or from where Gatekeeper runs an app it
    // has not let out of quarantine: the path is gone once Rama closes.
    if text.starts_with("/Volumes/") || text.contains("/AppTranslocation/") {
        return Some(CommandStatus::Unavailable(
            "Move Rama to Applications first".into(),
        ));
    }
    Some(if points_at(link, exe) {
        CommandStatus::Ready
    } else {
        CommandStatus::Missing
    })
}

/// Whether `link` leads to `exe`.
fn points_at(link: &Path, exe: &Path) -> bool {
    match (link.canonicalize(), exe.canonicalize()) {
        (Ok(target), Ok(exe)) => target == exe,
        _ => false,
    }
}

/// Make the link, off the main thread: macOS's password prompt waits on
/// the person.
fn install(cx: &mut App) {
    if !matches!(
        status(cx),
        Some(CommandStatus::Missing | CommandStatus::Failed(_))
    ) {
        return;
    }
    let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok())
    else {
        return;
    };
    set(CommandStatus::Installing, cx);
    cx.spawn(async move |cx| {
        let link = PathBuf::from(LINK);
        let outcome = cx
            .background_spawn(async move { make_link(&exe, &link) })
            .await;
        cx.update(|cx| {
            set(
                match outcome {
                    Ok(()) => CommandStatus::Ready,
                    Err(Outcome::Cancelled) => CommandStatus::Missing,
                    Err(Outcome::Failed(message)) => CommandStatus::Failed(message.into()),
                },
                cx,
            )
        });
    })
    .detach();
}

#[derive(Debug, PartialEq)]
enum Outcome {
    /// The password prompt was dismissed.
    Cancelled,
    Failed(String),
}

/// Point `link` at `exe`: directly when the folder allows it, otherwise as
/// an administrator.
fn make_link(exe: &Path, link: &Path) -> Result<(), Outcome> {
    if link_directly(exe, link).is_ok() {
        return Ok(());
    }
    link_as_admin(exe, link)
}

#[cfg(unix)]
fn link_directly(exe: &Path, link: &Path) -> std::io::Result<()> {
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Whatever was there — an older link, a moved copy — is replaced.
    if link.symlink_metadata().is_ok() {
        std::fs::remove_file(link)?;
    }
    std::os::unix::fs::symlink(exe, link)
}

#[cfg(not(unix))]
fn link_directly(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// macOS's own administrator prompt, through `osascript`.
fn link_as_admin(exe: &Path, link: &Path) -> Result<(), Outcome> {
    if !cfg!(target_os = "macos") {
        return Err(Outcome::Failed("Only needed on macOS".into()));
    }
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(admin_script(exe, link))
        .output()
        .map_err(|error| Outcome::Failed(error.to_string()))?;
    if output.status.success() {
        return Ok(());
    }
    let error = String::from_utf8_lossy(&output.stderr);
    // AppleScript's "User canceled." is error -128.
    if error.contains("-128") {
        return Err(Outcome::Cancelled);
    }
    Err(Outcome::Failed(
        error
            .trim()
            .lines()
            .last()
            .unwrap_or("Could not install the command")
            .to_owned(),
    ))
}

/// The AppleScript that makes the link with administrator rights. Paths go
/// in as AppleScript strings and reach the shell through `quoted form of`,
/// so a space or a quote in them stays part of the path.
fn admin_script(exe: &Path, link: &Path) -> String {
    let text = |path: &Path| {
        format!(
            "\"{}\"",
            path.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        )
    };
    let dir = link.parent().unwrap_or(Path::new("/"));
    format!(
        "do shell script \"mkdir -p \" & quoted form of {} & \" && ln -sf \" & quoted form of {} \
         & \" \" & quoted form of {} with prompt \"Rama will install the rama command, so \
         rama . opens a repository from the terminal.\" with administrator privileges",
        text(dir),
        text(exe),
        text(link),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_bundle_in_a_lasting_place_is_offered() {
        let link = Path::new("/nonexistent/rama");
        assert_eq!(
            initial_status(Path::new("/work/target/debug/rama"), link),
            None
        );
        assert_eq!(
            initial_status(
                Path::new("/Applications/Rama.app/Contents/MacOS/rama"),
                link
            ),
            Some(CommandStatus::Missing)
        );
        assert!(matches!(
            initial_status(
                Path::new("/Volumes/Rama/Rama.app/Contents/MacOS/rama"),
                link
            ),
            Some(CommandStatus::Unavailable(_))
        ));
        assert!(matches!(
            initial_status(
                Path::new(
                    "/private/var/folders/x/AppTranslocation/y/d/Rama.app/Contents/MacOS/rama"
                ),
                link
            ),
            Some(CommandStatus::Unavailable(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn the_link_is_made_and_then_found() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("Rama.app/Contents/MacOS/rama");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        let link = dir.path().join("bin/rama");
        assert!(!points_at(&link, &exe));
        link_directly(&exe, &link).unwrap();
        assert!(points_at(&link, &exe));
        // A second install replaces the link rather than failing on it.
        link_directly(&exe, &link).unwrap();
        assert_eq!(initial_status(&exe, &link), Some(CommandStatus::Ready));
    }

    #[test]
    fn the_script_quotes_its_paths() {
        let script = admin_script(
            Path::new("/Applications/My \"Apps\"/Rama.app/Contents/MacOS/rama"),
            Path::new("/usr/local/bin/rama"),
        );
        assert!(script.contains(
            r#"quoted form of "/Applications/My \"Apps\"/Rama.app/Contents/MacOS/rama""#
        ));
        assert!(script.contains(r#"quoted form of "/usr/local/bin""#));
        assert!(script.ends_with("with administrator privileges"));
    }
}
