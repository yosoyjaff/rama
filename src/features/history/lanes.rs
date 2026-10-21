//! Where each commit of the graph sits, and which lines run through its row.
//!
//! Commits come newest first, every child before its parents. Going down the
//! list, each lane is a vertical line waiting for the commit it leads to.
//! When that commit arrives, it takes the leftmost lane waiting for it, and
//! any other lane waiting for it bends into it on its row. Then its lane goes
//! on waiting for its first parent; any further parent gets a lane of its
//! own, or joins the one already waiting for it.
//!
//! A line always keeps to the lane of the commit it leaves until the row of
//! the commit it reaches, so the bends are where the commits are. A line's
//! colour is its lane's, which is why no lane ever shows two colours.
//!
//! The rest of this module prepares the list the layout runs on: stashes put
//! in among the commits, folded branches, and the search results joined up.

use super::WORKTREE_ROW;
use std::collections::{HashMap, HashSet};

use crate::git::{Commit, RefBadge, RefKind};

/// A line on one row. It is drawn in its lane's colour: the lane it runs
/// in away from the commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Line {
    /// Runs past the commit, straight down `lane`.
    Past { lane: usize, dashed: bool },
    /// Comes down `lane` and bends into the commit.
    Into { lane: usize, dashed: bool },
    /// Leaves the commit and bends down into `lane`.
    Out { lane: usize, dashed: bool },
}

impl Line {
    pub fn lane(self) -> usize {
        match self {
            Self::Past { lane, .. } | Self::Into { lane, .. } | Self::Out { lane, .. } => lane,
        }
    }

    /// Leaves a stash or the working tree, neither of which is committed
    /// where it hangs.
    pub fn dashed(self) -> bool {
        match self {
            Self::Past { dashed, .. } | Self::Into { dashed, .. } | Self::Out { dashed, .. } => {
                dashed
            }
        }
    }
}

/// What sits on a commit's spot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    /// The author's photo.
    Commit,
    /// A small disc.
    Merge,
    /// A dotted square.
    Stash,
    /// The ring of uncommitted changes, even mid-merge with two parents.
    WorkingTree,
}

/// One commit's row of the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub sha: String,
    /// Where the commit sits, which is also its colour.
    pub lane: usize,
    pub mark: Mark,
    pub head: bool,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Graph {
    pub rows: Vec<Row>,
    /// The most lanes any row uses.
    pub lanes: usize,
}

/// What a lane is waiting for.
#[derive(Debug, Clone, Copy)]
struct Waiting<'a> {
    sha: &'a str,
    dashed: bool,
}

/// The lanes, left to right; `None` is a free one.
struct Lanes<'a>(Vec<Option<Waiting<'a>>>);

impl<'a> Lanes<'a> {
    /// The leftmost free lane, made if need be.
    fn claim(&mut self, what: Waiting<'a>) -> usize {
        let lane = match self.0.iter().position(Option::is_none) {
            Some(free) => free,
            None => {
                self.0.push(None);
                self.0.len() - 1
            }
        };
        self.0[lane] = Some(what);
        lane
    }

