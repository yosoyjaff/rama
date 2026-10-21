//! Where a remote is hosted: which forge, and whose repository it is, read
//! from the URL git keeps for it. Author photos ask GitHub about it, and the
//! history's menus link into its web pages.
//!
//! Only GitHub, GitLab and Bitbucket are known. Any other host is treated
//! as unknown rather than guessed at: a guessed path lands on a 404 that
//! reads like a broken repository.

/// A forge Rama knows the web pages of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
    Bitbucket,
}

impl Forge {
    pub fn name(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::Bitbucket => "Bitbucket",
        }
    }

    fn serving(host: &str) -> Option<Self> {
        match host.trim_start_matches("www.") {
            "github.com" => Some(Self::GitHub),
            "gitlab.com" => Some(Self::GitLab),
            "bitbucket.org" => Some(Self::Bitbucket),
            _ => None,
        }
    }
}

/// A repository on a known forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hosted {
    pub forge: Forge,
    /// As the remote spells it, lowercased: `github.com`, `www.gitlab.com`.
    pub host: String,
    /// The account, or on GitLab the group path (`group/subgroup`).
    pub owner: String,
    pub repo: String,
}

/// A page of a hosted repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page<'a> {
    Commit(&'a str),
    Branch(&'a str),
    Tag(&'a str),
    /// The form for a new pull (or merge) request from this branch. The
    /// target is left for the forge to fill with the default branch.
    NewPullRequest(&'a str),
}

/// The hosted repository a remote URL names, in any of the forms git takes:
/// `https://`, `ssh://` and `git://` URLs, and the `[user@]host:path`
/// shorthand. `None` for a local path or an unknown host.
pub fn hosted(remote_url: &str) -> Option<Hosted> {
    let remote = remote_url.trim();
    let (authority, path) = match remote.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        None if remote.starts_with('/') || remote.starts_with('.') => return None,
        None => {
            let after_user = remote.rsplit_once('@').map_or(remote, |(_, rest)| rest);
            after_user.split_once(':')?
        }
    };
    // Users, passwords and ports never belong in a link.
    let host = authority.rsplit('@').next()?.split(':').next()?;
    let host = host.to_ascii_lowercase();
    let forge = Forge::serving(&host)?;

    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let segments: Vec<&str> = path.split('/').collect();
    let safe = |segment: &&str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    };
    // Only GitLab nests a repository under groups.
    let fits = match forge {
        Forge::GitLab => segments.len() >= 2,
        Forge::GitHub | Forge::Bitbucket => segments.len() == 2,
    };
    if !fits || !segments.iter().all(safe) {
        return None;
    }
    let (repo, owner) = segments.split_last()?;
    Some(Hosted {
        forge,
        host,
        owner: owner.join("/"),
        repo: (*repo).to_owned(),
    })
}

impl Hosted {
    /// The repository's home page.
    pub fn home(&self) -> String {
        format!("https://{}/{}/{}", self.host, self.owner, self.repo)
    }

    pub fn page(&self, page: Page) -> String {
        use Forge::*;
        let tail = match (self.forge, page) {
            (GitHub, Page::Commit(sha)) => format!("commit/{sha}"),
            (GitLab, Page::Commit(sha)) => format!("-/commit/{sha}"),
            (Bitbucket, Page::Commit(sha)) => format!("commits/{sha}"),
            // A nested branch keeps its slashes: all three address it so.
            (GitHub, Page::Branch(name)) => format!("tree/{}", escape_ref(name)),
            (GitLab, Page::Branch(name)) => format!("-/tree/{}", escape_ref(name)),
            (Bitbucket, Page::Branch(name)) => format!("src/{}", escape_ref(name)),
            // GitHub shows a tag as a release, GitLab in its list of tags.
            (GitHub, Page::Tag(tag)) => format!("releases/tag/{}", escape(tag)),
            (GitLab, Page::Tag(tag)) => format!("-/tags/{}", escape(tag)),
            (Bitbucket, Page::Tag(tag)) => format!("src/{}", escape(tag)),
            (GitHub, Page::NewPullRequest(from)) => {
                format!("compare/{}?expand=1", escape_ref(from))
            }
            (GitLab, Page::NewPullRequest(from)) => format!(
                "-/merge_requests/new?merge_request%5Bsource_branch%5D={}",
                escape(from)
            ),
            (Bitbucket, Page::NewPullRequest(from)) => {
                format!("pull-requests/new?source={}", escape(from))
            }
        };
        format!("{}/{tail}", self.home())
    }
}

/// `text` with every byte outside RFC 3986's unreserved set percent-encoded.
pub fn escape(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// A ref as a URL path: each segment escaped, its slashes kept.
fn escape_ref(name: &str) -> String {
    name.split('/').map(escape).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(remote: &str) -> Hosted {
        hosted(remote).unwrap_or_else(|| panic!("{remote} should be hosted"))
    }

    #[test]
    fn every_form_git_takes_names_the_same_repository() {
        for remote in [
            "git@github.com:acme/tool.git",
            "github.com:acme/tool",
            "https://github.com/acme/tool.git",
            "https://github.com/acme/tool/",
            "http://github.com/acme/tool",
            "ssh://git@github.com/acme/tool.git",
            "git://github.com/acme/tool",
            "  https://GitHub.com/acme/tool  ",
        ] {
            let found = read(remote);
            assert_eq!(found.forge, Forge::GitHub, "{remote}");
            assert_eq!(
                (found.owner.as_str(), found.repo.as_str()),
                ("acme", "tool")
            );
            assert_eq!(found.home(), "https://github.com/acme/tool", "{remote}");
        }
    }

    #[test]
    fn credentials_and_ports_are_dropped() {
        let found = read("https://someone:secret@www.gitlab.com:8443/team/app");
        assert_eq!(found.forge, Forge::GitLab);
        assert_eq!(found.home(), "https://www.gitlab.com/team/app");
    }

    #[test]
    fn gitlab_groups_nest_and_the_others_do_not() {
        let found = read("git@gitlab.com:group/sub/app.git");
        assert_eq!(
            (found.owner.as_str(), found.repo.as_str()),
            ("group/sub", "app")
        );
        assert!(hosted("https://github.com/owner/repo/extra").is_none());
    }

    #[test]
    fn unknown_hosts_local_paths_and_odd_names_are_not_hosted() {
        for remote in [
            "https://code.example.org/acme/tool.git",
            "git@example.com:owner/repo.git",
            "/srv/git/tool.git",
            "../sibling",
            "https://github.com/acme",
            "https://github.com//repo",
            "https://github.com/ac me/tool",
            "github.com",
            "",
        ] {
            assert!(hosted(remote).is_none(), "{remote:?}");
        }
    }

    #[test]
    fn each_forge_has_its_own_pages() {
        let cases = [
            (
                "https://github.com/a/b",
                "https://github.com/a/b/commit/f00",
                "https://github.com/a/b/releases/tag/v2",
                "https://github.com/a/b/compare/topic?expand=1",
            ),
            (
                "https://gitlab.com/a/b",
                "https://gitlab.com/a/b/-/commit/f00",
                "https://gitlab.com/a/b/-/tags/v2",
                "https://gitlab.com/a/b/-/merge_requests/new?merge_request%5Bsource_branch%5D=topic",
            ),
            (
                "https://bitbucket.org/a/b",
                "https://bitbucket.org/a/b/commits/f00",
                "https://bitbucket.org/a/b/src/v2",
                "https://bitbucket.org/a/b/pull-requests/new?source=topic",
            ),
        ];
        for (remote, commit, tag, pull) in cases {
            let found = read(remote);
            assert_eq!(found.page(Page::Commit("f00")), commit);
            assert_eq!(found.page(Page::Tag("v2")), tag);
            assert_eq!(found.page(Page::NewPullRequest("topic")), pull);
        }
    }

    #[test]
    fn refs_escape_but_branches_keep_their_slashes() {
        let found = read("https://github.com/a/b");
        assert_eq!(
            found.page(Page::Branch("fix/the bug#2")),
            "https://github.com/a/b/tree/fix/the%20bug%232"
        );
        assert_eq!(
            found.page(Page::Tag("release/1")),
            "https://github.com/a/b/releases/tag/release%2F1"
        );
        assert_eq!(escape("a+b@x.com"), "a%2Bb%40x.com");
    }
}
