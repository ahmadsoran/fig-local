#!/usr/bin/env bash
# Start fig_desktop from the figlocal bundle with retries and socket readiness.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DESKTOP_BIN="${SCRIPT_DIR}/fig_desktop"
export FIG_LOCAL_ONLY=1
export Q_DISABLE_TELEMETRY=1

LOG="${HOME}/Library/Logs/figlocal.log"
SOCKET_DIR="${TMPDIR:-/tmp}/cwrun"
SOCKET="${SOCKET_DIR}/desktop.sock"
MAX_ATTEMPTS=3
READY_POLLS=40
POLL_SECS=0.5

mkdir -p "$(dirname "$LOG")" "${SOCKET_DIR}"

desktop_args=(--ignore-immediate-update "$@")
if [[ "${FIGLOCAL_NO_DASHBOARD:-0}" == "1" ]]; then
  desktop_args+=(--no-dashboard)
fi

desktop_running() {
  pgrep -x fig_desktop >/dev/null 2>&1
}

socket_ready() {
  [[ -S "${SOCKET}" ]]
}

notify() {
  local title="$1"
  local message="$2"
  osascript -e "display notification \"${message}\" with title \"${title}\"" 2>/dev/null || true
}

alert_fail() {
  osascript -e 'display alert "figlocal failed to start" message "See ~/Library/Logs/figlocal.log" as critical' 2>/dev/null || true
}

if desktop_running || socket_ready; then
  notify "figlocal" "figlocal is already running."
  exit 0
fi

for attempt in $(seq 1 "${MAX_ATTEMPTS}"); do
  {
    echo "==> launch attempt ${attempt} at $(date)"
  } >>"${LOG}"

  nohup "${DESKTOP_BIN}" "${desktop_args[@]}" >>"${LOG}" 2>&1 &
  disown || true

  for _ in $(seq 1 "${READY_POLLS}"); do
    sleep "${POLL_SECS}"
    if socket_ready; then
      notify "figlocal" "figlocal is running. Run: exec zsh"
      exit 0
    fi
    if ! desktop_running; then
      break
    fi
  done

  if desktop_running; then
    notify "figlocal" "figlocal started. Run: exec zsh"
    exit 0
  fi

  pkill -x fig_desktop 2>/dev/null || true
  sleep 1
done

alert_fail
exit 1