    fn waiting_for(&self, sha: &str) -> impl Iterator<Item = (usize, Waiting<'a>)> + '_ {
        let sha = sha.to_owned();
        self.0
            .iter()
            .enumerate()
            .filter_map(move |(lane, slot)| slot.filter(|w| w.sha == sha).map(|w| (lane, w)))
    }

    fn trim(&mut self) {
        while matches!(self.0.last(), Some(None)) {
            self.0.pop();
        }
    }
}

/// Lay out the graph for `commits`, which must list every child before its
/// parents.
pub(super) fn lay_out(commits: &[Commit], head_sha: Option<&str>) -> Graph {
    let present: HashSet<&str> = commits.iter().map(|c| c.sha.as_str()).collect();
    let mut lanes = Lanes(Vec::new());
    let mut graph = Graph::default();

    for commit in commits {
        let sha = commit.sha.as_str();
        let arriving: Vec<(usize, Waiting)> = lanes.waiting_for(sha).collect();

        // Every lane not waiting for this commit runs past it.
        let mut lines: Vec<Line> = lanes
            .0
            .iter()
            .enumerate()
            .filter_map(|(lane, slot)| {
                let waiting = slot.filter(|waiting| waiting.sha != sha)?;
                Some(Line::Past {
                    lane,
                    dashed: waiting.dashed,
                })
            })
            .collect();

        // The commit sits in the leftmost lane that leads to it; the others
        // bend in and end here. A commit no one leads to (a branch tip) takes
        // the first free lane.
        let at = match arriving.first() {
            Some(&(lane, _)) => lane,
            None => lanes.claim(Waiting { sha, dashed: false }),
        };
        for &(lane, waiting) in &arriving {
            lines.push(Line::Into {
                lane,
                dashed: waiting.dashed,
            });
            lanes.0[lane] = None;
        }

        // Lines from a stash or the working tree are dashed all the way to
        // their parent.
        let working_tree = sha == WORKTREE_ROW;
        let dashed = commit.is_stash || working_tree;
        let mut leaving = Vec::new();
        for (i, parent) in commit.parents.iter().enumerate() {
            let next = Waiting {
                sha: parent.as_str(),
                dashed,
            };
            if i == 0 {
                // The first parent carries on in this lane, even past the
                // end of the page.
                lanes.0[at] = Some(next);
                leaving.push(at);
                continue;
            }
            // Another parent needs to be on the page for a line to it.
            if !present.contains(parent.as_str()) || commit.parents[..i].contains(parent) {
                continue;
            }
            let waiting = lanes.waiting_for(parent).next().map(|(lane, _)| lane);
            leaving.push(waiting.unwrap_or_else(|| lanes.claim(next)));
        }
        lanes.trim();
        leaving.sort_unstable();
        lines.extend(leaving.into_iter().map(|lane| Line::Out { lane, dashed }));

        let widest = lines
            .iter()
            .map(|line| line.lane())
            .chain([at, lanes.0.len().saturating_sub(1)])
            .max()
            .unwrap_or(0)
            + 1;
        graph.lanes = graph.lanes.max(widest);
        graph.rows.push(Row {
            sha: commit.sha.clone(),
            lane: at,
            mark: match () {
                _ if working_tree => Mark::WorkingTree,
                _ if commit.is_stash => Mark::Stash,
                _ if commit.parents.len() > 1 => Mark::Merge,
                _ => Mark::Commit,
            },
            head: head_sha == Some(sha),
            lines,
        });
    }
    graph
}

/// Put each stash in among the commits, above the commit it was taken from:
/// as high as its date says, but never above a newer commit, and never below
/// its parent, which would draw its line upwards. A stash whose parent is not
/// in the list is left out, as its line would lead nowhere.
pub(super) fn merge_stash_rows(commits: Vec<Commit>, stashes: &[Commit]) -> Vec<Commit> {
    if stashes.is_empty() {
        return commits;
    }
    let time = |c: &Commit| {
        chrono::DateTime::parse_from_rfc3339(&c.author.when).map_or(0, |d| d.timestamp())
    };
    let row_of: HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.sha.as_str(), i))
        .collect();
    // For each row, the stashes that go just above it.
    let mut above: HashMap<usize, Vec<Commit>> = HashMap::new();
    for stash in stashes {
        let Some(&parent_row) = stash.parents.first().and_then(|p| row_of.get(p.as_str())) else {
            continue;
        };
        let when = time(stash);
        let row = (0..parent_row)
            .find(|&i| time(&commits[i]) <= when)
            .unwrap_or(parent_row);
        above.entry(row).or_default().push(stash.clone());
    }
    let mut merged = Vec::with_capacity(commits.len() + stashes.len());
    for (i, commit) in commits.into_iter().enumerate() {
        merged.extend(above.remove(&i).unwrap_or_default());
        merged.push(commit);
    }
    merged
}

/// Keep only `visible` commits, in order, with each parent that was left
/// out replaced by its own nearest kept ancestors, so the graph still joins
/// every commit to what it came from.
pub(super) fn compact_commits_to_visible(
    commits: &[Commit],
    visible: &HashSet<String>,
) -> Vec<Commit> {
    // What each dropped commit stands for: its nearest kept ancestors. The
    // list goes child first, so walking it backwards meets every parent
    // before its children, and each answer is ready when it is needed.
    let mut stands_for: HashMap<&str, Vec<String>> = HashMap::new();
    let resolve = |parents: &[String], known: &HashMap<&str, Vec<String>>| {
        let mut out: Vec<String> = Vec::new();
        for parent in parents {
            let found = match known.get(parent.as_str()) {
                Some(ancestors) => ancestors.clone(),
                None => vec![parent.clone()],
            };
            for ancestor in found {
                if !out.contains(&ancestor) {
                    out.push(ancestor);
                }
            }
        }
        out
    };
    for commit in commits.iter().rev() {
        if !visible.contains(&commit.sha) {
            let ancestors = resolve(&commit.parents, &stands_for);
            stands_for.insert(&commit.sha, ancestors);
        }
    }
    commits
        .iter()
        .filter(|commit| visible.contains(&commit.sha))
        .map(|commit| Commit {
            parents: resolve(&commit.parents, &stands_for),
            ..commit.clone()
        })
        .collect()
}

