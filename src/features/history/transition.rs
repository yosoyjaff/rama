//! Changing which commits the list shows without the rows jumping: folding
//! a branch, switching to the branch tips, searching.
//!
//! While the change animates, the list holds both the old and the new rows.
//! Rows only in the new list grow in where they will end up; rows only in the
//! old list shrink away right under the last row they shared with the new
//! one, so the rows around them close up instead of reshuffling.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use gpui_kit::{Context, ListOffset, Pixels, px};

use super::HistoryView;
use crate::git::Commit;
use crate::ui::motion;

/// What a row does while the list changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HistoryRowTransition {
    Stable,
    Entering,
    Exiting,
}

/// A change in progress: the rows on screen and what each is doing, and the
/// list to settle on once it is done.
pub(super) struct ScopeChange {
    pub rows: Vec<HistoryRowTransition>,
    pub final_commits: Vec<Commit>,
    pub final_collapsed_counts: HashMap<String, usize>,
    pub epoch: usize,
}

/// The commit the view is scrolled to, and how far into its row.
#[derive(Clone)]
pub(super) struct ScrollAnchor {
    pub sha: String,
    pub offset_in_item: Pixels,
}

fn shas(commits: &[Commit]) -> HashSet<&str> {
    commits.iter().map(|c| c.sha.as_str()).collect()
}

/// Where to keep the view scrolled once `old` becomes `target`. If the
/// commit at the top is still there it stays; otherwise the nearest one
/// below it that survives takes its place, or failing that the nearest above.
pub(super) fn resolve_history_scroll_anchor(
    anchor: Option<ScrollAnchor>,
    old: &[Commit],
    target: &[Commit],
) -> Option<ScrollAnchor> {
    let anchor = anchor?;
    let kept = shas(target);
    if kept.contains(anchor.sha.as_str()) {
        return Some(anchor);
    }
    let at = old.iter().position(|c| c.sha == anchor.sha)?;
    let below = old[at + 1..].iter();
    let above = old[..at].iter().rev();
    let survivor = below.chain(above).find(|c| kept.contains(c.sha.as_str()))?;
    Some(ScrollAnchor {
        sha: survivor.sha.clone(),
        offset_in_item: px(0.0),
    })
}

