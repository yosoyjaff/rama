# AGENTS.md

Rama is a native Git client for Windows, macOS and Linux, in Rust on
[GPUI Kit](https://gpui-kit.com) (the `gpui-kit` crate, which re-exports GPUI).
It drives the `git` CLI; there is no libgit2. What it does is in
[README.md](README.md), how it is laid out in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), how it ships in
[docs/PACKAGING.md](docs/PACKAGING.md).

## Commands

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings   # what CI runs; --all-targets compiles the tests
cargo test                                   # or: cargo test <path filter>
cargo run                                    # anything visual
```

`cargo check` skips `#[cfg(test)]` code; use clippy. Builds compile all of GPUI
and are slow, so start them early.

**Done** means clippy clean, tests green and, for a visual change, the app run
from the binary you just built (a window opened before the build is old code).

## Before writing UI

The GPUI Kit guides are vendored in `.claude/skills/build-gpui-apps/` and are
normative. Start at its `SKILL.md`; the Coding and Design Guides are under
`references/gpui-kit/upstream/docs/`. Don't edit the copies; refresh them with
`scripts/sync_gpui_kit_docs.py`.

Never guess an API by analogy with React, CSS or old GPUI examples. Read the
signature in the crate source under `~/.cargo/registry/src/*/gpui-kit-0.6.*/`
and `gpui-component-0.6.*/`.

## Rules

- **Layering:** `shell → features → ui`, `features → git, settings`,
  `shell → update`. `ui`, `git` and `update` never import views.
- **One command, one implementation:** a command is a GPUI `Action`; its key
  binding, menu item and button all dispatch it. Every action ships with a
  visible trigger, and no event is emitted without a handler.
- **Colours** come from `Theme::of(cx)` roles (`src/ui/theme.rs`). No
  `hsla`/`rgb` literals in views; new colours are new roles, for both themes.
- **Recurring sizes** are named `const`s next to their view.
- **`ElementId`s** come from the domain (SHA, ref name, path), never an index.
- **`render` is pure:** no state changes, no `cx.notify()`.
- **Git, disk and network** run on the background executor.
- **A shrinking flex child** needs `min_w_0`/`min_h_0`.
- **Prefer GPUI Kit components** to hand-built `div` controls.
- **Naming:** `<X>State` for retained models, `on_<intent>` callbacks,
  `render_<region>` helpers (full table in the Coding Guides).
- **Tests** for everything that doesn't need a window; git tests use
  `git::test_repo::TestRepo`.
- **User-facing text** is short, literal English, ready for localisation.
- **No copied code or assets.** Icons come from Solar (CC BY 4.0) or Hugeicons
  (MIT), with attribution kept in `src/ui/icons.rs` and the README.

## Known traps

- Depend on `gpui-kit` only, never on `gpui` directly: two GPUI versions in one
  build are incompatible crates.
- There are two `IconName`s. Pass `gpui_kit::assets::IconName` between layers
  (it is `Copy`); `Icon::new()` takes either.
- `Icon` has no `group_hover`; use `svg().path(name.path())`.
- Icons outside the default bundle go in `assets/icons/`, served by
  `src/assets.rs`.
- GPUI Kit's `TitleBar` forces its own window buttons, so `src/ui/title_bar.rs`
  is hand-built. Controls on the drag strip need `occlude()` and must stop
  `mouse_down`.
- `KeyBinding::new` panics at startup on a bad keystroke. Use `secondary` for
  ⌘/Ctrl, and run the app after touching shortcuts.
- An empty flex child still takes the row's `gap`: don't emit empty slots.
- The repo watcher must ignore `EventKind::Access`, or every git read triggers
  another reload (`src/git/watch.rs`).
