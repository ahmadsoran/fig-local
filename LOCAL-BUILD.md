# Local-only build (Fig-style autocomplete)

Fork patches for **option B**: autocomplete only — no Amazon login, no chat/agent/MCP, no auto-updates, no telemetry, specs loaded from the bundled app (not `specs.q.us-east-1.amazonaws.com`).

## What changed

| Area | Behavior |
|------|----------|
| `local-only` Cargo feature + `FIG_LOCAL_ONLY=1` | Central switch (`fig_util::local_only`) |
| `fig_auth` | `is_logged_in()` always true in local mode |
| `fig_desktop` | No dashboard webview; autocomplete enabled without login; no update checks |
| `q_cli` | Blocks `chat`, `mcp`, `agent`, `inline`, `translate`; `q` with no args prints help; `q launch` starts app only |
| `fig_telemetry` | Disabled in local mode |
| `autocomplete-parser` | With `VITE_LOCAL_SPECS_ONLY=1`, never hits Amazon specs CDN |

Amazon SDK crates still **link** into the binary; they are unused on the autocomplete path. Removing them completely would be a larger refactor.

## Build (macOS)

```bash
./scripts/build-local-macos.sh
```

Requires: Xcode CLT, Node 22+, pnpm, Rust stable (edition 2024), protobuf.

Outputs under `build/` (see script log).

## Install (macOS, from source build)

After `./scripts/build-local-macos.sh`:

```bash
./scripts/install-local-macos.sh
```

Add `~/.local/bin` to PATH, wire shell (see script output), then start **figlocal** from Applications or `open -a figlocal`.

**Do not use `q launch`** for this fork — it tries to open the official `Amazon Q.app` by bundle id. Use **FigLocal** or run `fig_desktop` from the mini app bundle.

## Run (dev, without install script)

```bash
export FIG_LOCAL_ONLY=1
export Q_DISABLE_TELEMETRY=1
export PATH="$HOME/.local/bin:$PATH"
open -g -a figlocal --args --ignore-immediate-update
exec zsh
```

## License

Upstream is MIT OR Apache-2.0. This fork keeps the same licenses; you maintain the derivative.
