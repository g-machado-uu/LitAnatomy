#!/usr/bin/env bash
# Install LitAnatomy for the current user on Linux (no root needed).
#
#   ./install.sh               build and install (or update)
#   ./install.sh --uninstall   remove the binary, launcher and icons (keeps your library)
#
# Installs:
#   ~/.local/bin/litanatomy
#   ~/.local/share/applications/litanatomy.desktop   (app menus, Rofi drun)
#   ~/.local/share/icons/hicolor/*/apps/litanatomy.{svg,png}
#
# Your library stays where it is: by default ~/.local/share/litanatomy/library.litanatomy
# (Settings › Library file shows and changes it).

set -euo pipefail

APP=litanatomy
SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
APPS_DIR="$DATA_HOME/applications"
ICON_DIR="$DATA_HOME/icons/hicolor"

uninstall=0
case "${1:-}" in
    "") ;;
    --uninstall) uninstall=1 ;;
    -h|--help) sed -n '2,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "error: unknown option '$1'" >&2; exit 1 ;;
esac

refresh_caches() {
    command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$ICON_DIR" 2>/dev/null || true
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$APPS_DIR" 2>/dev/null || true
}

if [[ $uninstall -eq 1 ]]; then
    rm -f "$BIN_DIR/$APP" "$APPS_DIR/$APP.desktop"
    rm -f "$ICON_DIR"/scalable/apps/$APP.svg "$ICON_DIR"/*/apps/$APP.png
    refresh_caches
    echo "Removed LitAnatomy. Your library and settings were left in place."
    exit 0
fi

# --- Build -----------------------------------------------------------------
binary="$SRC_DIR/target/release/$APP"
if command -v cargo >/dev/null; then
    echo "==> Building release binary"
    cargo build --release --manifest-path "$SRC_DIR/Cargo.toml"
elif [[ ! -x "$binary" ]]; then
    echo "error: cargo not found and no prebuilt $binary" >&2
    exit 1
fi

# --- Install ---------------------------------------------------------------
echo "==> Installing"
install -Dm755 "$binary" "$BIN_DIR/$APP"
install -Dm644 "$SRC_DIR/packaging/$APP.svg" "$ICON_DIR/scalable/apps/$APP.svg"
install -Dm644 "$SRC_DIR/packaging/$APP.png" "$ICON_DIR/256x256/apps/$APP.png"

# Absolute Exec path: some launchers' environments don't include ~/.local/bin.
mkdir -p "$APPS_DIR"
sed -e "s|^Exec=.*|Exec=$BIN_DIR/$APP|" "$SRC_DIR/packaging/$APP.desktop" > "$APPS_DIR/$APP.desktop"
chmod 644 "$APPS_DIR/$APP.desktop"
refresh_caches

cat <<EOF

LitAnatomy installed.

  Binary:    $BIN_DIR/$APP
  Launcher:  $APPS_DIR/$APP.desktop
  Window class (for window rules): $APP
EOF
case ":$PATH:" in
    *":$BIN_DIR:"*) echo; echo "Run 'litanatomy' from a terminal, or find LitAnatomy in your app menu." ;;
    *) echo; echo "$BIN_DIR is not on your PATH: start LitAnatomy from your app menu, or add it to PATH." ;;
esac
