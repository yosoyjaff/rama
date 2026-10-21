//! Installing a release in place: download, verify, swap, relaunch.
//!
//! The archives this replaces are the ones `packaging/package.sh` builds, and
//! the installation it targets is the one that archive produces:
//!
//! * **Linux** — the `rama` binary wherever it runs from (`install.sh` puts it
//!   in `~/.local/bin`, which the user owns);
//! * **Windows** — the portable `rama.exe` from the `.zip`;
//! * **macOS** — the whole `Rama.app` bundle, never the binary inside it, so
//!   the bundle stays one consistent thing.
//!
//! Everything is staged in a folder *beside* what it replaces, so the final
//! step is a rename on one filesystem rather than a copy that can be cut in
//! half. Nothing is touched until the archive's size and SHA-256 match the
//! manifest. A running executable is renamed out of the way rather than
//! overwritten — Windows allows that, and on Unix the running process keeps
//! its open file — and the old copy is removed on the next start.
//!
//! The manifest is fetched over HTTPS from the project's own GitHub releases,
//! and the checksum proves the bytes are the ones that release published. It
//! does not prove who published them: that needs a signature, which the
//! manifest has a field for and this does not check yet.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};

use super::manifest::ReleaseArtifact;

/// Whether this build can replace itself. A development build runs out of
/// `target/`, which is not an installation; it is offered the release page
/// instead.
pub fn supported() -> bool {
    !cfg!(debug_assertions)
}

/// What a finished install hands back: the thing to launch in its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub relaunch: PathBuf,
}

/// Download `artifact`, check it, and put it in place of the running build.
/// Blocking: run it on a background thread.
pub fn install(artifact: &ReleaseArtifact, version: &str) -> Result<Installed> {
    if !supported() {
        bail!("a development build does not update itself");
    }
    let target = InstallTarget::current()?;
    let staging = target.parent()?.join(format!(".rama-update-{version}"));
    // A previous attempt that stopped half way starts over.
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).with_context(|| {
        format!(
            "cannot write next to the installation ({})",
            target
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        )
    })?;
    let result = (|| {
        let archive = download(artifact, &staging)?;
        let unpacked = staging.join("unpacked");
        fs::create_dir_all(&unpacked)?;
        unpack(&archive, &unpacked)?;
        target.replace_with(&unpacked)
    })();
    // The archive and whatever was not moved into place are not needed
    // either way; a swapped-out old copy lives outside this folder.
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Start the installed build, detached. The caller quits right after.
pub fn relaunch(installed: &Installed) -> Result<()> {
    if cfg!(target_os = "macos")
        && installed
            .relaunch
            .extension()
            .is_some_and(|ext| ext == "app")
    {
        // `open -n` starts the bundle as the system would, with its own
        // Dock icon, even while this process is still alive.
        Command::new("open")
            .arg("-n")
            .arg(&installed.relaunch)
            .spawn()
            .context("could not start the new version")?;
    } else {
        Command::new(&installed.relaunch)
            .spawn()
            .context("could not start the new version")?;
    }
    Ok(())
}

/// Remove what the last update left behind: the swapped-out copy, which the
/// old process held open until it exited.
pub fn remove_leftovers() {
    if let Ok(target) = InstallTarget::current() {
        let _ = match &target {
            InstallTarget::Binary(path) => fs::remove_file(old_path(path)),
            InstallTarget::Bundle(path) => fs::remove_dir_all(old_path(path)),
        };
    }
}

/// What gets replaced.
#[derive(Debug)]
enum InstallTarget {
    /// A lone executable (Linux, Windows).
    Binary(PathBuf),
    /// A macOS `.app` bundle.
    Bundle(PathBuf),
}

impl InstallTarget {
    fn current() -> Result<Self> {
        let exe = std::env::current_exe()
            .and_then(|path| path.canonicalize())
            .context("cannot find the running executable")?;
        Ok(match bundle_of(&exe) {
            Some(bundle) => Self::Bundle(bundle),
            None => Self::Binary(exe),
        })
    }

    fn path(&self) -> &Path {
        match self {
            Self::Binary(path) | Self::Bundle(path) => path,
        }
    }

    fn parent(&self) -> Result<PathBuf> {
        self.path()
            .parent()
            .map(Path::to_path_buf)
            .context("the installation has no parent folder")
    }

