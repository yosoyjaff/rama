# Building, packaging and updates

## Profiles

`Cargo.toml` optimizes dependencies even in `dev` (`[profile.dev.package."*"]
opt-level = 2`): an unoptimized GPUI app is unusable, because text shaping,
layout and the renderer run every frame. Rama's own code stays at
`opt-level = 1` so rebuilds stay fast.

`release` uses `lto = "thin"`, `codegen-units = 1` and `strip = true`.

## Per platform

| | Architectures | What CI publishes | Next real step |
| --- | --- | --- | --- |
| Linux | x86_64, aarch64 | `.tar.gz`, `.deb`, `.rpm`, `.AppImage` | Flatpak for a sandbox |
| macOS | Apple Silicon, Intel | `.tar.gz` with `Rama.app`, `.dmg` | Developer ID signing + notarization |
| Windows | x64, Arm64 | `.zip`, `.msi` | Signing the `.msi` too |

Everything is built by `packaging/package.sh` from the release binary. The
archive is what the updater installs; the rest are installers:

- **`.deb` / `.rpm`**: `cargo-deb` and `cargo-generate-rpm`, configured in
  `Cargo.toml` (`[package.metadata.deb]`, `[package.metadata.generate-rpm]`).
  Both depend on `git`.
- **`.AppImage`**: an AppDir handed to `appimagetool`.
- **`.dmg`**: `hdiutil`, with `Rama.app` beside a link to `/Applications`.
- **`.msi`**: WiX v5 from `packaging/windows/rama.wxs`, installed per machine
  with a Start menu shortcut. Its `UpgradeCode` must never change.

Linux builds on Ubuntu 22.04, so the packages run on glibc 2.35 or newer.

Linux build dependencies: `libfontconfig-dev`, `libxkbcommon-dev`,
`libxkbcommon-x11-dev`, `libwayland-dev`. The X11 backend is `x11rb` (pure
Rust), so `libX11` isn't needed. At runtime a Vulkan/GL driver is.

### macOS

The bundle gets an ad-hoc signature (`codesign --sign -`), enough to run on
Apple Silicon, but Gatekeeper blocks it when downloaded from a browser. The
full path is `codesign --options runtime` with a Developer ID certificate →
`notarytool submit --wait` → `stapler staple`. It needs secrets in CI (the
`.p12` certificate, its password, a notarization API key), so the workflow
doesn't do it yet: a signing step with empty secrets either fails silently or
publishes something unsigned.

### Windows

Signing is done by **SignPath Foundation** (free for open source; the key lives
in their HSM, never in CI). `release.yml` uploads `rama.exe`, requests the
signature, checks it is a valid, timestamped Authenticode signature, and swaps
it in for the binary **before** packaging, so the `.sha256` and `latest.json`
describe the signed exe.

These steps switch on when the repository variable `SIGNPATH_ORGANIZATION_ID`
exists; without it the build is unsigned. With it, any signing failure fails
the job. In GitHub (Settings → Secrets and variables → Actions):

| Kind | Name |
| --- | --- |
| Secret | `SIGNPATH_API_TOKEN` (a user with the *submitter* role) |
| Variable | `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG` |

In SignPath: the GitHub.com *trusted build system* linked to the project, and
an *artifact configuration* `<zip-file>` holding a `<pe-file path="rama.exe">`
(GitHub artifacts arrive zipped). The Foundation requires a public repository,
an OSI licence and the README's *Code signing policy* section. Once accepted,
restore the README's "Free code signing provided by SignPath.io" line.

A new certificate doesn't silence SmartScreen at once: "unknown publisher"
becomes the project's name, and the warning goes away as reputation builds.

## Releasing

Releases are published by `.github/workflows/release.yml`, which runs **only**
when a version tag is pushed (or by hand from the Actions tab), so ordinary
pushes cost no Actions minutes.

