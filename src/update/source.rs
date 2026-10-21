//! Where a release manifest comes from.
//!
//! The transport is a trait so the application can compile, run and be tested
//! without one: nothing in this crate opens a socket. A real deployment
//! implements this over HTTPS — typically one `GET` of the channel's
//! `latest.json` with a short timeout — and hands the implementation to
//! [`super::state::UpdateState`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;

use super::manifest::ReleaseManifest;

/// A future a release source returns. Boxed because the trait is used as an
/// object, and `Send` because the fetch runs on GPUI's background executor.
pub type FetchManifest = Pin<Box<dyn Future<Output = Result<ReleaseManifest>> + Send>>;

pub trait ReleaseSource: Send + Sync + 'static {
    /// Fetch the channel's current manifest.
    ///
    /// Failure is expected and ordinary — offline, captive portal, a 500 — and
    /// must surface as a recoverable error, never as a panic or a silent
    /// "up to date".
    fn fetch(&self) -> FetchManifest;
}

/// A source that answers with a manifest it already holds.
///
/// Used by tests and by a development build that wants the update path
/// exercised without a server.
#[derive(Clone)]
pub struct StaticSource(Arc<ReleaseManifest>);

impl StaticSource {
    pub fn new(manifest: ReleaseManifest) -> Self {
        Self(Arc::new(manifest))
    }
}

impl ReleaseSource for StaticSource {
    fn fetch(&self) -> FetchManifest {
        let manifest = self.0.clone();
        Box::pin(async move { Ok((*manifest).clone()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{Context as TaskContext, Poll, Waker};

    /// Drive a future that resolves without ever yielding.
    ///
    /// Enough for [`StaticSource`], and it keeps the test free of an executor:
    /// a source that needs one is by definition doing I/O, which belongs in an
    /// integration test against a real server.
    fn resolve(mut fetch: FetchManifest) -> Result<ReleaseManifest> {
        let mut cx = TaskContext::from_waker(Waker::noop());
        match fetch.as_mut().poll(&mut cx) {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("a static source must answer without yielding"),
        }
    }

    #[test]
    fn a_static_source_answers_with_the_manifest_it_holds() {
        let source = StaticSource::new(ReleaseManifest::new("9.9.9"));
        let manifest = resolve(source.fetch()).expect("static source cannot fail");
        assert_eq!(manifest.version, "9.9.9");
    }

    #[test]
    fn a_static_source_can_be_fetched_more_than_once() {
        // The state machine re-checks; a source that answered once and then
        // went empty would look like "up to date" forever.
        let source = StaticSource::new(ReleaseManifest::new("1.2.3"));
        assert_eq!(resolve(source.fetch()).unwrap().version, "1.2.3");
        assert_eq!(resolve(source.fetch()).unwrap().version, "1.2.3");
    }
}
