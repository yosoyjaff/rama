//! The release source this application ships with: the `latest.json` the
//! release workflow attaches to the newest GitHub release.
//!
//! `releases/latest/download/<asset>` is a plain download URL that GitHub
//! redirects to the newest non-prerelease, so the check is one anonymous GET
//! with no API rate limit and no token in the binary. The repository has to be
//! public for that to answer; a private one answers 404 to everyone outside.

use std::time::Duration;

use anyhow::{Context as _, bail};

use super::manifest::ReleaseManifest;
use super::source::{FetchManifest, ReleaseSource};

/// The manifest of the newest release of `repository`, e.g.
/// `https://github.com/owner/name`.
pub struct GithubSource {
    url: String,
}

impl GithubSource {
    pub fn new(repository: &str) -> Self {
        Self {
            url: format!(
                "{}/releases/latest/download/latest.json",
                repository.trim_end_matches('/')
            ),
        }
    }
}

/// A manifest is a few kilobytes; anything near this is not one.
const MAX_MANIFEST_BYTES: u64 = 1 << 20;

impl ReleaseSource for GithubSource {
    fn fetch(&self) -> FetchManifest {
        let url = self.url.clone();
        // Runs on the background executor (see `UpdateState::check`), so the
        // blocking client is fine here and keeps the app free of a runtime.
        Box::pin(async move {
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(15)))
                .http_status_as_error(false)
                .build()
                .new_agent();
            let mut response = agent
                .get(&url)
                .header("User-Agent", crate::APP_NAME)
                .call()
                .context("could not reach GitHub")?;
            match response.status().as_u16() {
                200 => {}
                // GitHub answers 404 both when nothing is published and when
                // the repository is private: it does not tell an anonymous
                // caller which.
                404 => bail!("no public release found"),
                status => bail!("GitHub answered {status}"),
            }
            let body = response
                .body_mut()
                .with_config()
                .limit(MAX_MANIFEST_BYTES)
                .read_to_vec()
                .context("the release manifest did not download")?;
            serde_json::from_slice::<ReleaseManifest>(&body)
                .context("the release manifest is not valid")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_url_is_the_latest_release_asset() {
        assert_eq!(
            GithubSource::new("https://github.com/o/r/").url,
            "https://github.com/o/r/releases/latest/download/latest.json"
        );
    }
}