```bash
# 1. bump `version` in Cargo.toml, move the [Unreleased] entries of
#    CHANGELOG.md under `## [0.2.0] - YYYY-MM-DD`, and commit
# 2. tag that version
git tag v0.2.0 && git push origin v0.2.0
```

The workflow:

1. checks that the tag matches `Cargo.toml` (otherwise the build would offer
   itself the same update forever) and that `CHANGELOG.md` has a section for
   that version, which becomes the release notes;
2. creates the release as a **draft**;
3. builds Linux, Windows and macOS, each for x86_64 and Arm, and packages each
   with `packaging/package.sh` (archive, installers, a `.sha256` for each);
4. uploads everything plus `latest.json` (from `packaging/manifest.sh`, with
   this release's URLs) and only then publishes. While anything is missing it
   stays a draft, invisible to the updater and to everyone else.

### Third-party licences

Every archive carries `LICENSE` and `THIRD_PARTY_LICENSES.txt`: MIT and Apache
require their text to travel with each copy of the binary. The `prepare` job
generates it once with `cargo about`, and `package.sh` refuses to package
without it. `packaging/licenses/about.toml` lists the accepted licences;
GPL/LGPL are not on it, and the crates that offer them also offer MIT or
Apache. To package locally:

```bash
cargo install --locked cargo-about@0.9.2 --features cli
cargo about generate --locked -c packaging/licenses/about.toml \
  packaging/licenses/about.hbs -o THIRD_PARTY_LICENSES.txt
```

If a new dependency brings a licence that isn't on the list, `cargo about`
fails: review it before adding it to `about.toml`.

### CI

The same tag runs `.github/workflows/ci.yml`: `cargo fmt --check`,
`clippy --all-targets -D warnings` and `cargo test` on Linux, macOS and
Windows. It doesn't block the release; if it goes red, delete the draft and the
tag, fix, and tag again.

## Updates

`src/update/` and `src/updater.rs`:

- **When:** a few seconds after the window opens, in the background. If it
  fails (offline, no releases) it says nothing; **Check for updates** in
  Settings → About does report it.
- **Where:** a notification at the bottom right (**Update** while a new
  version is available, **Restart** once it is installed) and the same row in
  Settings → About.
- **From:** `https://github.com/yosoyjaff/rama/releases/latest/download/latest.json`
  (`repository` in `Cargo.toml`). An anonymous download: no token in the binary
  and no API rate limit. **The repository must be public**: while private,
  GitHub answers 404 (the startup check says nothing; the manual one says "no
  public release found").
- **How it installs** (`src/update/install.rs`): downloads its platform's
  archive, checks size and SHA-256 against the manifest **before touching
  anything**, unpacks it next to the installation and swaps it in with a
  rename: the binary on Linux (`~/.local/bin/rama`) and Windows (the portable
  `rama.exe`), the whole `Rama.app` on macOS. The old copy is set aside as
  `*.old` and removed on the next start. **Restart** launches the new one and
  quits.
- A development build (`cargo run`) is not replaced; it offers **View
  release**.

What it does **not** cover: the SHA-256 proves the bytes are the release's, not
who published it (that is what the manifest's `signature` field and a key are
for); and when Rama is installed by a package manager (Homebrew, a `.deb`, an
installer), that manager should update it, not the app.

### Without certificates

- **macOS:** the **first** browser download is blocked by Gatekeeper:
  right-click → Open (or System Settings → Privacy & Security → Open Anyway),
  once. Updates are downloaded by the app itself, without the browser's
  quarantine flag, so they don't ask again.
- **Windows:** running needs no signature. SmartScreen warns ("unknown
  publisher") the first time something downloaded with a browser opens: More
  info → Run anyway. The app's own updates don't go through it.
- **Linux:** nothing to sign.

Removing those first-install warnings takes a Developer ID and notarization on
macOS (an Apple Developer account, USD 99/year) and a code signing certificate
on Windows.
