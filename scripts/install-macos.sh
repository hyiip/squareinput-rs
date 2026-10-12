#!/bin/bash
# Build Square Input for macOS and install it into ~/Library/Input Methods.
#
#   scripts/install-macos.sh
#
# 1. Builds squareinput-imk for Apple silicon and Intel and joins them with
#    lipo, bundles the .app (Info.plist, menu icon) and signs it ad hoc:
#    Apple silicon only loads signed code.
# 2. Rebuilds the data directory (~/Library/Application Support/squareinput)
#    with squareinput-data when the harvested tables are in local/. Otherwise
#    copy codes.tsv, punct.tsv and homophones.tsv (and promotions.tsv, to keep
#    the auto-promotions) there from another machine's data directory.
# 3. Replaces the installed .app and quits the running input method; macOS
#    starts the new one when it is next used.
#
# First install: log out and back in, then add "Square Input" in System
# Settings > Keyboard > Input Sources (Chinese, Traditional).
# Errors go to ~/Library/Logs/squareinput/squareinput.log.
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="Square Input"
BIN=squareinput-imk
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
INSTALL_DIR="$HOME/Library/Input Methods"
DATA_DIR="$HOME/Library/Application Support/squareinput"

binaries=()
for target in "${TARGETS[@]}"; do
    rustup target add "$target" >/dev/null
    cargo build --release -p "$BIN" --target "$target"
    binaries+=("target/$target/release/$BIN")
done

app="target/macos/$APP_NAME.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/$BIN" "${binaries[@]}"
cp squareinput-imk/Info.plist "$app/Contents/Info.plist"
cp squareinput-imk/resources/menu.pdf "$app/Contents/Resources/"
# Extended attributes (e.g. from iCloud folders) make codesign refuse the bundle.
xattr -cr "$app"
codesign --force --deep --sign - "$app"

if [[ -f local/codes.tsv && -f local/punct.tsv ]]; then
    cargo build --release -p squareinput --bin squareinput-data
    target/release/squareinput-data
elif [[ ! -f "$DATA_DIR/codes.tsv" ]]; then
    echo "note: no tables in local/ or $DATA_DIR; copy codes.tsv, punct.tsv and homophones.tsv there" >&2
fi

mkdir -p "$INSTALL_DIR"
rm -rf "$INSTALL_DIR/$APP_NAME.app"
cp -R "$app" "$INSTALL_DIR/"
killall "$BIN" 2>/dev/null || true
echo "installed $INSTALL_DIR/$APP_NAME.app"
