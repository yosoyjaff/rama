//! Pictures for the history: each author's photo beside their commits, and
//! each remote owner's on the remote's badges.
//!
//! Lookups (the forge, then Gravatar, behind a disk cache) run in the
//! background. Until one lands, or when offline, the initials or the forge's
//! logo stay.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{Context, Image, Task};

use super::HistoryView;
use super::cells::sniff_image_format;
use crate::git::{GitRemote, avatars};

#[derive(Default)]
pub(super) struct Photos {
    /// By lowercase email.
    pub authors: HashMap<String, Arc<Image>>,
    pub authors_task: Option<Task<()>>,
    /// By remote name.
    pub remotes: HashMap<String, Arc<Image>>,
    pub remotes_task: Option<Task<()>>,
}

impl Photos {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// An email as the photo map keys it.
pub(super) fn email_key(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

impl HistoryView {
    /// Look up the photos of authors on screen that have none yet.
    pub(super) fn want_author_photos(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.checkout.shown.clone() else {
            return;
        };
        let mut missing: Vec<String> = Vec::new();
        for commit in &self.shown_rows {
            let key = email_key(&commit.author.email);
            if !key.is_empty() && !self.photos.authors.contains_key(&key) && !missing.contains(&key)
            {
                missing.push(key);
            }
        }
        if missing.is_empty() {
            return;
        }
        let repo = PathBuf::from(&root);
        let cache = avatars::cache_dir();
        let job = cx
            .background_executor()
            .spawn(async move { avatars::resolve(&repo, &missing, &cache) });
        self.photos.authors_task = Some(cx.spawn(async move |this, cx| {
            let found = job.await;
            this.update(cx, |history, cx| {
                if history.checkout.shown.as_deref() != Some(root.as_str()) {
                    return;
                }
                for (email, path) in found {
                    if let Some(image) = load_image(&path) {
                        history.photos.authors.entry(email).or_insert(image);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Look up each remote's owner photo, for its badges.
    pub(super) fn want_remote_photos(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.checkout.shown.clone() else {
            return;
        };
        let missing: Vec<GitRemote> = self
            .repo
            .iter()
            .flat_map(|repo| &repo.remotes)
            .filter(|remote| !self.photos.remotes.contains_key(&remote.name))
            .cloned()
            .collect();
        if missing.is_empty() {
            return;
        }
        let cache = avatars::cache_dir();
        let job = cx.background_executor().spawn(async move {
            missing
                .into_iter()
                .filter_map(|remote| {
                    let path = avatars::remote_owner_avatar(&remote.url, &cache)?;
                    Some((remote.name, path))
                })
                .collect::<Vec<_>>()
        });
        self.photos.remotes_task = Some(cx.spawn(async move |this, cx| {
            let found = job.await;
            this.update(cx, |history, cx| {
                if history.checkout.shown.as_deref() != Some(root.as_str()) {
                    return;
                }
                for (name, path) in found {
                    if let Some(image) = load_image(&path) {
                        history.photos.remotes.insert(name, image);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

/// A cached photo, if the file holds an image.
fn load_image(path: &Path) -> Option<Arc<Image>> {
    let bytes = std::fs::read(path).ok()?;
    let format = sniff_image_format(&bytes)?;
    Some(Arc::new(Image::from_bytes(format, bytes)))
}
