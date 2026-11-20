#!/bin/sh
# Puts Horizon NX Companion in ~/.local/bin and in the applications menu (no root needed).
# Uninstall: ./install.sh --remove
set -eu
cd "$(dirname "$0")"
bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
if [ "${1:-}" = "--remove" ]; then
  rm -f "$bin/horizon-nx-companion" "$data/applications/horizon-nx-companion.desktop" \
    "$data/icons/hicolor/256x256/apps/horizon-nx-companion.png"
  echo "Removed."
  exit 0
fi
mkdir -p "$bin" "$data/applications" "$data/icons/hicolor/256x256/apps"
cp horizon-nx-companion "$bin/"
chmod +x "$bin/horizon-nx-companion"
cp horizon-nx-companion.png "$data/icons/hicolor/256x256/apps/"
sed "s|^Exec=.*|Exec=$bin/horizon-nx-companion|" horizon-nx-companion.desktop \
  > "$data/applications/horizon-nx-companion.desktop"
echo "Installed: find Horizon NX Companion in your applications, or run $bin/horizon-nx-companion"
