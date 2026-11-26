# Changelog

All notable changes to Rama are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org).

## [0.10.0] - 2026-11-26

### Added
- Tabs can be dragged. Dropped beside another tab, a tab lands on that side
  and takes its group, or none; dropped on a group it joins it; dropped at the
  end of the strip it goes last, out of any group. A bar shows where it lands.
- A group's dropdown rows drag out the same way, back into the strip.
- Groups can be dragged whole, before or after another group, beside a tab or
  to the end; their tabs move together.
- Across the top, a tab can sit between two groups: the left half of a group
  takes it just before the group.
- A tab's menu can close the other tabs, the tabs to its right, or all of
  them.
- Rename tab heads a tab's menu. A double click renames a tab in a group's
  dropdown, the tab a group shows across the top, or a group's heading down
  the side.
- Move to opens a tab's groups beside the menu.
- A tooltip shows a group's name, or a name in a menu, when it is cut short.

### Changed
- Across the top, the strip follows the tabs' order: a group stands where its
  tabs are, and the tab title gives way before the group's name.
- A group with no tabs shows its colour dimmed.

### Fixed
- On Windows, cloning a local folder named the copy after the whole path.

## [0.9.0] - 2026-11-26

### Added
- A tab whose name is too long for it shows the whole name in a tooltip.
- A group's colour can be any colour: the palette at the end of its colours
  opens a colour picker.

### Changed
- A group's menu offers eight colours in one row; Grey is no longer offered,
  and groups that have it keep it.

## [0.8.0] - 2026-11-17

### Added
- Project tabs can be grouped. A right click on a tab adds it to a new group
  or moves it to another; a right click on a group renames it, changes its
  colour, ungroups it or closes its tabs.
- Down the sidebar, a Groups section lists each group as a heading over its
  tabs; a click folds it. Its button makes a new, empty group.
- Across the top, a group is one tab that opens a list of its projects, and
  shows the one on screen.

## [0.7.0] - 2026-11-16

### Added
- The project tabs can run down a sidebar on the left or the right, from
  Settings → Appearance → Tabs. The sidebar lists them under a Projects
  heading, with a + to open another.
- A button in the history's header hides or shows the sidebar of branches,
  remotes, tags and stashes; the choice is remembered.
- Ctrl+F (Cmd+F on macOS) opens the commit search. Enter goes to the next
  match, Shift+Enter to the previous one, Escape closes it.
- A Mono accent: near white on the dark theme, near black on the light one.
- Settings has Back to app at the bottom of its rail.

### Changed
- The branch, the toolbar and search share one band above the graph. The
  branch is a pill, and search drops down a find card under its button.
- Tabs show their project's icon without a tile around it.
- While Settings is open the tabs are hidden. Its pages have a description
  under the title and their rows sit in an outlined card; choices are a
  segmented control.
- About shows the app's mark on its own, larger, in the page's colour.
- The window buttons have new glyphs, and closing no longer turns red on
  hover on Linux.
- The accent swatches are ringed, so the chosen one is clear in any colour.

### Fixed
- The + by the tabs opens the folder picker; it used to only go Home.
- Graph palettes with fewer colours line up with the others in Settings.

## [0.6.0] - 2026-11-16

### Added
- Each file in the working tree's staged and unstaged lists can be discarded
  on its own, from a trash button on its row. Rama asks first; Discard All
  stays.

### Changed
- Project tabs are plates with the project's icon on a small tile. They share
  the title band when it runs short, the selected one staying readable, and a
  long name fades out instead of ending in an ellipsis. The + button sits
  right after the last tab.

### Fixed
- Discarding a renamed file brings its old name back.

## [0.5.1] - 2026-11-11

### Added
- A double click on a project tab renames it. The name is kept for that
  repository; leaving it blank brings back the folder's name.
- Right-clicking a stash, in the history or the sidebar, offers to copy its
  sha or delete it.

### Fixed
- Toasts leave on their own, failures included; hovering one holds it.
- A long commit message scrolls inside the side panel instead of pushing
  the author and the files out of view.

## [0.5.0] - 2026-11-06

### Changed
- Rama draws its text in Geist and its code in Geist Mono, shipped with the
  app, on every platform.
- Branch and tag labels in the history are quieter: a faint fill with a thin
  edge in their colour, and rounder corners.
- The sidebar's rows highlight as rounded plates, its sections are parted by
  space instead of lines, and the filter has no border. The branch header and
  the toolbar under it read as one band.
- Settings centres its content in a column, with each page's rows in one
  filled block. About centres the app's name under a larger icon.

### Fixed
- With more tabs than fit, the active tab is scrolled into view, including
  the one restored at startup.

## [0.4.1] - 2026-11-05

### Added
- macOS: the first launch installs the `rama` terminal command. If that
  fails, Settings > About > Terminal command installs it.

### Changed
- Opening a file in the diff scrolls to its first change, with three lines
  above it. Changing the layout does the same.

### Fixed
- Windows: the build no longer fails lint on a Unix-only constant.

## [0.4.0] - 2026-11-05

### Added
- `rama .` opens the repository you are in from a terminal, like `code .`.
  Any path inside a checkout works; if Rama is already open, the repository
  opens there as a new tab. The Windows installer puts `rama` on the `PATH`.
- In the diff, click a line to select it; Shift-click or drag to select more,
  and Ctrl+C (⌘C) copies their code. Right-click a line to copy it, its hunk
  or the file's path.

### Fixed
- Windows: Rama no longer opens a console window beside its own, and closing
  one no longer closes the other.
- Windows: the installer shows its steps and a Finish page, and creates
  shortcuts on the desktop and in the Start menu. Run the new `.msi` to get
  them; it replaces the installed version.

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
