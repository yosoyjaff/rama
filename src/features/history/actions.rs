//! The graph's context menu, as plain data.
//!
//! [`menu_for`] looks at a [`Scene`] (what was right-clicked, in which
//! repository) and lists [`Item`]s. Each item carries a [`Command`] holding
//! every value it needs, so turning a pick into an [`Outcome`] is one `match`
//! over the command and the answers its form collected. No git runs here and
//! nothing is drawn: tests build a scene, pick a command and compare.

use std::collections::HashMap;

use crate::git::actions::{GitAction, RebaseOp, RebaseStep, ResetMode};
use crate::git::{RefBadge, RefKind};

use crate::git::forge::{Hosted, Page};

// ── What was clicked ────────────────────────────────────────────────────────

/// The thing a right-click points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aim {
    /// A commit, with none of its refs singled out.
    Commit,
    Branch(BranchAim),
    Tag(String),
}

/// A branch badge, local or remote-tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchAim {
    /// The badge text: `topic`, `origin/topic`.
    pub badge: String,
    /// Without the remote: `topic`.
    pub name: String,
    /// Set for a remote-tracking branch.
    pub remote: Option<String>,
}

impl BranchAim {
    /// How the forge names it: a remote branch by its name over there.
    fn forge_name(&self) -> &str {
        match self.remote {
            Some(_) => &self.name,
            None => &self.badge,
        }
    }
}

impl Aim {
    pub fn from_badge(label: &str, kind: RefKind) -> Self {
        let branch = |name: &str, remote: Option<&str>| {
            Aim::Branch(BranchAim {
                badge: label.to_owned(),
                name: name.to_owned(),
                remote: remote.map(str::to_owned),
            })
        };
        match kind {
            RefKind::Tag => Aim::Tag(label.to_owned()),
            RefKind::Branch => branch(label, None),
            RefKind::Remote => match label.split_once('/') {
                Some((remote, name)) if !remote.is_empty() && !name.is_empty() => {
                    branch(name, Some(remote))
                }
                _ => branch(label, None),
            },
        }
    }
}

/// The aim for a click on a row, off any badge.
///
/// Badges sit in a column that starts hidden, and a menu for the bare commit
/// would leave out merging, deleting and opening a pull request. So the row
/// stands for its most useful ref: another local branch beats the checked-out
/// one, local beats remote, and any branch beats a tag. Ties go to the ref
/// listed first.
pub fn aim_for_row(refs: &[RefBadge], checked_out: &str) -> Aim {
    let rank = |r: &RefBadge| match r.kind {
        RefKind::Branch if r.label != checked_out => 0,
        RefKind::Branch => 1,
        RefKind::Remote => 2,
        RefKind::Tag => 3,
    };
    refs.iter()
        .min_by_key(|r| rank(r))
        .map_or(Aim::Commit, |r| Aim::from_badge(&r.label, r.kind))
}

/// A first-parent line with no merges, from the clicked commit's parent up to
/// HEAD. Reordering, squashing and dropping replay exactly these commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearRun {
    /// What the replay starts from: the parent's parent, or `None` when the
    /// parent is a root commit.
    pub base: Option<String>,
    /// Oldest first: the parent, the clicked commit, then on up to HEAD.
    pub commits: Vec<String>,
}

/// Everything the menu depends on.
#[derive(Debug, Clone)]
pub struct Scene {
    pub root: String,
    pub sha: String,
    /// The checked-out branch; `None` while HEAD is detached.
    pub branch: Option<String>,
    pub aim: Aim,
    pub forge: Option<Hosted>,
    pub remote: Option<String>,
    /// Only HEAD can be reworded or amended.
    pub is_head: bool,
    /// The full message, to start a reword from.
    pub message: Option<String>,
    pub run: Option<LinearRun>,
}

impl Scene {
    fn short(&self) -> String {
        abbrev(&self.sha)
    }

    /// The checked-out branch as labels name it.
    fn here(&self) -> &str {
        self.branch.as_deref().unwrap_or("HEAD")
    }
}

/// A sha cut to the seven characters labels show.
pub fn abbrev(sha: &str) -> String {
    sha.chars().take(7).collect()
}

// ── What an item is ─────────────────────────────────────────────────────────

/// Everything an item can do, with the values it does it with.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    FastForward {
        to: String,
    },
    Merge {
        from: String,
    },
    Rebase {
        onto: String,
    },
    Switch {
        branch: String,
    },
    /// Check out a remote branch through a local one that tracks it.
    SwitchTracking {
        remote_branch: String,
        name: String,
    },
    Detach {
        at: String,
    },
    BranchFrom {
        start: String,
    },
    BranchAt {
        sha: String,
    },
    Rename {
        branch: String,
    },
    Worktree {
        at: String,
        kind: WorktreeKind,
    },
    Push {
        branch: String,
        remote: String,
    },
    /// With a lease: refused if the remote moved since the last fetch.
    ForcePush {
        branch: String,
        remote: String,
    },
    Track {
        branch: String,
        remote: String,
    },
    CherryPick {
        sha: String,
    },
    Revert {
        sha: String,
    },
    Reset {
        sha: String,
        mode: ResetMode,
    },
    Reword,
    Amend,
    Reshape {
        kind: Reshape,
        base: String,
        steps: Vec<RebaseStep>,
    },
    Browse {
        url: String,
    },
    DeleteBranch {
        branch: String,
    },
    DeleteRemoteBranch {
        remote: String,
        branch: String,
    },
    Tag {
        sha: String,
        annotated: bool,
    },
    Annotate {
        tag: String,
        sha: String,
    },
    DeleteTag {
        tag: String,
    },
    PushTag {
        tag: String,
        remote: String,
    },
    DeleteRemoteTag {
        tag: String,
        remote: String,
    },
    Copy {
        text: String,
    },
    Patch {
        sha: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeKind {
    /// On the branch itself.
    Existing,
    /// On a new local branch tracking the remote one.
    Tracking {
        name: String,
    },
    /// On a new branch the form names.
    Fresh,
    Detached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reshape {
    IntoParent,
    Earlier,
    Later,
    Remove,
}

/// What a pick comes to once its form is filled.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Git(Box<GitAction>),
    Clipboard(String),
    Browse(String),
}

/// The inputs a form can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Name,
    NewBranch,
    Message,
    Path,
    Folder,
}

