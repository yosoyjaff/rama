#!/usr/bin/env bash
# Print the release manifest `base_update` reads, from a directory of packaged
# archives and their .sha256 files.
#
# Usage: VERSION=0.2.0 BASE_URL=https://… packaging/manifest.sh dist
set -euo pipefail

dir="${1:-dist}"
version="${VERSION:?set VERSION}"
base_url="${BASE_URL:?set BASE_URL}"
notes_url="${NOTES_URL:-}"

artifacts='[]'
for archive in "$dir"/*.tar.gz "$dir"/*.zip; do
  [ -e "$archive" ] || continue
  file="$(basename "$archive")"
  # rama-<version>-<arch>-<os>.<ext>, as package.sh names it.
  target="${file#*-${version}-}"
  target="${target%.tar.gz}"
  target="${target%.zip}"
  # Files are named <arch>-<os> (package.sh), but a running build matches
  # its full triple, so that is what the manifest declares.
  case "$target" in
    *-linux) target="${target%-linux}-unknown-linux-gnu" ;;
    *-macos) target="${target%-macos}-apple-darwin" ;;
    *-windows) target="${target%-windows}-pc-windows-msvc" ;;
  esac
  artifacts="$(jq \
    --arg target "$target" \
    --arg url "${base_url}/${file}" \
    --argjson size "$(wc -c < "$archive")" \
    --arg sha256 "$(cat "${archive}.sha256")" \
    '. + [{target: $target, url: $url, size: $size, sha256: $sha256}]' \
    <<<"$artifacts")"
done

jq -n \
  --arg version "$version" \
  --arg notes "$notes_url" \
  --arg published_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --argjson artifacts "$artifacts" \
  '{version: $version, notes: (if $notes == "" then null else $notes end),
    published_at: $published_at, artifacts: $artifacts}
   | with_entries(select(.value != null))'
