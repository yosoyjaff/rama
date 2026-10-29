# Changelog

All notable changes to Rama are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org).

## [0.3.0] - 2026-10-28

### Changed
- A merge records a merge commit even when it could fast-forward. Settings >
  History turns this off.
- Checking out a commit or creating a branch carries uncommitted changes along,
  as switching branches does; if they clash, they are kept in the stash.

### Fixed
- Files left in conflict by a stash that did not apply cleanly show in the
  panel as Conflicted and Resolved, as during a merge, and Commit waits until
  they are resolved.

## [0.2.0] - 2026-10-26

### Changed
- Built on GPUI Kit 0.8. On Linux the window now has a square frame.

### Fixed
- "Load more" at the bottom of the history no longer scrolls back to the top.

## [0.1.0] - 2026-10-21

First public release.

### Added
- Commit graph with branch and tag badges, stashes and a work-in-progress row.
- Search that highlights matching commits in place.
- Commit details, side-by-side and inline diffs, blame and a merge editor.
- Pull, push, fetch all, stash, undo/redo, branches, merge, rebase, cherry-pick,
  revert, reset, tags, patches and worktrees.
- Tabs for several repositories, a home screen with recent repositories, clone
  and create.
- Light and dark themes with preset and custom accent colours.
- Graph colour palettes: Rama, plus palettes inspired by GitKraken, Tower,
  GitHub, JetBrains IDEs, Fork and Git Graph for VS Code.
- In-app updates from GitHub Releases.
- Installers: `.deb`, `.rpm` and `.AppImage` on Linux, `.dmg` on macOS and
  `.msi` on Windows.
- Builds for x86_64 and Arm on every platform.