pub type Answers = HashMap<Key, String>;

/// One input of a form.
#[derive(Debug, Clone)]
pub struct Field {
    pub key: Key,
    pub caption: &'static str,
    pub hint: &'static str,
    pub initial: Option<String>,
    pub multiline: bool,
}

#[derive(Debug, Clone)]
pub struct Form {
    pub heading: String,
    pub note: Option<String>,
    pub button: &'static str,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone)]
pub struct Question {
    pub heading: String,
    pub body: String,
    pub button: &'static str,
    /// Paints the button and the menu row as dangerous.
    pub danger: bool,
}

/// What stands between a pick and its run.
#[derive(Debug, Clone)]
pub enum Gate {
    Open,
    Confirm(Question),
    Form(Form),
}

/// A row of the menu that does something.
#[derive(Debug, Clone)]
pub struct Item {
    pub label: String,
    pub command: Command,
    pub gate: Gate,
    /// The toast once it worked. Anything that changes the repository has
    /// one: a graph that barely moves is easy to miss.
    pub receipt: Option<String>,
}

impl Item {
    fn new(label: impl Into<String>, command: Command) -> Self {
        Self {
            label: label.into(),
            command,
            gate: Gate::Open,
            receipt: None,
        }
    }

    fn done(mut self, receipt: impl Into<String>) -> Self {
        self.receipt = Some(receipt.into());
        self
    }

    fn confirm(mut self, heading: String, body: impl Into<String>, button: &'static str) -> Self {
        self.gate = Gate::Confirm(Question {
            heading,
            body: body.into(),
            button,
            danger: false,
        });
        self
    }

    fn dangerous(mut self, heading: String, body: impl Into<String>, button: &'static str) -> Self {
        self.gate = Gate::Confirm(Question {
            heading,
            body: body.into(),
            button,
            danger: true,
        });
        self
    }

    fn form(mut self, form: Form) -> Self {
        self.gate = Gate::Form(form);
        self
    }

    pub fn danger(&self) -> bool {
        matches!(&self.gate, Gate::Confirm(q) if q.danger)
    }

