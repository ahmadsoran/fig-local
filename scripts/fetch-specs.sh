#!/usr/bin/env bash
# Vendor @withfig/autocomplete build/ into vendor/specs for offline install
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/vendor/specs"
SPEC_VER="${WITHFIG_SPECS_VERSION:-2.692.3}"

if [[ -f "$DEST/index.json" ]]; then
  echo "Specs already at $DEST"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"
echo "==> Downloading @withfig/autocomplete@$SPEC_VER"
npm pack "@withfig/autocomplete@$SPEC_VER" >/dev/null
tar -xzf withfig-autocomplete-*.tgz
mkdir -p "$DEST"
rsync -a package/build/ "$DEST/"
echo "==> Installed $(wc -l < "$DEST/index.json" | tr -d ' ') spec index entries under $DEST"
