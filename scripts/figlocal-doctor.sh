#!/usr/bin/env bash
set -euo pipefail
export FIG_LOCAL_ONLY=1 Q_DISABLE_TELEMETRY=1
export PATH="${HOME}/.local/bin:${PATH}"
APP="${HOME}/Applications/figlocal.app"
BIN="${APP}/Contents/MacOS"

if ! pgrep -f "${BIN}/fig_desktop" >/dev/null 2>&1; then
  echo "FigLocal NOT running. Run: figlocal"
  exit 1
fi
echo "OK FigLocal process running"

if ! "${BIN}/q" debug accessibility 2>/dev/null | grep -q 'true'; then
  echo "Accessibility still false for fig_desktop."
  echo "Run: ${APP%/FigLocal.app}/../open-source/amazon-q-developer-cli-autocomplete/scripts/open-accessibility.sh"
  echo "Or: FIG_LOCAL_ONLY=1 q debug accessibility prompt"
  exit 1
fi
echo "OK Accessibility enabled"

if [[ -z "${Q_TERM:-}" ]]; then
  echo "This shell has no qterm (Q_TERM empty). Run: exec zsh"
  echo "Best test: macOS Terminal.app, not Cursor integrated terminal."
  exit 1
fi
echo "OK qterm active (Q_TERM=$Q_TERM)"
echo "Try typing: git␠"
