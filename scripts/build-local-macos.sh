#!/usr/bin/env bash
# Build fig_desktop + q_cli with local-only autocomplete (no Amazon login at runtime).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export FIG_LOCAL_ONLY=1
export Q_DISABLE_TELEMETRY=1
export VITE_LOCAL_SPECS_ONLY=1
export VITE_PRODUCT_NAME=figlocal
export AMAZON_Q_BUILD_VARIANT=minimal

echo "==> Node packages (autocomplete UI chain)"
if ! command -v pnpm >/dev/null; then
  echo "Install pnpm first (brew install pnpm)"
  exit 1
fi
pnpm install
pnpm turbo build --filter=@amzn/autocomplete-app...

echo "==> Rust (fig_desktop + q_cli, local-only feature)"
if ! command -v cargo >/dev/null; then
  echo "Install Rust via rustup"
  exit 1
fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
cargo build -p fig_desktop -p q_cli --release --features local-only
cargo build -p figterm --release

echo ""
echo "Done."
echo "  CLI:     $ROOT/target/release/q"
echo "  Desktop: $ROOT/target/release/q_desktop (or fig_desktop binary name — check target/release/)"
ls -1 "$ROOT/target/release/" | grep -E '^(q|q_desktop|fig_desktop)$' || ls "$ROOT/target/release/" | head -5