    /// The outcome of running this item with `answers`, or why it cannot.
    pub fn outcome(&self, answers: &Answers) -> Result<Outcome, String> {
        self.command.outcome(answers)
    }
}

/// A top-level row: an item, or a folder of items shown beside it.
#[derive(Debug, Clone)]
pub enum Entry {
    Item(Item),
    Folder { label: String, items: Vec<Item> },
}

/// Rows drawn together; a rule separates groups.
pub type Group = Vec<Entry>;

impl Command {
    fn outcome(&self, answers: &Answers) -> Result<Outcome, String> {
        let git = |action: GitAction| Ok(Outcome::Git(Box::new(action)));
        match self.clone() {
            Command::FastForward { to } => git(GitAction::Merge {
                reference: to,
                ff_only: true,
            }),
            Command::Merge { from } => git(GitAction::Merge {
                reference: from,
                ff_only: false,
            }),
            Command::Rebase { onto } => git(GitAction::Rebase { onto }),
            Command::Switch { branch } => git(GitAction::Checkout { name: branch }),
            Command::SwitchTracking {
                remote_branch,
                name,
            } => git(GitAction::CheckoutTracking {
                local: name,
                reference: remote_branch,
            }),
            Command::Detach { at } => git(GitAction::CheckoutDetached { sha: at }),
            Command::BranchFrom { start } => git(GitAction::CheckoutNewBranch {
                name: need(answers, Key::Name, "A branch name")?,
                start,
            }),
            Command::BranchAt { sha } => git(GitAction::CreateBranch {
                name: need(answers, Key::Name, "A branch name")?,
                sha,
            }),
            Command::Rename { branch } => git(GitAction::RenameBranch {
                old: branch,
                new: need(answers, Key::Name, "A branch name")?,
            }),
            Command::Worktree { at, kind } => {
                let path = need(answers, Key::Path, "A folder for the worktree")?;
                let (new_branch, track, detach) = match kind {
                    WorktreeKind::Existing => (None, false, false),
                    WorktreeKind::Tracking { name } => (Some(name), true, false),
                    WorktreeKind::Fresh => (
                        Some(need(answers, Key::NewBranch, "A branch name")?),
                        false,
                        false,
                    ),
                    WorktreeKind::Detached => (None, false, true),
                };
                git(GitAction::AddWorktree {
                    path,
                    reference: at,
                    new_branch,
                    track,
                    detach,
                })
            }
            Command::Push { branch, remote } => git(GitAction::PushBranch { branch, remote }),
            Command::ForcePush { branch, remote } => git(GitAction::PushForce { branch, remote }),
            Command::Track { branch, remote } => git(GitAction::SetUpstream { branch, remote }),
            Command::CherryPick { sha } => git(GitAction::CherryPick { sha }),
            Command::Revert { sha } => git(GitAction::Revert { sha }),
            Command::Reset { sha, mode } => git(GitAction::Reset { sha, mode }),
            Command::Reword | Command::Amend => git(GitAction::Commit {
                message: need(answers, Key::Message, "A message")?,
                amend: true,
            }),
            Command::Reshape { base, steps, .. } => {
                git(GitAction::RebaseInteractive { base, steps })
            }
            Command::Browse { url } => Ok(Outcome::Browse(url)),
            Command::DeleteBranch { branch } => git(GitAction::DeleteBranch {
                name: branch,
                force: false,
            }),
            Command::DeleteRemoteBranch { remote, branch } => {
                git(GitAction::DeleteRemoteBranch { remote, branch })
            }
            Command::Tag { sha, annotated } => git(GitAction::CreateTag {
                name: need(answers, Key::Name, "A tag name")?,
                sha,
                message: annotated.then(|| given(answers, Key::Message)).flatten(),
            }),
            Command::Annotate { tag, sha } => git(GitAction::AnnotateTag {
                name: tag,
                sha,
                message: need(answers, Key::Message, "A message")?,
            }),
            Command::DeleteTag { tag } => git(GitAction::DeleteTag { name: tag }),
            Command::PushTag { tag, remote } => git(GitAction::PushTag { name: tag, remote }),
            Command::DeleteRemoteTag { tag, remote } => {
                git(GitAction::DeleteRemoteTag { name: tag, remote })
            }
            Command::Copy { text } => Ok(Outcome::Clipboard(text)),
            Command::Patch { sha } => git(GitAction::FormatPatch {
                sha,
                dir: need(answers, Key::Folder, "A folder")?,
            }),
        }
    }
}

/// An answer, trimmed; blank counts as not given.
fn given(answers: &Answers, key: Key) -> Option<String> {
    let text = answers.get(&key)?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// An answer the command cannot do without.
fn need(answers: &Answers, key: Key, what: &str) -> Result<String, String> {
    given(answers, key).ok_or_else(|| format!("{what} is needed"))
}

// ── Forms ───────────────────────────────────────────────────────────────────

fn input(key: Key, caption: &'static str, hint: &'static str) -> Field {
    Field {
        key,
        caption,
        hint,
        initial: None,
        multiline: false,
    }
}

fn branch_input() -> Field {
    input(Key::Name, "Branch", "feature/short-name")
}

fn message_input(initial: Option<String>) -> Field {
    Field {
        initial,
        multiline: true,
        ..input(Key::Message, "Message", "What changed and why")
    }
}

fn worktree_form(scene: &Scene, at: &str) -> Form {
    Form {
        heading: format!("New worktree at {at}"),
        note: Some(format!(
            "Another checkout of this repository, starting at {at}. Both share one history."
        )),
        button: "Add worktree",
        fields: vec![Field {
            initial: Some(sibling_folder(&scene.root, at)),
            ..input(Key::Path, "Folder", "")
        }],
    }
}

/// A folder beside the repository for a worktree of `at`: the repository's
/// name, a dash, the ref. Inside the repository it would show up as an
/// untracked folder in every status.
pub fn sibling_folder(root: &str, at: &str) -> String {
    let slash = if root.contains('\\') { '\\' } else { '/' };
    let trimmed = root.trim_end_matches(['/', '\\']);
    let (parent, repo) = trimmed.rsplit_once(['/', '\\']).unwrap_or(("", trimmed));
    let repo = if repo.is_empty() { "repo" } else { repo };
    let mut leaf = path_segment(at);
    if leaf.is_empty() {
        leaf = "worktree".to_owned();
    }
    format!("{parent}{slash}{repo}-{leaf}")
}

/// A ref name as a single safe path segment: the `refs/...` namespace goes,
/// every run of other characters becomes one dash, and it stops at 60
/// characters.
pub fn path_segment(reference: &str) -> String {
    let name = reference
        .strip_prefix("refs/heads/")
        .or_else(|| reference.strip_prefix("refs/remotes/"))
        .or_else(|| reference.strip_prefix("refs/tags/"))
        .unwrap_or(reference);
    let mut out = String::new();
    let mut gap = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    let cut: String = out.chars().take(60).collect();
    cut.trim_matches(['-', '.']).to_owned()
}

// ── The menu ────────────────────────────────────────────────────────────────

/// The menu for `scene`, top to bottom. Empty groups are left out.
pub fn menu_for(scene: &Scene) -> Vec<Group> {
    let branch = match &scene.aim {
        Aim::Branch(branch) => Some(branch),
        _ => None,
    };
    let tag = match &scene.aim {
        Aim::Tag(tag) => Some(tag.as_str()),
        _ => None,
    };
    let checked_out = branch.is_some_and(|b| scene.branch.as_deref() == Some(&b.badge));

    let mut groups = vec![
        switching(scene, branch, tag),
        integrating(scene, branch, tag, checked_out),
        creating(scene),
        applying(scene),
        rewriting(scene),
        remote(scene, branch, tag),
        sharing(scene, branch, tag),
        deleting(scene, branch, tag, checked_out),
    ];
    groups.retain(|group| !group.is_empty());
    groups
}

fn item(item: Item) -> Entry {
    Entry::Item(item)
}

fn detach_item(scene: &Scene, label: String) -> Item {
    let short = scene.short();
    Item::new(
        label,
        Command::Detach {
            at: scene.sha.clone(),
        },
    )
    .confirm(
        format!("Check out {short}?"),
        "HEAD will point at the commit itself, on no branch.",
        "Check out",
    )
    .done(format!("Now at {short}, detached"))
}

fn detached_worktree(scene: &Scene, label: String) -> Item {
    Item::new(
        label,
        Command::Worktree {
            at: scene.sha.clone(),
            kind: WorktreeKind::Detached,
        },
    )
    .form(worktree_form(scene, &scene.short()))
    .done("Worktree added")
}

/// Checking out, and worktrees: other places to stand.
fn switching(scene: &Scene, branch: Option<&BranchAim>, tag: Option<&str>) -> Group {
    let short = scene.short();
    if let Some(tag) = tag {
        return vec![item(
            Item::new(
                format!("Check out {tag}"),
                Command::Detach {
                    at: format!("refs/tags/{tag}"),
                },
            )
            .confirm(
                format!("Check out {tag}?"),
                "Tags are not branches, so HEAD will be detached.",
                "Check out",
            )
            .done(format!("Now at {tag}, detached")),
        )];
    }
    let Some(branch) = branch else {
        return vec![
            item(detach_item(scene, "Check out this commit".into())),
            item(detached_worktree(scene, "New worktree here".into())),
        ];
    };

    let badge = &branch.badge;
    let mut checkout = Vec::new();
    match &branch.remote {
        Some(_) => checkout.push(
            Item::new(
                format!("As local {} tracking {badge}", branch.name),
                Command::SwitchTracking {
                    remote_branch: badge.clone(),
                    name: branch.name.clone(),
                },
            )
            .done(format!("On {}", branch.name)),
        ),
        None if scene.branch.as_deref() != Some(badge) => checkout.push(
            Item::new(
                badge.clone(),
                Command::Switch {
                    branch: badge.clone(),
                },
            )
            .done(format!("On {badge}")),
        ),
        None => {}
    }
    checkout.push(
        Item::new(
            format!("As a new branch from {badge}"),
            Command::BranchFrom {
                start: badge.clone(),
            },
        )
        .form(Form {
            heading: format!("New branch from {badge}"),
            note: None,
            button: "Create and check out",
            fields: vec![branch_input()],
        })
        .done("On the new branch"),
    );
    checkout.push(detach_item(
        scene,
        format!("Commit {short} only (detached)"),
    ));

    let worktrees = vec![
        Item::new(
            match &branch.remote {
                Some(_) => format!("New local {} tracking it", branch.name),
                None => format!("On {badge}"),
            },
            Command::Worktree {
                at: badge.clone(),
                kind: match &branch.remote {
                    Some(_) => WorktreeKind::Tracking {
                        name: branch.name.clone(),
                    },
                    None => WorktreeKind::Existing,
                },
            },
        )
        .form(worktree_form(scene, badge))
        .done("Worktree added"),
        Item::new(
            "On a new branch",
            Command::Worktree {
                at: badge.clone(),
                kind: WorktreeKind::Fresh,
            },
        )
        .form({
            let mut form = worktree_form(scene, badge);
            form.fields
                .push(input(Key::NewBranch, "New branch", "feature/short-name"));
            form
        })
        .done("Worktree added"),
        detached_worktree(scene, format!("At {short}, detached")),
    ];

    let mut group = vec![
        Entry::Folder {
            label: "Check out".into(),
            items: checkout,
        },
        Entry::Folder {
            label: "New worktree".into(),
            items: worktrees,
        },
    ];
    if branch.remote.is_none() {
        group.push(item(
            Item::new(
                format!("Rename {badge}…"),
                Command::Rename {
                    branch: badge.clone(),
                },
            )
            .form(Form {
                heading: format!("Rename {badge}"),
                note: None,
                button: "Rename",
                fields: vec![Field {
                    initial: Some(badge.clone()),
                    ..branch_input()
                }],
            })
            .done("Branch renamed"),
        ));
    }
    group
}

/// Bringing the aim into the checked-out branch.
fn integrating(
    scene: &Scene,
    branch: Option<&BranchAim>,
    tag: Option<&str>,
    checked_out: bool,
) -> Group {
    if checked_out {
        return Vec::new();
    }
    let here = scene.here();
    // Git takes a sha as readily as a name, but there is nothing to
    // fast-forward "to" without a name.
    let (what, shown, named) = match (branch, tag) {
        (Some(b), _) => (b.badge.clone(), b.badge.clone(), true),
        (None, Some(t)) => (t.to_owned(), t.to_owned(), true),
        (None, None) => (scene.sha.clone(), scene.short(), false),
    };
    let mut group = Vec::new();
    if named {
        group.push(item(
            Item::new(
                format!("Fast-forward {here} up to {shown}"),
                Command::FastForward { to: what.clone() },
            )
            .done(format!("{here} is now at {shown}")),
        ));
    }
    group.push(item(
        Item::new(
            format!("Merge {shown} into {here}"),
            Command::Merge { from: what.clone() },
        )
        .confirm(
            format!("Merge {shown}?"),
            format!("A merge brings {shown} and everything behind it into {here}."),
            "Merge",
        )
        .done(format!("{shown} merged into {here}")),
    ));
    group.push(item(
        Item::new(
            format!("Rebase {here} on {shown}"),
            Command::Rebase { onto: what },
        )
        .confirm(
            format!("Rebase {here}?"),
            format!("Commits only {here} has are replayed after {shown}, as new commits."),
            "Rebase",
        )
        .done(format!("{here} rebased on {shown}")),
    ));
    group
}

/// New refs at the commit.
fn creating(scene: &Scene) -> Group {
    let short = scene.short();
    let tag_input = || input(Key::Name, "Tag", "v1.2.3");
    vec![
        item(
            Item::new(
                "New branch here",
                Command::BranchAt {
                    sha: scene.sha.clone(),
                },
            )
            .form(Form {
                heading: format!("New branch at {short}"),
                note: None,
                button: "Create",
                fields: vec![branch_input()],
            })
            .done("Branch created"),
        ),
        item(
            Item::new(
                "New tag here",
                Command::Tag {
                    sha: scene.sha.clone(),
                    annotated: false,
                },
            )
            .form(Form {
                heading: format!("New tag at {short}"),
                note: None,
                button: "Create",
                fields: vec![tag_input()],
            })
            .done("Tag created"),
        ),
        item(
            Item::new(
                "New annotated tag here",
                Command::Tag {
                    sha: scene.sha.clone(),
                    annotated: true,
                },
            )
            .form(Form {
                heading: format!("New annotated tag at {short}"),
                note: None,
                button: "Create",
                fields: vec![tag_input(), message_input(None)],
            })
            .done("Tag created"),
        ),
    ]
}

/// Changing the checked-out branch with this commit.
fn applying(scene: &Scene) -> Group {
    let (here, short, sha) = (scene.here(), scene.short(), &scene.sha);
    let reset = |label: &str, mode| {
        Item::new(
            label,
            Command::Reset {
                sha: sha.clone(),
                mode,
            },
        )
        .done(format!("{here} moved to {short}"))
    };
    vec![
        item(
            Item::new("Cherry-pick", Command::CherryPick { sha: sha.clone() })
                .confirm(
                    format!("Cherry-pick {short}?"),
                    format!("Its changes are committed again on top of {here}."),
                    "Cherry-pick",
                )
                .done(format!("{short} picked onto {here}")),
        ),
        item(
            Item::new("Revert", Command::Revert { sha: sha.clone() })
                .confirm(
                    format!("Revert {short}?"),
                    format!("A new commit on {here} undoes what {short} did."),
                    "Revert",
                )
                .done(format!("{short} reverted")),
        ),
        Entry::Folder {
            label: format!("Move {here} here"),
            items: vec![
                reset("Keep the changes, staged", ResetMode::Soft),
                reset("Keep the changes, unstaged", ResetMode::Mixed),
                reset("Throw the changes away", ResetMode::Hard).dangerous(
                    format!("Hard reset {here} to {short}?"),
                    "Every later commit and every uncommitted change is lost, with no undo.",
                    "Reset",
                ),
            ],
        },
    ]
}

/// Editing commits already made.
fn rewriting(scene: &Scene) -> Group {
    let short = scene.short();
    let mut group = Vec::new();
    if scene.is_head {
        let message_form = |heading: String, note: &str, button| Form {
            heading,
            note: Some(note.to_owned()),
            button,
            fields: vec![message_input(scene.message.clone())],
        };
        group.push(item(
            Item::new("Edit message", Command::Reword)
                .form(message_form(
                    format!("New message for {short}"),
                    "The files stay as they were committed.",
                    "Save",
                ))
                .done(format!("{short} has a new message")),
        ));
        group.push(item(
            Item::new("Add staged changes to this commit", Command::Amend)
                .form(message_form(
                    format!("Amend {short}"),
                    "The staged changes become part of it.",
                    "Amend",
                ))
                .done(format!("{short} amended")),
        ));
    }
    let Some(run) = scene.run.as_ref().filter(|run| run.commits.len() >= 2) else {
        return group;
    };
    let base = run.base.clone().unwrap_or_default();
    let reshape = |kind: Reshape| Command::Reshape {
        kind,
        base: base.clone(),
        steps: reshaped(&run.commits, kind),
    };
    let warning = "Every commit from here up to HEAD is rewritten.";
    group.push(item(
        Item::new("Squash into parent", reshape(Reshape::IntoParent))
            .confirm(
                format!("Squash {short} into its parent?"),
                format!("Its changes join the parent's, under the parent's message. {warning}"),
                "Squash",
            )
            .done(format!("{short} squashed")),
    ));
    group.push(item(
        Item::new("Swap with parent", reshape(Reshape::Earlier))
            .done(format!("{short} moved one down")),
    ));
    if run.commits.len() >= 3 {
        group.push(item(
            Item::new("Swap with child", reshape(Reshape::Later))
                .done(format!("{short} moved one up")),
        ));
    }
    group.push(item(
        Item::new("Remove from history", reshape(Reshape::Remove))
            .dangerous(
                format!("Remove {short}?"),
                format!("Its changes leave the branch. {warning}"),
                "Remove",
            )
            .done(format!("{short} removed")),
    ));
    group
}

/// The rebase steps for `kind`, from `commits` oldest first. The clicked
/// commit is the second one.
fn reshaped(commits: &[String], kind: Reshape) -> Vec<RebaseStep> {
    let mut steps: Vec<RebaseStep> = commits
        .iter()
        .map(|sha| RebaseStep {
            op: RebaseOp::Pick,
            sha: sha.clone(),
        })
        .collect();
    match kind {
        Reshape::IntoParent => steps[1].op = RebaseOp::Fixup,
        Reshape::Earlier => steps.swap(0, 1),
        Reshape::Later => steps.swap(1, 2),
        Reshape::Remove => {
            steps.remove(1);
        }
    }
    steps
}

/// Pushing, and the forge's pages.
fn remote(scene: &Scene, branch: Option<&BranchAim>, tag: Option<&str>) -> Group {
    let mut group = Vec::new();
    if let (Some(remote), Some(branch)) = (&scene.remote, branch)
        && branch.remote.is_none()
    {
        let name = branch.badge.clone();
        group.push(item(
            Item::new(
                format!("Push {name} to {remote}"),
                Command::Push {
                    branch: name.clone(),
                    remote: remote.clone(),
                },
            )
            .done(format!("{name} is on {remote}")),
        ));
        group.push(item(
            Item::new(
                format!("Force push {name} to {remote}"),
                Command::ForcePush {
                    branch: name.clone(),
                    remote: remote.clone(),
                },
            )
            .dangerous(
                format!("Force push {name}?"),
                format!(
                    "{remote}/{name} becomes your {name}, and commits only it had are gone. \
                     If someone pushed since your last fetch, git refuses."
                ),
                "Force push",
            )
            .done(format!("{name} force pushed to {remote}")),
        ));
        group.push(item(
            Item::new(
                format!("Track {remote}/{name}"),
                Command::Track {
                    branch: name.clone(),
                    remote: remote.clone(),
                },
            )
            .done(format!("{name} now tracks {remote}/{name}")),
        ));
    }
    if let (Some(remote), Some(tag)) = (&scene.remote, tag) {
        group.push(item(
            Item::new(
                format!("Push {tag} to {remote}"),
                Command::PushTag {
                    tag: tag.to_owned(),
                    remote: remote.clone(),
                },
            )
            .done(format!("{tag} is on {remote}")),
        ));
    }
    if let (Some(forge), Some(branch)) = (&scene.forge, branch) {
        let name = branch.forge_name();
        group.push(item(Item::new(
            format!("Open a pull request for {name}"),
            Command::Browse {
                url: forge.page(Page::NewPullRequest(name)),
            },
        )));
        group.push(item(Item::new(
            format!("Show {name} on {}", forge.forge.name()),
            Command::Browse {
                url: forge.page(Page::Branch(name)),
            },
        )));
    }
    group
}

/// Taking something out of the app: text, links, a patch, a comparison.
fn sharing(scene: &Scene, branch: Option<&BranchAim>, tag: Option<&str>) -> Group {
    let copy = |label: String, text: String, receipt: &str| {
        item(Item::new(label, Command::Copy { text }).done(receipt))
    };
    let mut group = vec![copy("Copy sha".into(), scene.sha.clone(), "Sha copied")];
    if let Some(branch) = branch {
        group.push(copy(
            "Copy branch name".into(),
            branch.badge.clone(),
            "Name copied",
        ));
    }
    if let Some(tag) = tag {
        group.push(copy("Copy tag name".into(), tag.to_owned(), "Name copied"));
    }
    if let Some(forge) = &scene.forge {
        group.push(copy(
            "Copy commit link".into(),
            forge.page(Page::Commit(&scene.sha)),
            "Link copied",
        ));
        if let Some(branch) = branch {
            group.push(copy(
                format!("Copy link to {}", branch.badge),
                forge.page(Page::Branch(branch.forge_name())),
                "Link copied",
            ));
        }
        if let Some(tag) = tag {
            group.push(copy(
                format!("Copy link to {tag}"),
                forge.page(Page::Tag(tag)),
                "Link copied",
            ));
        }
    }
    group.push(item(
        Item::new(
            "Save as patch…",
            Command::Patch {
                sha: scene.sha.clone(),
            },
        )
        .form(Form {
            heading: format!("Save {} as a patch", scene.short()),
            note: Some("A file `git am` can apply in any repository.".into()),
            button: "Save",
            fields: vec![Field {
                initial: Some(scene.root.clone()),
                ..input(Key::Folder, "Folder", "")
            }],
        })
        .done("Patch saved"),
    ));
    if let Some(tag) = tag {
        group.push(item(
            Item::new(
                format!("Add a message to {tag}"),
                Command::Annotate {
                    tag: tag.to_owned(),
                    sha: scene.sha.clone(),
                },
            )
            .form(Form {
                heading: format!("Annotate {tag}"),
                note: Some("The tag is recreated on the same commit, with a message.".into()),
                button: "Save",
                fields: vec![message_input(None)],
            })
            .done(format!("{tag} annotated")),
        ));
    }
    group
}

/// Removing refs, here or on the remote. Last, away from the rest.
fn deleting(
    scene: &Scene,
    branch: Option<&BranchAim>,
    tag: Option<&str>,
    checked_out: bool,
) -> Group {
    let mut group = Vec::new();
    match branch {
        Some(_) if checked_out => {}
        Some(BranchAim {
            badge,
            name,
            remote: Some(remote),
        }) => group.push(item(
            Item::new(
                format!("Delete {badge}"),
                Command::DeleteRemoteBranch {
                    remote: remote.clone(),
                    branch: name.clone(),
                },
            )
            .dangerous(
                format!("Delete {name} from {remote}?"),
                format!("It goes from {remote} itself, for everyone who uses it."),
                "Delete",
            )
            .done(format!("{name} deleted from {remote}")),
        )),
        Some(BranchAim { badge, .. }) => group.push(item(
            Item::new(
                format!("Delete {badge}"),
                Command::DeleteBranch {
                    branch: badge.clone(),
                },
            )
            .dangerous(
                format!("Delete {badge}?"),
                "Git refuses if the branch has commits not merged anywhere else.",
                "Delete",
            )
            .done(format!("{badge} deleted")),
        )),
        None => {}
    }
    if let Some(tag) = tag {
        group.push(item(
            Item::new(
                format!("Delete {tag}"),
                Command::DeleteTag {
                    tag: tag.to_owned(),
                },
            )
            .dangerous(
                format!("Delete {tag}?"),
                "Only this copy goes; remotes keep theirs.",
                "Delete",
            )
            .done(format!("{tag} deleted")),
        ));
        if let Some(remote) = &scene.remote {
            group.push(item(
                Item::new(
                    format!("Delete {tag} from {remote}"),
                    Command::DeleteRemoteTag {
                        tag: tag.to_owned(),
                        remote: remote.clone(),
                    },
                )
                .dangerous(
                    format!("Delete {tag} from {remote}?"),
                    format!("Nobody fetching from {remote} gets it any more."),
                    "Delete",
                )
                .done(format!("{tag} deleted from {remote}")),
            ));
        }
    }
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef";

    fn scene(aim: Aim) -> Scene {
        Scene {
            root: "/code/shop".into(),
            sha: SHA.into(),
            branch: Some("trunk".into()),
            aim,
            forge: crate::git::forge::hosted("https://github.com/acme/shop"),
            remote: Some("origin".into()),
            is_head: false,
            message: None,
            run: None,
        }
    }

    fn branch(label: &str) -> Aim {
        Aim::from_badge(label, RefKind::Branch)
    }

    fn remote_branch(label: &str) -> Aim {
        Aim::from_badge(label, RefKind::Remote)
    }

    fn tag(label: &str) -> Aim {
        Aim::from_badge(label, RefKind::Tag)
    }

    /// Every item, folders opened, in menu order.
    fn items(menu: &[Group]) -> Vec<&Item> {
        menu.iter()
            .flatten()
            .flat_map(|entry| match entry {
                Entry::Item(item) => vec![item],
                Entry::Folder { items, .. } => items.iter().collect(),
            })
            .collect()
    }

    fn find(menu: &[Group], wanted: impl Fn(&Command) -> bool) -> Option<&Item> {
        items(menu).into_iter().find(|item| wanted(&item.command))
    }

    fn has(menu: &[Group], wanted: impl Fn(&Command) -> bool) -> bool {
        find(menu, wanted).is_some()
    }

    fn answers(pairs: &[(Key, &str)]) -> Answers {
        pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    fn git(action: GitAction) -> Result<Outcome, String> {
        Ok(Outcome::Git(Box::new(action)))
    }

    #[test]
    fn a_plain_commit_offers_no_ref_items() {
        let menu = menu_for(&scene(Aim::Commit));
        assert!(has(
            &menu,
            |c| matches!(c, Command::Merge { from } if from == SHA)
        ));
        assert!(!has(&menu, |c| matches!(c, Command::FastForward { .. })));
        assert!(!has(&menu, |c| matches!(
            c,
            Command::Push { .. } | Command::DeleteBranch { .. } | Command::DeleteTag { .. }
        )));
        assert!(!has(&menu, |c| matches!(c, Command::Browse { .. })));
        let merge = find(&menu, |c| matches!(c, Command::Merge { .. })).unwrap();
        assert_eq!(merge.label, "Merge 0123456 into trunk");
    }

    #[test]
    fn detached_head_reads_as_head() {
        let mut detached = scene(branch("topic"));
        detached.branch = None;
        let menu = menu_for(&detached);
        let merge = find(&menu, |c| matches!(c, Command::Merge { .. })).unwrap();
        assert_eq!(merge.label, "Merge topic into HEAD");
    }

    #[test]
    fn a_row_stands_for_its_most_useful_ref() {
        use RefKind::*;
        let r = |label: &str, kind| RefBadge {
            label: label.into(),
            kind,
        };
        assert_eq!(aim_for_row(&[], "trunk"), Aim::Commit);
        assert_eq!(
            aim_for_row(
                &[r("v2", Tag), r("trunk", Branch), r("topic", Branch)],
                "trunk"
            ),
            branch("topic")
        );
        assert_eq!(aim_for_row(&[r("trunk", Branch)], "trunk"), branch("trunk"));
        assert_eq!(
            aim_for_row(&[r("v2", Tag), r("origin/x", Remote)], "trunk"),
            remote_branch("origin/x")
        );
        assert_eq!(aim_for_row(&[r("v2", Tag)], "trunk"), tag("v2"));
    }

    #[test]
    fn the_checked_out_branch_neither_merges_nor_deletes_itself() {
        let menu = menu_for(&scene(branch("trunk")));
        assert!(!has(&menu, |c| matches!(c, Command::Merge { .. })));
        assert!(!has(&menu, |c| matches!(c, Command::DeleteBranch { .. })));
        assert!(!has(&menu, |c| matches!(c, Command::Switch { .. })));
        assert!(has(
            &menu,
            |c| matches!(c, Command::Copy { text } if text == "trunk")
        ));
        assert!(has(
            &menu,
            |c| matches!(c, Command::Push { branch, .. } if branch == "trunk")
        ));
    }

    #[test]
    fn another_branch_merges_and_deletes_behind_a_question() {
        let menu = menu_for(&scene(branch("topic")));
        let merge = find(&menu, |c| matches!(c, Command::Merge { .. })).unwrap();
        assert!(matches!(merge.gate, Gate::Confirm(_)) && !merge.danger());
        let rename = find(&menu, |c| matches!(c, Command::Rename { .. })).unwrap();
        assert_eq!(
            rename.outcome(&answers(&[(Key::Name, "topic-2")])),
            git(GitAction::RenameBranch {
                old: "topic".into(),
                new: "topic-2".into()
            })
        );
        let delete = find(&menu, |c| matches!(c, Command::DeleteBranch { .. })).unwrap();
        assert!(delete.danger());
        assert_eq!(
            delete.outcome(&Answers::new()),
            git(GitAction::DeleteBranch {
                name: "topic".into(),
                force: false
            })
        );
    }

    #[test]
    fn a_remote_branch_checks_out_tracked_and_deletes_remotely() {
        let menu = menu_for(&scene(remote_branch("origin/feat/a")));
        let checkout = find(&menu, |c| matches!(c, Command::SwitchTracking { .. })).unwrap();
        assert_eq!(
            checkout.outcome(&Answers::new()),
            git(GitAction::CheckoutTracking {
                local: "feat/a".into(),
                reference: "origin/feat/a".into()
            })
        );
        assert!(!has(&menu, |c| matches!(
            c,
            Command::Push { .. } | Command::Rename { .. } | Command::DeleteBranch { .. }
        )));
        let delete = find(&menu, |c| matches!(c, Command::DeleteRemoteBranch { .. })).unwrap();
        assert_eq!(
            delete.outcome(&Answers::new()),
            git(GitAction::DeleteRemoteBranch {
                remote: "origin".into(),
                branch: "feat/a".into()
            })
        );
        let pr = find(
            &menu,
            |c| matches!(c, Command::Browse { url } if url.contains("compare")),
        );
        assert!(pr.is_some_and(|pr| pr.label.ends_with("feat/a")));
    }

    #[test]
    fn a_tag_checks_out_detached_and_reaches_the_remote() {
        let menu = menu_for(&scene(tag("v3")));
        let checkout = find(&menu, |c| matches!(c, Command::Detach { .. })).unwrap();
        assert_eq!(
            checkout.outcome(&Answers::new()),
            git(GitAction::CheckoutDetached {
                sha: "refs/tags/v3".into()
            })
        );
        assert!(has(
            &menu,
            |c| matches!(c, Command::FastForward { to } if to == "v3")
        ));
        assert!(has(&menu, |c| matches!(c, Command::PushTag { .. })));
        assert!(has(&menu, |c| matches!(c, Command::DeleteRemoteTag { .. })));
        let annotate = find(&menu, |c| matches!(c, Command::Annotate { .. })).unwrap();
        assert_eq!(
            annotate.outcome(&answers(&[(Key::Message, "Notes")])),
            git(GitAction::AnnotateTag {
                name: "v3".into(),
                sha: SHA.into(),
                message: "Notes".into()
            })
        );
    }

    #[test]
    fn no_remote_means_nothing_to_push() {
        let mut local = scene(branch("topic"));
        local.remote = None;
        local.forge = None;
        assert!(!has(&menu_for(&local), |c| matches!(
            c,
            Command::Push { .. }
        )));
        local.aim = tag("v1");
        let menu = menu_for(&local);
        assert!(!has(&menu, |c| matches!(
            c,
            Command::PushTag { .. } | Command::DeleteRemoteTag { .. }
        )));
    }

    #[test]
    fn links_need_a_known_forge() {
        let mut unknown = scene(branch("topic"));
        unknown.forge = None;
        let menu = menu_for(&unknown);
        assert!(!has(&menu, |c| matches!(c, Command::Browse { .. })));
        assert!(!has(
            &menu,
            |c| matches!(c, Command::Copy { text } if text.starts_with("https"))
        ));
        let sha = find(
            &menu,
            |c| matches!(c, Command::Copy { text } if text == SHA),
        )
        .unwrap();
        assert_eq!(
            sha.outcome(&Answers::new()),
            Ok(Outcome::Clipboard(SHA.into()))
        );
    }

    #[test]
    fn only_a_hard_reset_asks_first() {
        let menu = menu_for(&scene(Aim::Commit));
        let folder = menu
            .iter()
            .flatten()
            .find_map(|entry| match entry {
                Entry::Folder { label, items } if label.starts_with("Move") => Some(items),
                _ => None,
            })
            .expect("the reset folder");
        assert_eq!(folder.len(), 3);
        let asking: Vec<bool> = folder.iter().map(Item::danger).collect();
        assert_eq!(asking, [false, false, true]);
        assert_eq!(
            folder[2].outcome(&Answers::new()),
            git(GitAction::Reset {
                sha: SHA.into(),
                mode: ResetMode::Hard
            })
        );
    }

    #[test]
    fn a_form_needs_its_answers_trimmed_and_present() {
        let menu = menu_for(&scene(Aim::Commit));
        let new_branch = find(&menu, |c| matches!(c, Command::BranchAt { .. })).unwrap();
        assert_eq!(
            new_branch.outcome(&Answers::new()),
            Err("A branch name is needed".into())
        );
        assert_eq!(
            new_branch.outcome(&answers(&[(Key::Name, "  fix/a ")])),
            git(GitAction::CreateBranch {
                name: "fix/a".into(),
                sha: SHA.into()
            })
        );
        let annotated = find(&menu, |c| {
            matches!(
                c,
                Command::Tag {
                    annotated: true,
                    ..
                }
            )
        })
        .unwrap();
        assert_eq!(
            annotated.outcome(&answers(&[(Key::Name, "v4"), (Key::Message, " ")])),
            git(GitAction::CreateTag {
                name: "v4".into(),
                sha: SHA.into(),
                message: None
            })
        );
    }

    #[test]
    fn rewording_is_for_head_and_starts_from_its_message() {
        assert!(!has(&menu_for(&scene(Aim::Commit)), |c| *c == Command::Reword));
        let mut head = scene(Aim::Commit);
        head.is_head = true;
        head.message = Some("Old words".into());
        let menu = menu_for(&head);
        let reword = find(&menu, |c| *c == Command::Reword).unwrap();
        let Gate::Form(form) = &reword.gate else {
            panic!("reword has a form");
        };
        assert_eq!(form.fields[0].initial.as_deref(), Some("Old words"));
        assert!(form.fields[0].multiline);
        let amend = find(&menu, |c| *c == Command::Amend).unwrap();
        assert_eq!(
            amend.outcome(&answers(&[(Key::Message, "New words")])),
            git(GitAction::Commit {
                message: "New words".into(),
                amend: true
            })
        );
    }

    #[test]
    fn reshaping_replays_from_the_grandparent() {
        let mut s = scene(Aim::Commit);
        s.run = Some(LinearRun {
            base: Some("grand".into()),
            commits: vec!["parent".into(), SHA.into(), "head".into()],
        });
        let menu = menu_for(&s);
        let steps = |kind: Reshape| {
            let item = find(
                &menu,
                |c| matches!(c, Command::Reshape { kind: k, .. } if *k == kind),
            )
            .unwrap_or_else(|| panic!("{kind:?}"));
            match item.outcome(&Answers::new()) {
                Ok(Outcome::Git(action)) => match *action {
                    GitAction::RebaseInteractive { base, steps } => {
                        assert_eq!(base, "grand");
                        steps.into_iter().map(|s| (s.op, s.sha)).collect::<Vec<_>>()
                    }
                    other => panic!("{other:?}"),
                },
                other => panic!("{other:?}"),
            }
        };
        let pick = |sha: &str| (RebaseOp::Pick, sha.to_string());
        assert_eq!(
            steps(Reshape::IntoParent),
            [pick("parent"), (RebaseOp::Fixup, SHA.into()), pick("head")]
        );
        assert_eq!(
            steps(Reshape::Earlier),
            [pick(SHA), pick("parent"), pick("head")]
        );
        assert_eq!(
            steps(Reshape::Later),
            [pick("parent"), pick("head"), pick(SHA)]
        );
        assert_eq!(steps(Reshape::Remove), [pick("parent"), pick("head")]);

        // At HEAD there is no child to swap with.
        s.run = Some(LinearRun {
            base: None,
            commits: vec!["parent".into(), SHA.into()],
        });
        let menu = menu_for(&s);
        assert!(!has(&menu, |c| matches!(
            c,
            Command::Reshape {
                kind: Reshape::Later,
                ..
            }
        )));
        assert!(has(
            &menu,
            |c| matches!(c, Command::Reshape { base, .. } if base.is_empty())
        ));
        s.run = None;
        assert!(!has(&menu_for(&s), |c| matches!(
            c,
            Command::Reshape { .. }
        )));
    }

    #[test]
    fn worktrees_sit_beside_the_repository() {
        assert_eq!(sibling_folder("/code/shop", "fix/a"), "/code/shop-fix-a");
        assert_eq!(
            sibling_folder("/code/shop/", "origin/trunk"),
            "/code/shop-origin-trunk"
        );
        assert_eq!(sibling_folder("/shop", "!!!"), "/shop-worktree");
        assert_eq!(sibling_folder(r"D:\code\shop", "dev"), r"D:\code\shop-dev");
    }

    #[test]
    fn a_path_segment_is_clean() {
        assert_eq!(path_segment("refs/heads/fix/a"), "fix-a");
        assert_eq!(
            path_segment("ticket/XY-9 quick fix"),
            "ticket-XY-9-quick-fix"
        );
        assert_eq!(path_segment("__odd--"), "__odd");
        assert_eq!(path_segment("..x.."), "x");
        assert_eq!(path_segment(&"z".repeat(80)).len(), 60);
    }

    #[test]
    fn badges_split_into_remote_and_name() {
        assert_eq!(
            remote_branch("fork/fix/a"),
            Aim::Branch(BranchAim {
                badge: "fork/fix/a".into(),
                name: "fix/a".into(),
                remote: Some("fork".into()),
            })
        );
        assert_eq!(
            remote_branch("lonely"),
            Aim::Branch(BranchAim {
                badge: "lonely".into(),
                name: "lonely".into(),
                remote: None,
            })
        );
        assert_eq!(tag("v1"), Aim::Tag("v1".into()));
    }
}
