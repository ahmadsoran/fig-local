#!/usr/bin/env bash
# Trigger macOS Accessibility prompt for FigLocal, then open Settings.
set -euo pipefail
APP="${HOME}/Applications/figlocal.app"
BIN="${APP}/Contents/MacOS/fig_desktop"

pkill -x fig_desktop 2>/dev/null || true
sleep 1

echo "Starting FigLocal to show the Accessibility trust dialog..."
FIG_LOCAL_ONLY=1 Q_DISABLE_TELEMETRY=1 "$BIN" --no-dashboard --ignore-immediate-update &
DESKTOP_PID=$!
sleep 2

open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"

osascript <<EOF
display dialog "FigLocal needs Accessibility.

1. If macOS showed a prompt, click Open System Settings and turn ON FigLocal.
2. If no prompt: in Accessibility click + and choose:
   ${BIN}
3. Quit FigLocal (pkill fig_desktop) and run: figlocal
4. In Terminal: exec zsh

After each rebuild, you may need to toggle Accessibility again." buttons {"OK"} default button "OK" with title "FigLocal Accessibility"
EOF

echo "FigLocal PID: $DESKTOP_PID (leave running while you enable Accessibility in Settings)"
