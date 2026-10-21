//! Author photographs.
//!
//! Git stores a name and an email address and nothing else, so a face has to
//! be asked for elsewhere. A repository on GitHub asks GitHub — its commits
//! name each author's account, and the account has a real profile photo. A
//! LOCAL repository asks GitHub nothing. Whoever GitHub does not place — and
//! everyone in a local repository — gets Gravatar's `d=retro`, which never
//! 404s because it synthesizes an identicon from the address.
//!
//! Two caches on disk make this a once-per-machine cost. The images, keyed by
//! the URL they came from, and `index.json`, which remembers which photo each
//! author was placed with — so a history that was resolved once paints its
//! faces on the next launch without a single request. GitHub calls carry the
//! token of the environment or of the `gh` CLI session, without which a
//! private repository's commits answer 404.
//!
//! **This is the only network the application makes.** It runs on the
//! background executor, and every failure is silent by design: a history that
//! cannot reach the internet is a history with initials in it, not a history
//! with an error in it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::command::git;
use super::forge::{self, Forge};

/// Pixels asked of both sources. The node draws an 18px disc, so this is the
/// same image on a 2× display and nothing more.
const AVATAR_SIZE: u32 = 40;
/// The most a photo download may weigh; anything bigger is not a 40px
/// photo, and is cut off rather than read whole.
const PHOTO_LIMIT: u64 = 1 << 21;
/// How long the network is left alone after a miss. Without it, a machine with
/// no connection retries the whole visible page on every repaint.
const RETRY_COOLDOWN: Duration = Duration::from_secs(60);

/// An author, as git knows them: an email address. It is what the hosting
/// metadata is matched on and what addresses the identicon, and it is all
/// either source needs.
pub type AuthorRef = String;

/// What the misses of this process know, so a dead network is asked once a
/// minute rather than once a frame.
static MISSES: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// Where photographs land on this machine: a folder of the application's own,
/// shared by every repository — the same author's photo is fetched once, not
/// once per checkout.
pub fn cache_dir() -> PathBuf {
    crate::settings::avatars_dir()
}

/// Resolve photographs for these authors, returning the file each one landed
/// in, by lowercased email.
///
/// In order, per author:
/// 1. **The index on disk** — who was already placed, on any earlier run. A
///    placed author costs no request, ever again.
/// 2. **A `users.noreply.github.com` address**, which names its account id
///    and so its photo, with no API call at all.
/// 3. **GitHub**: the repository's recent commits in one request, then one
///    `?author=<email>` query for each author those did not cover — so an
///    author who only appears deep in the history still gets their photo.
///    With no GitHub remote, a token-backed search across GitHub.
/// 4. **Gravatar's identicon** for whoever GitHub does not know.
///
/// Best effort throughout: a source that is unreachable, unauthorized or
/// rate-limited leaves that author for next time rather than recording them
/// as having no photo.
pub fn resolve(
    repo_path: &Path,
    authors: &[AuthorRef],
    cache_dir: &Path,
) -> HashMap<String, PathBuf> {
    let mut resolved = HashMap::new();
    if authors.is_empty() {
        return resolved;
    }
    // One pass at a time: two pages resolving at once would ask GitHub the
    // same questions twice and race on the index file.
    let _guard = RESOLVE_LOCK.lock();
    // A repository with no GitHub remote is local: nothing in it is GitHub's
    // to answer for, so every author gets their Gravatar.
    let Some((owner, repo)) = github_remote(repo_path) else {
        gravatar_avatars(authors, cache_dir, &mut resolved);
        return resolved;
    };
    let now = unix_now();
    let mut index = PhotoIndex::load(cache_dir);
    let mut pending: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for email in authors.iter().map(|email| normalize(email)) {
        if email.is_empty() || !seen.insert(email.clone()) {
            continue;
        }
        match index.lookup(&email, now) {
            Known::Photo(url) => match cache(&url, cache_dir) {
                Some(path) => {
                    resolved.insert(email, path);
                }
                None => pending.push(email),
            },
            Known::NoPhoto => {}
            Known::Unknown => pending.push(email),
        }
    }

    let mut answers: HashMap<String, Option<String>> = HashMap::new();
    pending.retain(|email| match github_noreply_url(email) {
        Some(url) => {
            answers.insert(email.clone(), Some(url));
            false
        }
        None => true,
    });
    if !pending.is_empty() {
        github_repo_avatars(&owner, &repo, &pending, &mut answers);
    }
    for (email, answer) in answers {
        match answer {
            Some(url) => {
                if let Some(path) = cache(&url, cache_dir) {
                    index.put(&email, Some(url), now);
                    resolved.insert(email, path);
                }
            }
            None => index.put(&email, None, now),
        }
    }
    index.save(cache_dir);

    gravatar_avatars(authors, cache_dir, &mut resolved);
    resolved
}