    /// Swap the unpacked release in. On failure the running build is put
    /// back where it was.
    fn replace_with(&self, unpacked: &Path) -> Result<Installed> {
        let current = self.path();
        let name = current
            .file_name()
            .context("the installation has no file name")?;
        let fresh = unpacked.join(name);
        if !fresh.exists() {
            bail!("the release archive has no {}", Path::new(name).display());
        }
        let old = old_path(current);
        let _ = match self {
            Self::Binary(_) => fs::remove_file(&old),
            Self::Bundle(_) => fs::remove_dir_all(&old),
        };
        fs::rename(current, &old)
            .with_context(|| format!("cannot move {} aside", current.display()))?;
        if let Err(error) = fs::rename(&fresh, current) {
            let _ = fs::rename(&old, current);
            return Err(error)
                .with_context(|| format!("cannot install into {}", current.display()));
        }
        #[cfg(unix)]
        if let Self::Binary(path) = self {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
        }
        Ok(Installed {
            relaunch: current.to_path_buf(),
        })
    }
}

/// The `.app` an executable runs from, when it does:
/// `…/Rama.app/Contents/MacOS/rama` → `…/Rama.app`.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

/// Where the replaced copy waits until the next start removes it.
fn old_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    path.with_file_name(name)
}

/// Fetch the archive into `dir` and check it against the manifest.
fn download(artifact: &ReleaseArtifact, dir: &Path) -> Result<PathBuf> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .build()
        .new_agent();
    let mut response = agent
        .get(&artifact.url)
        .header("User-Agent", crate::APP_NAME)
        .call()
        .context("the download did not start")?;
    // A little over the announced size, so a wrong manifest cannot make this
    // read without bound.
    let bytes = response
        .body_mut()
        .with_config()
        .limit(artifact.size.saturating_add(1 << 20))
        .read_to_vec()
        .context("the download did not finish")?;
    verify(&bytes, artifact)?;
    let file = artifact
        .url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("update.tar.gz");
    let path = dir.join(file);
    fs::write(&path, &bytes).context("cannot save the download")?;
    Ok(path)
}

/// The bytes are the ones the release published: same size, same SHA-256.
fn verify(bytes: &[u8], artifact: &ReleaseArtifact) -> Result<()> {
    if bytes.len() as u64 != artifact.size {
        bail!(
            "the download is {} bytes, the release says {}",
            bytes.len(),
            artifact.size
        );
    }
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !digest.eq_ignore_ascii_case(artifact.sha256.trim()) {
        bail!("the download does not match the release's checksum");
    }
    Ok(())
}

/// Unpack with the system's `tar`: present on macOS and Linux, and on Windows
/// 10 and later, where it also reads `.zip`.
fn unpack(archive: &Path, into: &Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .status()
        .context("cannot run tar to unpack the update")?;
    if !status.success() {
        bail!("the update archive did not unpack ({status})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(bytes: &[u8]) -> ReleaseArtifact {
        let sha256 = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        ReleaseArtifact {
            target: "t".into(),
            url: "https://example.com/rama-1.0.0-t.tar.gz".into(),
            size: bytes.len() as u64,
            sha256,
            signature: None,
        }
    }

    #[test]
    fn only_the_published_bytes_pass() {
        let release = artifact(b"the real build");
        assert!(verify(b"the real build", &release).is_ok());
        assert!(verify(b"the real build!", &release).is_err(), "size");
        assert!(verify(b"the fake build", &release).is_err(), "checksum");
    }

    #[test]
    fn a_bundle_is_found_from_its_executable() {
        assert_eq!(
            bundle_of(Path::new("/Applications/Rama.app/Contents/MacOS/rama")),
            Some(PathBuf::from("/Applications/Rama.app"))
        );
        assert_eq!(bundle_of(Path::new("/home/u/.local/bin/rama")), None);
    }

    #[test]
    fn a_swap_puts_the_new_build_in_place_and_keeps_the_old_aside() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("rama");
        fs::write(&installed, "old").unwrap();
        let unpacked = dir.path().join("unpacked");
        fs::create_dir_all(&unpacked).unwrap();
        fs::write(unpacked.join("rama"), "new").unwrap();

        let result = InstallTarget::Binary(installed.clone())
            .replace_with(&unpacked)
            .unwrap();
        assert_eq!(result.relaunch, installed);
        assert_eq!(fs::read_to_string(&installed).unwrap(), "new");
        assert_eq!(fs::read_to_string(old_path(&installed)).unwrap(), "old");
    }

    #[test]
    fn a_release_without_the_binary_leaves_the_installation_alone() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("rama");
        fs::write(&installed, "old").unwrap();
        let unpacked = dir.path().join("unpacked");
        fs::create_dir_all(&unpacked).unwrap();

        assert!(
            InstallTarget::Binary(installed.clone())
                .replace_with(&unpacked)
                .is_err()
        );
        assert_eq!(fs::read_to_string(&installed).unwrap(), "old");
    }
}