/// The name a branch is folded under, telling a local branch from a remote
/// one of the same name. Tags do not fold.
pub(super) fn branch_ref_key(reference: &RefBadge) -> Option<String> {
    match reference.kind {
        RefKind::Branch => Some(format!("local:{}", reference.label)),
        RefKind::Remote => Some(format!("remote:{}", reference.label)),
        RefKind::Tag => None,
    }
}

/// Fold the branches named in `folded`: the plain commits along their lanes
/// are dropped, keeping their tips, anything with a ref, merges, roots and
/// commits where history splits. Returns the commits left, and how many
/// were dropped for each folded branch.
pub(super) fn collapse_branch_runs(
    commits: &[Commit],
    folded: &HashSet<String>,
    head_sha: Option<&str>,
) -> (Vec<Commit>, HashMap<String, usize>) {
    if folded.is_empty() || commits.is_empty() {
        return (commits.to_vec(), HashMap::new());
    }
    let graph = lay_out(commits, head_sha);
    let folded_ref = |commit: &Commit| {
        commit
            .refs
            .iter()
            .filter_map(branch_ref_key)
            .find(|key| folded.contains(key))
    };
    // A folded branch is the lane its tip sits in.
    let mut lane_of: HashMap<String, usize> = HashMap::new();
    for (commit, row) in commits.iter().zip(&graph.rows) {
        for key in commit.refs.iter().filter_map(branch_ref_key) {
            if folded.contains(&key) {
                lane_of.insert(key, row.lane);
            }
        }
    }
    if lane_of.is_empty() {
        return (commits.to_vec(), HashMap::new());
    }
    let mut children: HashMap<&str, usize> = HashMap::new();
    for parent in commits.iter().flat_map(|c| &c.parents) {
        *children.entry(parent.as_str()).or_default() += 1;
    }

    let mut visible = HashSet::new();
    let mut dropped: HashMap<String, usize> = HashMap::new();
    for (commit, row) in commits.iter().zip(&graph.rows) {
        let plain = commit.refs.is_empty()
            && folded_ref(commit).is_none()
            && commit.parents.len() == 1
            && children.get(commit.sha.as_str()).copied().unwrap_or(0) <= 1;
        let owners: Vec<&String> = lane_of
            .iter()
            .filter(|(_, lane)| **lane == row.lane)
            .map(|(key, _)| key)
            .collect();
        if plain && !owners.is_empty() {
            for key in owners {
                *dropped.entry(key.clone()).or_default() += 1;
            }
        } else {
            visible.insert(commit.sha.clone());
        }
    }
    (compact_commits_to_visible(commits, &visible), dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, parents: &[&str]) -> Commit {
        Commit {
            sha: sha.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            ..Commit::default()
        }
    }

    fn lanes_of(graph: &Graph) -> Vec<usize> {
        graph.rows.iter().map(|row| row.lane).collect()
    }

    /// Follow the lines down the rows and check they really join each commit
    /// on the page to each parent on the page: whatever leaves a row in a
    /// lane is in that lane on the next row, and ends at a commit it leads to.
    fn assert_connected(commits: &[Commit], graph: &Graph) {
        let present: HashSet<&str> = commits.iter().map(|c| c.sha.as_str()).collect();
        // Lane → the commit whose line it carries.
        let mut open: HashMap<usize, String> = HashMap::new();
        for (commit, row) in commits.iter().zip(&graph.rows) {
            let mut below: HashMap<usize, String> = HashMap::new();
            for line in &row.lines {
                match *line {
                    Line::Past { lane, .. } => {
                        let from = open.get(&lane).expect("a line runs into this row");
                        below.insert(lane, from.clone());
                    }
                    Line::Into { lane, .. } => {
                        let from = open.get(&lane).expect("a line arrives");
                        let child = commits.iter().find(|c| &c.sha == from).unwrap();
                        assert!(
                            child.parents.contains(&commit.sha),
                            "{from} does not lead to {}",
                            commit.sha
                        );
                    }
                    Line::Out { lane, .. } => {
                        below.insert(lane, commit.sha.clone());
                    }
                }
            }
            open = below;
            if commit.parents.iter().any(|p| present.contains(p.as_str())) {
                assert!(
                    open.values().any(|from| from == &commit.sha),
                    "{} lost its line",
                    commit.sha
                );
            }
        }
    }

    #[test]
    fn a_straight_history_stays_in_one_lane() {
        let commits = [commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])];
        let graph = lay_out(&commits, Some("c"));
        assert_eq!(lanes_of(&graph), [0, 0, 0]);
        assert_eq!(graph.lanes, 1);
        assert!(graph.rows[0].head && !graph.rows[1].head);
        assert_eq!(
            graph.rows[2].lines,
            [Line::Into {
                lane: 0,
                dashed: false
            }],
            "a root only receives"
        );
        assert_connected(&commits, &graph);
    }

    #[test]
    fn a_merge_opens_a_lane_for_its_second_parent_and_closes_it_at_the_fork() {
        let commits = [
            commit("merge", &["main", "topic"]),
            commit("main", &["base"]),
            commit("topic", &["base"]),
            commit("base", &[]),
        ];
        let graph = lay_out(&commits, Some("merge"));
        assert_eq!(lanes_of(&graph), [0, 0, 1, 0]);
        assert_eq!(graph.lanes, 2);
        assert_eq!(graph.rows[0].mark, Mark::Merge);
        let into = |lane| Line::Into {
            lane,
            dashed: false,
        };
        assert_eq!(graph.rows[3].lines, [into(0), into(1)]);
        assert_connected(&commits, &graph);
    }

    #[test]
    fn the_trunk_keeps_its_lane_whichever_side_comes_first() {
        let branch_first = [
            commit("merge", &["main", "topic"]),
            commit("topic", &["base"]),
            commit("main", &["base"]),
            commit("base", &["root"]),
            commit("root", &[]),
        ];
        let graph = lay_out(&branch_first, None);
        assert_eq!(lanes_of(&graph), [0, 1, 0, 0, 0]);
        assert_connected(&branch_first, &graph);
    }

    #[test]
    fn freed_lanes_are_reused_from_the_left() {
        let commits = [
            commit("m2", &["m1", "b"]),
            commit("b", &["m1"]),
            commit("m1", &["m0", "a"]),
            commit("a", &["m0"]),
            commit("m0", &[]),
        ];
        let graph = lay_out(&commits, None);
        assert_eq!(lanes_of(&graph), [0, 1, 0, 1, 0]);
        assert_eq!(graph.lanes, 2);
        assert_connected(&commits, &graph);
    }

    #[test]
    fn parents_off_the_page_keep_their_line_only_for_the_first() {
        let commits = [commit("merge", &["older", "elsewhere"]), commit("x", &[])];
        let graph = lay_out(&commits, None);
        assert_eq!(
            graph.rows[0].lines.len(),
            1,
            "no line to a parent not shown"
        );
        // The first parent's line runs on past the next row, to the edge.
        assert!(
            graph.rows[1]
                .lines
                .iter()
                .any(|line| matches!(line, Line::Past { lane: 0, .. }))
        );
        assert_eq!(graph.rows[1].lane, 1);
    }

    #[test]
    fn a_tangle_of_merges_stays_connected() {
        let commits = [
            commit("h", &["g", "e"]),
            commit("g", &["f", "d"]),
            commit("f", &["c"]),
            commit("e", &["c"]),
            commit("d", &["b"]),
            commit("c", &["b", "a"]),
            commit("b", &["a"]),
            commit("a", &[]),
        ];
        let graph = lay_out(&commits, Some("h"));
        assert_connected(&commits, &graph);
        assert!(graph.lanes >= 3);
    }

    #[test]
    fn loading_more_commits_does_not_move_the_ones_shown() {
        let page = [commit("c", &["b"]), commit("b", &["a"])];
        let more = [commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])];
        let before = lay_out(&page, Some("c"));
        let after = lay_out(&more, Some("c"));
        assert_eq!(before.rows, after.rows[..2]);
    }

    #[test]
    fn lines_from_a_stash_or_the_working_tree_are_dashed_to_the_parent() {
        let mut stash = commit("s", &["b"]);
        stash.is_stash = true;
        let commits = [
            commit(WORKTREE_ROW, &["c"]),
            commit("c", &["b"]),
            stash,
            commit("b", &[]),
        ];
        let graph = lay_out(&commits, Some("c"));
        let dashed = |row: usize| -> Vec<bool> {
            graph.rows[row]
                .lines
                .iter()
                .map(|line| line.dashed())
                .collect()
        };
        assert_eq!(dashed(0), [true]);
        assert_eq!(graph.rows[0].mark, Mark::WorkingTree);
        assert_eq!(graph.rows[2].mark, Mark::Stash);
        // At b, the stash's line arrives dashed, the trunk's solid.
        let arrivals: Vec<(usize, bool)> = graph.rows[3]
            .lines
            .iter()
            .map(|line| (line.lane(), line.dashed()))
            .collect();
        assert_eq!(arrivals, [(0, false), (1, true)]);
    }

    #[test]
    fn the_working_tree_mid_merge_is_not_a_merge_node() {
        let commits = [
            commit(WORKTREE_ROW, &["ours", "theirs"]),
            commit("ours", &[]),
            commit("theirs", &[]),
        ];
        let row = &lay_out(&commits, None).rows[0];
        assert_eq!(row.mark, Mark::WorkingTree);
        assert_eq!(row.lines.len(), 2);
        assert!(row.lines.iter().all(|line| line.dashed()));
    }

    #[test]
    fn a_stash_goes_by_date_but_never_below_its_parent() {
        let dated = |sha: &str, parents: &[&str], when: &str| Commit {
            author: crate::git::Signature {
                when: when.into(),
                ..Default::default()
            },
            ..commit(sha, parents)
        };
        let commits = vec![
            dated("c", &["b"], "2026-01-03T00:00:00Z"),
            dated("b", &["a"], "2026-01-02T00:00:00Z"),
            dated("a", &[], "2026-01-01T00:00:00Z"),
        ];
        let mut recent = dated("s1", &["a"], "2026-01-02T12:00:00Z");
        recent.is_stash = true;
        let mut future = dated("s2", &["b"], "2027-01-01T00:00:00Z");
        future.is_stash = true;
        let lost = dated("s3", &["gone"], "2026-01-01T00:00:00Z");
        let order: Vec<String> = merge_stash_rows(commits, &[recent, future, lost])
            .into_iter()
            .map(|c| c.sha)
            .collect();
        assert_eq!(order, ["s2", "c", "s1", "b", "a"]);
    }

    #[test]
    fn compacting_joins_kept_commits_across_dropped_ones() {
        let commits = [
            commit("top", &["x"]),
            commit("x", &["y", "z"]),
            commit("y", &["bottom"]),
            commit("z", &["bottom"]),
            commit("bottom", &["off-page"]),
        ];
        let keep: HashSet<String> = ["top", "bottom"].map(String::from).into();
        let kept = compact_commits_to_visible(&commits, &keep);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].parents, ["bottom"]);
        assert_eq!(kept[1].parents, ["off-page"]);
    }

    #[test]
    fn compacting_a_huge_gap_needs_no_recursion() {
        let n = 20_000;
        let commits: Vec<Commit> = (0..n)
            .map(|i| {
                let parents: Vec<String> = if i + 1 < n {
                    vec![(i + 1).to_string()]
                } else {
                    Vec::new()
                };
                Commit {
                    sha: i.to_string(),
                    parents,
                    ..Commit::default()
                }
            })
            .collect();
        let keep: HashSet<String> = ["0".to_string(), (n - 1).to_string()].into();
        let kept = compact_commits_to_visible(&commits, &keep);
        assert_eq!(kept[0].parents, [(n - 1).to_string()]);
    }

    #[test]
    fn folding_a_branch_drops_its_plain_commits_and_counts_them() {
        let with_ref = |sha: &str, parents: &[&str], label: &str| Commit {
            refs: vec![RefBadge {
                kind: RefKind::Branch,
                label: label.into(),
            }],
            ..commit(sha, parents)
        };
        let commits = [
            commit("merge", &["m", "t3"]),
            with_ref("t3", &["t2"], "topic"),
            commit("m", &["base"]),
            commit("t2", &["t1"]),
            commit("t1", &["base"]),
            commit("base", &[]),
        ];
        let folded: HashSet<String> = ["local:topic".to_string()].into();
        let (kept, dropped) = collapse_branch_runs(&commits, &folded, None);
        let shas: Vec<&str> = kept.iter().map(|c| c.sha.as_str()).collect();
        assert_eq!(shas, ["merge", "t3", "m", "base"]);
        assert_eq!(dropped["local:topic"], 2);
        assert_eq!(
            kept[1].parents,
            ["base"],
            "the tip now leads straight to the fork"
        );
        assert_eq!(
            collapse_branch_runs(&commits, &HashSet::new(), None)
                .0
                .len(),
            commits.len()
        );
    }

    #[test]
    fn fold_keys_tell_local_from_remote() {
        let r = |kind, label: &str| RefBadge {
            kind,
            label: label.into(),
        };
        assert_eq!(
            branch_ref_key(&r(RefKind::Branch, "main")).as_deref(),
            Some("local:main")
        );
        assert_eq!(
            branch_ref_key(&r(RefKind::Remote, "origin/main")).as_deref(),
            Some("remote:origin/main")
        );
        assert_eq!(branch_ref_key(&r(RefKind::Tag, "v1")), None);
    }
}