/// The part of the list that differs between `old` and `target`: the range
/// of old rows to replace, and how many new rows go there. The "load more"
/// row at the end counts as a row. `None` when nothing changed.
pub(super) fn history_list_splice(
    old: &[Commit],
    old_has_load_more: bool,
    target: &[Commit],
    target_has_load_more: bool,
) -> Option<(Range<usize>, usize)> {
    fn keys(commits: &[Commit], load_more: bool) -> Vec<&str> {
        let mut keys: Vec<&str> = commits.iter().map(|c| c.sha.as_str()).collect();
        if load_more {
            // No sha can look like this.
            keys.push("\0load-more");
        }
        keys
    }
    let (before, after) = (
        keys(old, old_has_load_more),
        keys(target, target_has_load_more),
    );
    if before == after {
        return None;
    }
    let same_start = before
        .iter()
        .zip(&after)
        .take_while(|(a, b)| a == b)
        .count();
    let room = before.len().min(after.len()) - same_start;
    let same_end = before
        .iter()
        .rev()
        .zip(after.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    Some((
        same_start..before.len() - same_end,
        after.len() - same_start - same_end,
    ))
}

/// The rows to show while `old` turns into `target`, and what each does.
pub(super) fn history_transition_rows(
    old: &[Commit],
    target: &[Commit],
) -> (Vec<Commit>, Vec<HistoryRowTransition>) {
    let (staying, existing) = (shas(target), shas(old));
    // The old-only rows, grouped under the last shared row above them; those
    // above every shared row go first.
    let mut leading: Vec<&Commit> = Vec::new();
    let mut under: HashMap<&str, Vec<&Commit>> = HashMap::new();
    let mut last_shared: Option<&str> = None;
    for commit in old {
        match (staying.contains(commit.sha.as_str()), last_shared) {
            (true, _) => last_shared = Some(&commit.sha),
            (false, Some(shared)) => under.entry(shared).or_default().push(commit),
            (false, None) => leading.push(commit),
        }
    }

    let mut rows: Vec<(Commit, HistoryRowTransition)> =
        Vec::with_capacity(old.len() + target.len());
    let leave = |rows: &mut Vec<_>, commits: Vec<&Commit>| {
        rows.extend(
            commits
                .into_iter()
                .map(|c| (c.clone(), HistoryRowTransition::Exiting)),
        );
    };
    leave(&mut rows, leading);
    for commit in target {
        let what = match existing.contains(commit.sha.as_str()) {
            true => HistoryRowTransition::Stable,
            false => HistoryRowTransition::Entering,
        };
        rows.push((commit.clone(), what));
        if let Some(leaving) = under.remove(commit.sha.as_str()) {
            leave(&mut rows, leaving);
        }
    }
    // Only possible with a repeated sha in `old`; the rows still leave.
    for (_, leaving) in under {
        leave(&mut rows, leaving);
    }
    rows.into_iter().unzip()
}

impl HistoryView {
    /// The commit at the top of the list and how far into its row, to come
    /// back to after the rows change.
    fn top_anchor(&self) -> Option<ScrollAnchor> {
        let top = self.list.logical_scroll_top();
        let rows = &self.shown_rows;
        let (commit, offset) = match rows.get(top.item_ix) {
            Some(commit) => (commit, top.offset_in_item),
            // On the "load more" row: hold on to the last commit.
            None => (rows.last()?, px(0.0)),
        };
        Some(ScrollAnchor {
            sha: commit.sha.clone(),
            offset_in_item: offset,
        })
    }

    fn return_to(&self, anchor: Option<ScrollAnchor>) {
        let Some(anchor) = anchor else {
            return;
        };
        if let Some(item_ix) = self.shown_rows.iter().position(|c| c.sha == anchor.sha) {
            self.list.scroll_to(ListOffset {
                item_ix,
                offset_in_item: anchor.offset_in_item,
            });
        }
    }

    /// Swap the rows for `rows`, telling the list only what moved, and keep
    /// the view on the commit it was on (or the nearest that survives).
    fn swap_rows(&mut self, rows: Vec<Commit>, hidden: HashMap<String, usize>) {
        let anchor = resolve_history_scroll_anchor(self.top_anchor(), &self.shown_rows, &rows);
        let had_more = self.list.item_count() > self.shown_rows.len();
        let has_more = self.has_more_row();
        if let Some((range, count)) =
            history_list_splice(&self.shown_rows, had_more, &rows, has_more)
        {
            self.list.splice(range, count);
        }
        self.shown_rows = rows;
        self.folds.hidden = hidden;
        self.relayout_graph();
        self.return_to(anchor);
    }

    /// End a running fold animation on its final rows.
    pub(super) fn finish_transition(&mut self, cx: &mut Context<Self>) {
        let Some(done) = self.scope_change.take() else {
            return;
        };
        self.scope_task = None;
        self.swap_rows(done.final_commits, done.final_collapsed_counts);
        self.search.rematch(&self.shown_rows);
        cx.notify();
    }

    /// Show the rows the view mode and folds call for now. With `animate`
    /// (and motion allowed), rows that come or go ease in and out.
    pub(super) fn change_view(&mut self, animate: bool, cx: &mut Context<Self>) {
        // A change during an animation starts from where that one was going.
        self.finish_transition(cx);
        let before: Vec<String> = self.shown_rows.iter().map(|c| c.sha.clone()).collect();
        let (after, hidden) = self.derive_rows();
        let unchanged = after.iter().map(|c| &c.sha).eq(before.iter());
        if unchanged || !animate || motion::reduced_motion(cx) {
            self.swap_rows(after, hidden);
            self.search.rematch(&self.shown_rows);
            cx.notify();
            return;
        }

        let (during, rows) = history_transition_rows(&self.shown_rows, &after);
        self.swap_rows(during, hidden.clone());
        self.search.rematch(&self.shown_rows);
        let epoch = self.scope_epoch;
        self.scope_change = Some(ScopeChange {
            rows,
            final_commits: after,
            final_collapsed_counts: hidden,
            epoch,
        });
        let length = motion::COLLAPSE.length();
        self.scope_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(length).await;
            this.update(cx, |history, cx| {
                let current = history.scope_change.as_ref().map(|t| t.epoch);
                if current == Some(epoch) {
                    history.finish_transition(cx);
                }
            })
            .ok();
        }));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HistoryRowTransition::{Entering, Exiting, Stable};

    fn list(shas: &[&str]) -> Vec<Commit> {
        shas.iter()
            .map(|sha| Commit {
                sha: sha.to_string(),
                ..Commit::default()
            })
            .collect()
    }

    fn order(commits: &[Commit]) -> Vec<&str> {
        commits.iter().map(|c| c.sha.as_str()).collect()
    }

    #[test]
    fn leaving_rows_wait_under_the_row_they_followed() {
        let (rows, what) =
            history_transition_rows(&list(&["x", "a", "y", "z", "b"]), &list(&["a", "b"]));
        assert_eq!(order(&rows), ["x", "a", "y", "z", "b"]);
        assert_eq!(what, [Exiting, Stable, Exiting, Exiting, Stable]);
    }

    #[test]
    fn arriving_rows_take_their_final_place() {
        let (rows, what) =
            history_transition_rows(&list(&["a", "b"]), &list(&["a", "n", "b", "m"]));
        assert_eq!(order(&rows), ["a", "n", "b", "m"]);
        assert_eq!(what, [Stable, Entering, Stable, Entering]);
    }

    #[test]
    fn the_splice_covers_only_what_changed() {
        let old = list(&["a", "b", "c", "d"]);
        assert_eq!(
            history_list_splice(&old, true, &list(&["a", "d"]), true),
            Some((1..3, 0))
        );
        assert_eq!(
            history_list_splice(&old, false, &list(&["a", "x", "b", "c", "d"]), false),
            Some((1..1, 1))
        );
        assert_eq!(
            history_list_splice(&old, true, &old, false),
            Some((4..5, 0)),
            "the load-more row went"
        );
        assert_eq!(history_list_splice(&old, true, &old, true), None);
    }

    #[test]
    fn a_vanished_anchor_moves_down_first_then_up() {
        let old = list(&["a", "b", "c", "d"]);
        let anchor = |sha: &str| {
            Some(ScrollAnchor {
                sha: sha.into(),
                offset_in_item: px(5.0),
            })
        };
        let kept = resolve_history_scroll_anchor(anchor("c"), &old, &list(&["c"])).unwrap();
        assert_eq!((kept.sha.as_str(), kept.offset_in_item), ("c", px(5.0)));
        let moved = resolve_history_scroll_anchor(anchor("b"), &old, &list(&["a", "d"])).unwrap();
        assert_eq!((moved.sha.as_str(), moved.offset_in_item), ("d", px(0.0)));
        let up = resolve_history_scroll_anchor(anchor("d"), &old, &list(&["a"])).unwrap();
        assert_eq!(up.sha, "a");
        assert!(resolve_history_scroll_anchor(anchor("q"), &old, &list(&["a"])).is_none());
    }
}
