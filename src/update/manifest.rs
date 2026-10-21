//! The release manifest: what CI publishes next to the binaries, and the only
//! thing a running build has to parse to know an update exists.
//!
//! One file per channel, e.g. `https://…/stable/latest.json`:
//!
//! ```json
//! {
//!   "version": "0.2.0",
//!   "notes": "https://…/releases/v0.2.0",
//!   "published_at": "2026-09-21T10:00:00Z",
//!   "artifacts": [
//!     {
//!       "target": "x86_64-unknown-linux-gnu",
//!       "url": "https://…/rama-0.2.0-x86_64-unknown-linux-gnu.tar.gz",
//!       "size": 18234123,
//!       "sha256": "…",
//!       "signature": "…"
//!     }
//!   ]
//! }
//! ```

use serde::{Deserialize, Serialize};

/// One downloadable build.
///
/// `sha256` is mandatory and `signature` is not, so a project can ship
/// integrity now and authenticity when it has a signing key. Verify both
/// before writing anything to disk: an update channel is a code-execution
/// path, and a manifest is fetched over a network somebody else may own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReleaseArtifact {
    /// The Rust target triple this build runs on, e.g.
    /// `aarch64-apple-darwin`. Matched against [`crate::current_target`].
    pub target: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// One release, across every target it was built for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReleaseManifest {
    /// Semver, without a leading `v`.
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    pub artifacts: Vec<ReleaseArtifact>,
}

impl ReleaseManifest {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            notes: None,
            published_at: None,
            artifacts: Vec::new(),
        }
    }

    pub fn with_artifact(mut self, artifact: ReleaseArtifact) -> Self {
        self.artifacts.push(artifact);
        self
    }

    /// The build for `target`, if this release has one. A release that skipped
    /// a platform is not an update *for that platform*.
    pub fn artifact_for(&self, target: &str) -> Option<&ReleaseArtifact> {
        self.artifacts
            .iter()
            .find(|artifact| artifact.target == target)
    }
}
