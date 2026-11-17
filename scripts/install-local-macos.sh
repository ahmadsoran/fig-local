#!/usr/bin/env bash
# Install local-only build into ~/Applications/figlocal.app and ~/.local/bin
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$HOME/Applications/figlocal.app"
BIN="$HOME/.local/bin"
LEGACY_APP="$HOME/Applications/FigLocal.app"

[[ -x "$ROOT/target/release/fig_desktop" ]] || {
  echo "Run ./scripts/build-local-macos.sh first"
  exit 1
}
[[ -x "$ROOT/target/release/figterm" ]] || {
  echo "Missing target/release/figterm — run build-local-macos.sh (builds qterm)"
  exit 1
}
[[ -d "$ROOT/packages/autocomplete-app/dist" ]] || {
  echo "Missing packages/autocomplete-app/dist — run build-local-macos.sh"
  exit 1
}

"$ROOT/scripts/fetch-specs.sh"

echo "==> App bundle at $APP"
rm -rf "$APP"
[[ -d "$LEGACY_APP" ]] && rm -rf "$LEGACY_APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/autocomplete" "$APP/Contents/Resources/specs" "$APP/Contents/Resources/status"
cp "$ROOT/target/release/fig_desktop" "$APP/Contents/MacOS/fig_desktop"
cp "$ROOT/target/release/q_cli" "$APP/Contents/MacOS/q"
cp "$ROOT/target/release/figterm" "$APP/Contents/MacOS/qterm"
cp "$ROOT/scripts/figlocal-launcher.sh" "$APP/Contents/MacOS/figlocal"
chmod +x "$APP/Contents/MacOS/"*
rsync -a "$ROOT/packages/autocomplete-app/dist/" "$APP/Contents/Resources/autocomplete/"
rsync -a "$ROOT/vendor/specs/" "$APP/Contents/Resources/specs/"
rsync -a "$ROOT/resources/figlocal/status/" "$APP/Contents/Resources/status/"

ICON_SRC="$ROOT/resources/figlocal"
ICONSET="$APP/Contents/Resources/figlocal.iconset"
ICNS="$APP/Contents/Resources/figlocal.icns"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"

build_icon_png() {
  local size="$1"
  local out="$2"
  if command -v rsvg-convert >/dev/null 2>&1; then
    rsvg-convert -w "$size" -h "$size" "$ICON_SRC/icon.svg" -o "$out"
  elif command -v qlmanage >/dev/null 2>&1; then
    qlmanage -t -s "$size" -o "$(dirname "$out")" "$ICON_SRC/icon.svg" >/dev/null 2>&1 || true
    local gen="$(dirname "$out")/$(basename "$ICON_SRC/icon.svg").png"
    [[ -f "$gen" ]] && mv "$gen" "$out"
  fi
}

FALLBACK_ICON="$ROOT/crates/fig_desktop/icons"
if [[ -f "$ICON_SRC/icon.svg" ]]; then
  build_icon_png 16 "$ICONSET/icon_16x16.png"
  build_icon_png 32 "$ICONSET/icon_16x16@2x.png"
  build_icon_png 32 "$ICONSET/icon_32x32.png"
  build_icon_png 64 "$ICONSET/icon_32x32@2x.png"
  build_icon_png 128 "$ICONSET/icon_128x128.png"
  build_icon_png 256 "$ICONSET/icon_128x128@2x.png"
  build_icon_png 256 "$ICONSET/icon_256x256.png"
  build_icon_png 512 "$ICONSET/icon_256x256@2x.png"
  build_icon_png 512 "$ICONSET/icon_512x512.png"
  build_icon_png 1024 "$ICONSET/icon_512x512@2x.png"
fi

if [[ ! -f "$ICONSET/icon_512x512.png" ]]; then
  ICON_SRC="$FALLBACK_ICON"
  cp "$ICON_SRC/32x32.png" "$ICONSET/icon_16x16@2x.png"
  cp "$ICON_SRC/128x128.png" "$ICONSET/icon_128x128.png"
  cp "$ICON_SRC/128x128@2x.png" "$ICONSET/icon_256x256.png"
  cp "$ICON_SRC/128x128@2x.png" "$ICONSET/icon_128x128@2x.png"
  cp "$ICON_SRC/icon.png" "$ICONSET/icon_512x512.png" 2>/dev/null || cp "$ICON_SRC/128x128@2x.png" "$ICONSET/icon_512x512.png"
fi

iconutil -c icns "$ICONSET" -o "$ICNS"
rm -rf "$ICONSET"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>fig_desktop</string>
  <key>CFBundleIdentifier</key>
  <string>dev.figlocal.autocomplete</string>
  <key>CFBundleName</key>
  <string>figlocal</string>
  <key>CFBundleDisplayName</key>
  <string>figlocal</string>
  <key>CFBundleIconFile</key>
  <string>figlocal</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>1.0.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>LSUIElement</key>
  <false/>
  <key>LSEnvironment</key>
  <dict>
    <key>FIG_LOCAL_ONLY</key>
    <string>1</string>
    <key>Q_DISABLE_TELEMETRY</key>
    <string>1</string>
  </dict>
</dict>
</plist>
PLIST
printf 'APPL????' > "$APP/Contents/PkgInfo"

echo "==> Ad-hoc sign app bundle (stable Accessibility entry in System Settings)"
codesign --force --deep -s - "$APP" 2>/dev/null || echo "Warning: codesign failed — grant Accessibility manually"

cat > "$APP/Contents/MacOS/figlocal-cli" <<'OPEN'
#!/usr/bin/env bash
export FIGLOCAL_NO_DASHBOARD=1
exec "${HOME}/Applications/figlocal.app/Contents/MacOS/figlocal"
OPEN
chmod +x "$APP/Contents/MacOS/figlocal-cli"

echo "==> CLI symlinks in $BIN"
mkdir -p "$BIN"
ln -sf "$APP/Contents/MacOS/q" "$BIN/q"
ln -sf "$APP/Contents/MacOS/qterm" "$BIN/qterm"
ln -sf "$APP/Contents/MacOS/fig_desktop" "$BIN/fig_desktop"
ln -sf "$APP/Contents/MacOS/figlocal-cli" "$BIN/figlocal"

ZSHRC="${ZDOTDIR:-$HOME}/.zshrc"
export FIG_LOCAL_ONLY=1
export Q_DISABLE_TELEMETRY=1
Q_SHELL_DIR="$HOME/Library/Application Support/amazon-q/shell"
mkdir -p "$Q_SHELL_DIR"
for f in zshrc.pre.zsh zshrc.post.zsh; do
  path="$Q_SHELL_DIR/$f"
  touch "$path"
  grep -q 'FIG_LOCAL_ONLY' "$path" 2>/dev/null || {
    printf '%s\n' 'export FIG_LOCAL_ONLY=1' 'export Q_DISABLE_TELEMETRY=1' >> "$path"
  }
done
echo "==> Shell integrations (amazon-q pre/post in $ZSHRC)"
"$APP/Contents/MacOS/q" integrations install dotfiles zsh || true

pkill -x fig_desktop 2>/dev/null || true
"$APP/Contents/MacOS/figlocal"

echo ""
echo "Installed figlocal.app"
echo "  Double-click ~/Applications/figlocal.app  # status window + service"
echo "  figlocal          # background start (no main window)"
echo "  q launch          # same as opening the app"
echo "  exec zsh          # reload shell in terminals"
echo "  q doctor"