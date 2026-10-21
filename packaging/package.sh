#!/usr/bin/env bash
# Package one built target into dist/: the archive the updater installs, the
# platform's installers, and a SHA-256 next to each.
#
#   Linux    .tar.gz, .deb, .rpm, .AppImage
#   macOS    .tar.gz, .dmg
#   Windows  .zip, .msi
#
# Inputs: $BINARY, $VERSION, $TARGET. Run from the repository root, with
# THIRD_PARTY_LICENSES.txt next to it (packaging/licenses/, `cargo about`).
# The installers need their tools on PATH: cargo-deb, cargo-generate-rpm and
# appimagetool on Linux, hdiutil on macOS, wix (v5) on Windows.
set -euo pipefail

binary="${BINARY:-rama}"
version="${VERSION:?set VERSION}"
target="${TARGET:?set TARGET}"
# Files are named <arch>-<os> (x86_64-linux, aarch64-macos, x86_64-windows),
# not by the full triple; manifest.sh maps the name back to the triple.
case "$target" in
  *-linux-*) os=linux ;;
  *-apple-darwin) os=macos ;;
  *-windows-*) os=windows ;;
  *) echo "error: no file name for target $target" >&2; exit 1 ;;
esac
name="${binary}-${version}-${target%%-*}-${os}"
built="target/${target}/release"

# A binary carries its dependencies' licences: MIT and Apache ask for their
# text to travel with every copy, so an archive without it is not shipped.
notices="THIRD_PARTY_LICENSES.txt"
if [ ! -s "$notices" ]; then
  echo "error: $notices is missing; generate it with cargo about (docs/PACKAGING.md)" >&2
  exit 1
fi

# The manifest needs one for the archive, and a checksum the release did not
# compute itself is a checksum of whatever was uploaded.
checksum() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1 > "$1.sha256"
  else
    shasum -a 256 "$1" | cut -d' ' -f1 > "$1.sha256"
  fi
  echo "packaged $1"
}

rm -rf staging
mkdir -p dist staging
case "$target" in
  *windows*)
    cp "${built}/${binary}.exe" "staging/${binary}.exe"
    cp LICENSE "$notices" staging/
    (cd staging && 7z a -tzip "../dist/${name}.zip" .) >/dev/null
    checksum "dist/${name}.zip"

    # Windows Installer takes only numeric versions: 0.2.0-beta.1 is 0.2.0.
    case "$target" in
      aarch64-*) arch=arm64 ;;
      *) arch=x64 ;;
    esac
    mkdir -p staging/msi
    wix build packaging/windows/rama.wxs -arch "$arch" \
      -d Version="${version%%-*}" -d BinDir="$built" \
      -o "staging/msi/${binary}.msi"
    # Built aside: wix also writes a .wixpdb, which is not shipped.
    mv "staging/msi/${binary}.msi" "dist/${name}.msi"
    checksum "dist/${name}.msi"
    ;;
  *apple-darwin*)
    # The icon comes from the bundle, not the binary: Contents/Info.plist
    # names the .icns in Contents/Resources.
    app="staging/Rama.app/Contents"
    mkdir -p "$app/MacOS" "$app/Resources"
    cp "${built}/${binary}" "$app/MacOS/${binary}"
    cp assets/app-icon/app.rama.icns "$app/Resources/app.rama.icns"
    sed "s/__VERSION__/${version}/g" packaging/macos/Info.plist > "$app/Info.plist"
    cp LICENSE "$notices" "$app/Resources/"
    # Ad-hoc signature: no Apple account or certificate involved. It is the
    # minimum Apple Silicon needs to run the binary, and it seals the bundle
    # as one thing. It does not satisfy Gatekeeper for a browser download —
    # that needs a Developer ID and notarization (docs/PACKAGING.md).
    codesign --force --deep --sign - "staging/Rama.app"
    tar -czf "dist/${name}.tar.gz" -C staging .
    checksum "dist/${name}.tar.gz"

    # The disk image opens on Rama.app beside a link to /Applications, the
    # usual drag-to-install window. ditto keeps the signature intact.
    mkdir staging/dmg
    ditto staging/Rama.app staging/dmg/Rama.app
    ln -s /Applications staging/dmg/Applications
    # hdiutil fails now and then on CI runners ("Resource busy"): retry.
    for attempt in 1 2 3; do
      hdiutil create -volname Rama -srcfolder staging/dmg -ov -format UDZO \
        "dist/${name}.dmg" && break
      [ "$attempt" -eq 3 ] && exit 1
      sleep 5
    done
    checksum "dist/${name}.dmg"
    ;;
  *)
    # Linux: the desktop finds the icon through the .desktop entry, which
    # install.sh puts in place next to the hicolor PNGs.
    cp "${built}/${binary}" "staging/${binary}"
    cp packaging/linux/app.rama.desktop packaging/linux/install.sh staging/
    cp LICENSE "$notices" staging/
    mkdir -p staging/icons
    cp assets/app-icon/*.png staging/icons/
    tar -czf "dist/${name}.tar.gz" -C staging .
    checksum "dist/${name}.tar.gz"

    # What goes in each package is in Cargo.toml's [package.metadata.deb]
    # and [package.metadata.generate-rpm].
    cargo deb --no-build --no-strip --target "$target" -o "dist/${name}.deb"
    checksum "dist/${name}.deb"
    cargo generate-rpm --target "$target" -o "dist/${name}.rpm"
    checksum "dist/${name}.rpm"

    # AppImage: the same files under an AppDir, whose root holds the desktop
    # entry, its icon and AppRun, the program appimagetool starts.
    appdir="staging/Rama.AppDir"
    install -Dm755 "${built}/${binary}" "$appdir/usr/bin/${binary}"
    install -Dm644 packaging/linux/app.rama.desktop "$appdir/usr/share/applications/app.rama.desktop"
    for size in 32 64 128 256 512; do
      install -Dm644 "assets/app-icon/${size}x${size}.png" \
        "$appdir/usr/share/icons/hicolor/${size}x${size}/apps/app.rama.png"
    done
    install -Dm644 LICENSE "$notices" -t "$appdir/usr/share/doc/${binary}/"
    cp packaging/linux/app.rama.desktop "$appdir/"
    cp assets/app-icon/512x512.png "$appdir/app.rama.png"
    ln -s app.rama.png "$appdir/.DirIcon"
    ln -s "usr/bin/${binary}" "$appdir/AppRun"
    ARCH="${target%%-*}" appimagetool --no-appstream "$appdir" "dist/${name}.AppImage"
    checksum "dist/${name}.AppImage"
    ;;
esac
rm -rf staging
