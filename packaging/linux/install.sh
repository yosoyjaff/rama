#!/usr/bin/env bash
# Install Rama for the current user: the binary in ~/.local/bin, the desktop
# entry and the icon where XDG desktops look for them. The desktop entry's
# file name and `Icon=` must match the window's app_id (`app.rama`), or the
# taskbar shows a generic icon and does not group the windows.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"

install -Dm755 "$here/rama" "$HOME/.local/bin/rama"
install -Dm644 "$here/app.rama.desktop" "$data/applications/app.rama.desktop"
for size in 32 64 128 256 512; do
  install -Dm644 "$here/icons/${size}x${size}.png" \
    "$data/icons/hicolor/${size}x${size}/apps/app.rama.png"
done

command -v update-desktop-database >/dev/null 2>&1 \
  && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 \
  && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true

echo "installed rama to $HOME/.local/bin/rama"
