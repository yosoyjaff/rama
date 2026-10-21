//! The update decision, kept pure so it can be tested without a network.

use semver::Version;

use super::manifest::{ReleaseArtifact, ReleaseManifest};

/// The target triple this binary was built for.
///
/// Composed from the `target_*` cfgs rather than read at runtime, so it is the
/// triple CI used for *this* artifact even when the host could run several.
pub const fn current_target() -> &'static str {
    // Extend this list as the release matrix grows; an unknown target simply
    // never matches an artifact, so the app reports "up to date" instead of
    // offering a build it cannot run.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "aarch64-pc-windows-msvc"
    }
}

/// The artifact this build should install, or `None` when there is nothing to
/// do.
///
/// `None` covers every "nothing to do" case on purpose — same version, older
/// version, a manifest whose version does not parse, and a release that has no
/// build for this target. The caller cannot then mistake one for an update.
pub fn available_update<'a>(
    current: &Version,
    manifest: &'a ReleaseManifest,
    target: &str,
) -> Option<&'a ReleaseArtifact> {
    let released = Version::parse(&manifest.version).ok()?;
    if released <= *current {
        return None;
    }
    manifest.artifact_for(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str, targets: &[&str]) -> ReleaseManifest {
        let mut manifest = ReleaseManifest::new(version);
        for target in targets {
            manifest = manifest.with_artifact(ReleaseArtifact {
                target: (*target).to_owned(),
                url: format!("https://example.test/{version}/{target}.tar.gz"),
                size: 1,
                sha256: "0".repeat(64),
                signature: None,
            });
        }
        manifest
    }

    const HERE: &str = "x86_64-unknown-linux-gnu";

    #[test]
    fn a_newer_release_for_this_target_is_an_update() {
        let current = Version::parse("0.1.0").unwrap();
        let release = manifest("0.2.0", &[HERE]);
        let found = available_update(&current, &release, HERE);
        assert_eq!(found.map(|artifact| artifact.target.as_str()), Some(HERE));
    }

    #[test]
    fn the_same_or_an_older_release_is_not() {
        let current = Version::parse("1.0.0").unwrap();
        assert!(available_update(&current, &manifest("1.0.0", &[HERE]), HERE).is_none());
        assert!(available_update(&current, &manifest("0.9.9", &[HERE]), HERE).is_none());
    }

    #[test]
    fn a_prerelease_does_not_outrank_the_release_it_precedes() {
        // Semver order, not string order: 1.0.0-rc.1 < 1.0.0.
        let current = Version::parse("1.0.0").unwrap();
        assert!(available_update(&current, &manifest("1.0.0-rc.1", &[HERE]), HERE).is_none());

        let current = Version::parse("1.0.0-rc.1").unwrap();
        assert!(available_update(&current, &manifest("1.0.0", &[HERE]), HERE).is_some());
    }

    #[test]
    fn a_release_without_a_build_for_this_target_is_not_an_update() {
        // Offering it would hand the user an archive their machine cannot run.
        let current = Version::parse("0.1.0").unwrap();
        assert!(
            available_update(
                &current,
                &manifest("0.2.0", &["aarch64-apple-darwin"]),
                HERE
            )
            .is_none()
        );
    }

    #[test]
    fn an_unparseable_version_is_not_an_update() {
        let current = Version::parse("0.1.0").unwrap();
        assert!(available_update(&current, &manifest("nightly", &[HERE]), HERE).is_none());
    }
}