/// Serializes [`resolve`] across the process.
static RESOLVE_LOCK: Mutex<()> = Mutex::new(());

/// How long "GitHub has no photo for this address" is believed before it is
/// asked again: long enough that a history is not re-queried every launch,
/// short enough that a new account shows up within the week.
const NO_PHOTO_TTL_SECS: u64 = 7 * 24 * 60 * 60;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// What the index knows about one address.
#[derive(Debug, PartialEq, Eq)]
enum Known {
    Photo(String),
    NoPhoto,
    Unknown,
}

/// Which photo each author was placed with, kept beside the images:
/// `index.json`, lowercased email → the photo's URL, or `null` for an address
/// GitHub was asked about and does not know.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct PhotoIndex {
    authors: HashMap<String, IndexEntry>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct IndexEntry {
    url: Option<String>,
    /// Unix seconds of the answer.
    at: u64,
}

impl PhotoIndex {
    fn path(cache_dir: &Path) -> PathBuf {
        cache_dir.join("index.json")
    }

    fn load(cache_dir: &Path) -> Self {
        std::fs::read(Self::path(cache_dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn lookup(&self, email: &str, now: u64) -> Known {
        match self.authors.get(email) {
            Some(IndexEntry { url: Some(url), .. }) => Known::Photo(url.clone()),
            Some(IndexEntry { url: None, at }) if now.saturating_sub(*at) < NO_PHOTO_TTL_SECS => {
                Known::NoPhoto
            }
            _ => Known::Unknown,
        }
    }

    fn put(&mut self, email: &str, url: Option<String>, now: u64) {
        self.authors
            .insert(email.to_string(), IndexEntry { url, at: now });
    }

    /// Write beside the target and rename, like the images: a half-written
    /// index read back as empty would re-ask GitHub about everyone.
    fn save(&self, cache_dir: &Path) {
        let Ok(bytes) = serde_json::to_vec(self) else {
            return;
        };
        if std::fs::create_dir_all(cache_dir).is_err() {
            return;
        }
        let path = Self::path(cache_dir);
        let temporary = path.with_extension("json.tmp");
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
    }
}

/// `12345+octocat@users.noreply.github.com` names its own photo: the number
/// is the account id the CDN serves by.
fn github_noreply_url(email: &str) -> Option<String> {
    let local = email.strip_suffix("@users.noreply.github.com")?;
    let (id, _) = local.split_once('+')?;
    if id.is_empty() || !id.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    Some(format!(
        "https://avatars.githubusercontent.com/u/{id}?s={AVATAR_SIZE}"
    ))
}

/// The GitHub repository `origin` points at, if it points at one.
fn github_remote(repo_path: &Path) -> Option<(String, String)> {
    let url = git(repo_path, &["remote", "get-url", "origin"]).ok()?;
    on_github(&url)
}

/// Owner and repository of a remote hosted on GitHub.
fn on_github(remote_url: &str) -> Option<(String, String)> {
    let hosted = forge::hosted(remote_url).filter(|hosted| hosted.forge == Forge::GitHub)?;
    Some((hosted.owner, hosted.repo))
}

/// The photo of the account that owns the GitHub repository `remote_url`
/// points at, which a ref badge draws for that remote. `github.com/<owner>.png`
/// answers for people and organizations alike, without the API. `None` off
/// GitHub, or offline: the badge then draws the forge's mark.
pub fn remote_owner_avatar(remote_url: &str, cache_dir: &Path) -> Option<PathBuf> {
    let (owner, _) = on_github(remote_url)?;
    cache(
        &format!("https://github.com/{owner}.png?size={AVATAR_SIZE}"),
        cache_dir,
    )
}

/// Photographs from the repository's own commits on GitHub.
///
/// The recent commits first, in one request, because that answer carries
/// every author on it. Then one `?author=<email>` query per author it did not
/// place: GitHub filters a repository's commits by author address, which is
/// what finds someone who only appears far down the history. An `answers`
/// entry of `None` is a real "no photo"; an author left out was not answered
/// (network, rate limit, no access) and is asked again next time.
fn github_repo_avatars(
    owner: &str,
    repo: &str,
    pending: &[String],
    answers: &mut HashMap<String, Option<String>>,
) {
    let wanted: HashSet<&str> = pending.iter().map(String::as_str).collect();
    let recent = format!("https://api.github.com/repos/{owner}/{repo}/commits?per_page=100");
    let Some(rows) = get_json::<Vec<GitHubCommit>>(&recent) else {
        // The repository itself did not answer: private without a token, or
        // offline. Nothing here is known to have no photo.
        return;
    };
    for row in rows {
        let email = normalize(&row.commit.author.email);
        if wanted.contains(email.as_str())
            && !answers.contains_key(&email)
            && let Some(url) = row
                .author
                .and_then(|author| sized_avatar(&author.avatar_url))
        {
            answers.insert(email, Some(url));
        }
    }
    for email in pending {
        if answers.contains_key(email) {
            continue;
        }
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/commits?per_page=1&author={}",
            forge::escape(email)
        );
        let Some(rows) = get_json::<Vec<GitHubCommit>>(&url) else {
            continue;
        };
        let photo = rows
            .into_iter()
            .next()
            .and_then(|row| row.author)
            .and_then(|author| sized_avatar(&author.avatar_url));
        answers.insert(email.clone(), photo);
    }
}

/// GitHub's photo URL at the size the node draws — and only if it is on
/// GitHub's avatar CDN: the URL is remote input, so it is pinned to the host
/// it should be on rather than trusted for its scheme alone.
fn sized_avatar(avatar_url: &str) -> Option<String> {
    if !avatar_url.starts_with("https://avatars.githubusercontent.com/") {
        return None;
    }
    let separator = if avatar_url.contains('?') { '&' } else { '?' };
    Some(format!("{avatar_url}{separator}s={AVATAR_SIZE}"))
}

/// A GitHub API GET, parsed. `None` for any failure — the callers treat that
/// as "not answered", never as "no photo".
fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Option<T> {
    serde_json::from_slice(&get(url, true)?).ok()
}

/// Gravatar's `d=retro` identicon for every author the hosting metadata could
/// not place.
fn gravatar_avatars(
    authors: &[AuthorRef],
    cache_dir: &Path,
    resolved: &mut HashMap<String, PathBuf>,
) {
    let mut seen = HashSet::new();
    for email in authors {
        let email = normalize(email);
        if email.is_empty() || resolved.contains_key(&email) || !seen.insert(email.clone()) {
            continue;
        }
        if on_cooldown(&email) {
            continue;
        }
        match cache(&gravatar_url(&email), cache_dir) {
            Some(path) => {
                clear_miss(&email);
                resolved.insert(email, path);
            }
            None => note_miss(&email),
        }
    }
}

/// Gravatar addresses an identicon by the hash of the lowercased, trimmed
/// address. SHA-256 is the form it documents today; `d=retro` is what makes an
/// address it has never seen still answer with a mark.
pub fn gravatar_url(email: &str) -> String {
    let digest = Sha256::digest(normalize(email).as_bytes());
    format!(
        "https://gravatar.com/avatar/{}?d=retro&s={AVATAR_SIZE}",
        hex(&digest)
    )
}

/// The file `url`'s image is kept in, downloaded first if it is not there.
fn cache(url: &str, cache_dir: &Path) -> Option<PathBuf> {
    let file = cache_dir.join(cache_name(url));
    let cached = std::fs::metadata(&file).map(|meta| meta.len()).unwrap_or(0) > 0;
    if !cached {
        let bytes = get(url, false).filter(|bytes| !bytes.is_empty())?;
        std::fs::create_dir_all(cache_dir).ok()?;
        // Into a side file first: a file cut short by a crash would be read
        // as a photo on every later run.
        let partial = file.with_extension("partial");
        std::fs::write(&partial, bytes).ok()?;
        std::fs::rename(&partial, &file).ok()?;
    }
    Some(file)
}

/// A photo's file name: a digest of its URL, so one photo at one size is one
/// file, whoever it was fetched for.
pub fn cache_name(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    format!("{}.photo", hex(&digest[..12]))
}

/// One GET, bounded, with the headers these two services expect.
///
/// `authorized` sends a token when the environment carries one: GitHub answers
/// anonymous callers sixty times an hour, which one busy repository spends in
/// a minute.
fn get(url: &str, authorized: bool) -> Option<Vec<u8>> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .new_agent();
    let mut request = agent
        .get(url)
        // GitHub refuses a request with no user agent outright.
        .header("User-Agent", crate::APP_NAME)
        .header("Accept", "application/vnd.github+json");
    if authorized && let Some(token) = github_token() {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    let mut response = request.call().ok()?;
    if response.status() != 200 {
        return None;
    }
    response
        .body_mut()
        .with_config()
        .limit(PHOTO_LIMIT)
        .read_to_vec()
        .ok()
}

/// The token GitHub requests go out with, looked up once per process.
///
/// `GITHUB_TOKEN` or `GH_TOKEN` when the environment sets one; otherwise the
/// session of the `gh` CLI the user is already logged in to. Without a token
/// a PRIVATE repository's commit list answers 404, and every author falls
/// back to the Gravatar identicon instead of their photo.
fn github_token() -> Option<&'static str> {
    static TOKEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    TOKEN
        .get_or_init(|| {
            let from_env = |name: &str| std::env::var(name).ok().filter(|token| !token.is_empty());
            from_env("GITHUB_TOKEN")
                .or_else(|| from_env("GH_TOKEN"))
                .or_else(gh_cli_token)
        })
        .as_deref()
}

/// `gh auth token`: the token of the logged-in `gh` session, if there is one.
fn gh_cli_token() -> Option<String> {
    let output = std::process::Command::new("gh")
        .args(["auth", "token", "--hostname", "github.com"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

fn normalize(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn on_cooldown(email: &str) -> bool {
    let guard = MISSES.lock();
    let Ok(guard) = guard else { return false };
    guard
        .as_ref()
        .and_then(|misses| misses.get(email))
        .is_some_and(|last| last.elapsed() < RETRY_COOLDOWN)
}

fn note_miss(email: &str) {
    if let Ok(mut guard) = MISSES.lock() {
        let misses = guard.get_or_insert_with(HashMap::new);
        // Forget the misses whose cooldown is over, or the map only grows.
        misses.retain(|_, last| last.elapsed() < RETRY_COOLDOWN);
        misses.insert(email.to_string(), Instant::now());
    }
}

fn clear_miss(email: &str) {
    if let Ok(mut guard) = MISSES.lock()
        && let Some(misses) = guard.as_mut()
    {
        misses.remove(email);
    }
}

#[derive(serde::Deserialize)]
struct GitHubCommit {
    author: Option<GitHubUser>,
    commit: GitHubCommitMetadata,
}

#[derive(serde::Deserialize)]
struct GitHubUser {
    avatar_url: String,
}

#[derive(serde::Deserialize)]
struct GitHubCommitMetadata {
    author: GitHubCommitAuthor,
}

#[derive(serde::Deserialize)]
struct GitHubCommitAuthor {
    email: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;

    #[test]
    fn only_a_github_remote_is_asked_about() {
        assert_eq!(
            on_github("git@github.com:owner/repo.git"),
            Some(("owner".into(), "repo".into()))
        );
        assert_eq!(on_github("https://gitlab.com/owner/repo.git"), None);
        assert_eq!(on_github("/srv/git/repo.git"), None);
    }

    #[test]
    fn an_identicon_is_addressed_by_the_normalized_email() {
        let url = gravatar_url("  Person@Example.COM ");
        assert_eq!(url, gravatar_url("person@example.com"));
        assert!(url.contains("d=retro"), "{url}");
        assert!(url.contains(&format!("s={AVATAR_SIZE}")), "{url}");
        // The hash is of the address, not of the string as typed.
        let expected = hex(&Sha256::digest(b"person@example.com"));
        assert!(url.contains(&expected), "{url}");
    }

    #[test]
    fn a_cache_name_is_stable_and_per_url() {
        let one = cache_name("https://example.com/a?s=40");
        assert_eq!(one, cache_name("https://example.com/a?s=40"));
        assert_ne!(one, cache_name("https://example.com/a?s=80"));
        assert!(one.ends_with(".photo"));
    }

    #[test]
    fn a_cached_file_is_returned_without_asking_the_network() {
        let dir = tempfile::tempdir().expect("temporary directory");
        // No request can succeed against this host; the hit has to come from
        // the file that is already there.
        let url = "https://example.invalid/avatar.png";
        let path = dir.path().join(cache_name(url));
        std::fs::write(&path, b"image bytes").expect("write");
        assert_eq!(cache(url, dir.path()), Some(path));
    }

    /// The one thing the rest of these tests cannot prove: that the two
    /// sources answer and the bytes land. Ignored by default — a suite that
    /// needs the internet fails on a train — and run explicitly with
    /// `cargo test -- --ignored avatars`.
    #[test]
    #[ignore = "reaches gravatar.com"]
    fn an_identicon_really_downloads_and_caches() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let email = "person@example.com".to_string();
        let resolved = gravatar_only(std::slice::from_ref(&email), dir.path());

        let path = resolved.get(&email).expect("gravatar answered");
        let bytes = std::fs::read(path).expect("the cached file");
        assert!(!bytes.is_empty());
        // Gravatar's identicon is a PNG.
        assert_eq!(&bytes[..4], b"\x89PNG", "not a png: {:?}", &bytes[..8]);

        // The second pass is the cache: delete the network's answer and it
        // still resolves, because the file is what answers now.
        let again = gravatar_only(std::slice::from_ref(&email), dir.path());
        assert_eq!(again.get(&email), Some(path));
    }

    /// The whole path against a real checkout: the hosting metadata is asked
    /// first — this repository's own remote — and whoever it cannot place
    /// falls through to the identicon. Ignored for the same reason as above.
    #[test]
    #[ignore = "reaches github.com and gravatar.com"]
    fn every_author_of_a_real_checkout_gets_a_photograph() {
        let repo = TestRepo::new();
        repo.commit("first");
        repo.git(&[
            "remote",
            "add",
            "origin",
            "https://github.com/rust-lang/rust.git",
        ]);
        let dir = tempfile::tempdir().expect("temporary directory");

        let emails = vec!["test@example.com".to_string()];
        let resolved = resolve(repo.path(), &emails, dir.path());
        let path = resolved
            .get("test@example.com")
            .expect("every author lands somewhere");
        assert!(std::fs::metadata(path).expect("the file").len() > 0);
    }

    /// [`gravatar_avatars`] on its own, for the test above: no repository, so
    /// no hosting metadata to consult.
    fn gravatar_only(emails: &[String], cache_dir: &Path) -> HashMap<String, PathBuf> {
        let mut resolved = HashMap::new();
        for email in emails {
            clear_miss(email);
        }
        gravatar_avatars(emails, cache_dir, &mut resolved);
        resolved
    }

    #[test]
    fn a_noreply_address_names_its_own_photo() {
        assert_eq!(
            github_noreply_url("12345+octocat@users.noreply.github.com").as_deref(),
            Some("https://avatars.githubusercontent.com/u/12345?s=40")
        );
        assert_eq!(github_noreply_url("octocat@users.noreply.github.com"), None);
        assert_eq!(github_noreply_url("person@example.com"), None);
    }

    #[test]
    fn only_githubs_avatar_cdn_is_trusted() {
        assert_eq!(
            sized_avatar("https://avatars.githubusercontent.com/u/1?v=4").as_deref(),
            Some("https://avatars.githubusercontent.com/u/1?v=4&s=40")
        );
        assert_eq!(sized_avatar("https://evil.example.com/u/1"), None);
        assert_eq!(
            sized_avatar("http://avatars.githubusercontent.com/u/1"),
            None
        );
    }

    #[test]
    fn the_index_remembers_photos_for_good_and_absences_for_a_week() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut index = PhotoIndex::default();
        index.put(
            "with@x.com",
            Some("https://avatars.githubusercontent.com/u/1".into()),
            1_000,
        );
        index.put("without@x.com", None, 1_000);
        index.save(dir.path());

        let index = PhotoIndex::load(dir.path());
        let later = 1_000 + NO_PHOTO_TTL_SECS + 1;
        assert_eq!(
            index.lookup("with@x.com", later),
            Known::Photo("https://avatars.githubusercontent.com/u/1".into())
        );
        assert_eq!(index.lookup("without@x.com", 1_001), Known::NoPhoto);
        assert_eq!(index.lookup("without@x.com", later), Known::Unknown);
        assert_eq!(index.lookup("never@x.com", 1_001), Known::Unknown);
    }

    #[test]
    fn a_miss_is_not_retried_until_the_cooldown_passes() {
        let email = "cooldown@example.com";
        clear_miss(email);
        assert!(!on_cooldown(email));
        note_miss(email);
        assert!(on_cooldown(email));
        clear_miss(email);
        assert!(!on_cooldown(email));
    }
}
