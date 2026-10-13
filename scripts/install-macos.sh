#!/bin/bash
# Build Square Input for macOS and install it into ~/Library/Input Methods.
#
#   scripts/install-macos.sh                build, refresh the data, install
#   scripts/install-macos.sh --data-only    only refresh the data
#   scripts/install-macos.sh --bundle-only  only build the .app, in target/macos
#
# 1. Builds squareinput-imk for Apple silicon and Intel and joins them with
#    lipo, bundles the .app (Info.plist, menu icon) and signs it ad hoc:
#    Apple silicon only loads signed code. A release ships the .app prebuilt
#    in bin/ and is not built, so it needs neither Rust nor Xcode.
# 2. Refreshes the data directory (~/Library/Application Support/squareinput,
#    or $SQUAREINPUT_DATA). Where local/ holds the harvested tables they are
#    rebuilt with squareinput-data. Elsewhere they are copied from tsv/:
#    codes.tsv, punct.tsv, homophones.tsv and associations.tsv, as the
#    Windows install script leaves them in its tsv\. The auto-promotion log is
#    made by typing here; a promotions.tsv put in tsv/ by hand only seeds a
#    new data directory, so auto-promotions learnt here are never overwritten.
# 3. Replaces the installed .app and quits the running input method; macOS
#    starts the new one, with the new data, when it is next used.
# 4. Registers and enables the input source (squareinput-imk --register), so
#    there is no need to log out; a first install also switches to it.
#    macOS 26 lets only System Settings enable a third-party input method, so
#    there the script opens it at Keyboard for "Square Input" to be added
#    (Input Sources > Edit > +, Chinese, Traditional). If registering fails,
#    log out and back in, then add it there.
# Errors go to ~/Library/Logs/squareinput/squareinput.log.
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="Square Input"
BIN=squareinput-imk
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
INSTALL_DIR="$HOME/Library/Input Methods"
DATA_DIR="${SQUAREINPUT_DATA:-$HOME/Library/Application Support/squareinput}"

install_data() {
    if [[ -f local/codes.tsv && -f local/punct.tsv ]]; then
        cargo build --release -p squareinput --bin squareinput-data
        target/release/squareinput-data
    elif [[ -f tsv/codes.tsv && -f tsv/punct.tsv ]]; then
        mkdir -p "$DATA_DIR"
        cp tsv/codes.tsv tsv/punct.tsv "$DATA_DIR/"
        local optional
        for optional in homophones.tsv associations.tsv; do
            if [[ -f "tsv/$optional" ]]; then
                cp "tsv/$optional" "$DATA_DIR/"
            fi
        done
        if [[ -f tsv/promotions.tsv && ! -f "$DATA_DIR/promotions.tsv" ]]; then
            cp tsv/promotions.tsv "$DATA_DIR/"
        fi
        echo "copied the tables from tsv/ into $DATA_DIR"
    elif [[ ! -f "$DATA_DIR/codes.tsv" ]]; then
        echo "note: no tables in local/ or tsv/: put codes.tsv, punct.tsv, homophones.tsv and associations.tsv in tsv/" >&2
    fi
}

build_app() {
    local binaries=() target
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
    # Display names, including the input mode's, keyed by its ID.
    cp -R squareinput-imk/resources/*.lproj "$app/Contents/Resources/"
    # Extended attributes (e.g. from iCloud folders) make codesign refuse the bundle.
    xattr -cr "$app"
    codesign --force --deep --sign - "$app"
}

case "${1:-}" in
--data-only)
    install_data
    # The input method reads the tables once; restart it to pick them up.
    killall "$BIN" 2>/dev/null || true
    exit 0
    ;;
--bundle-only)
    build_app
    echo "built $app"
    exit 0
    ;;
esac

if [[ -d "bin/$APP_NAME.app" ]]; then
    app="bin/$APP_NAME.app"
else
    build_app
fi

install_data

mkdir -p "$INSTALL_DIR"
rm -rf "$INSTALL_DIR/$APP_NAME.app"
cp -R "$app" "$INSTALL_DIR/"
# A downloaded release is quarantined, and macOS will not run an ad hoc
# signed app that is.
xattr -cr "$INSTALL_DIR/$APP_NAME.app"
killall "$BIN" 2>/dev/null || true
echo "installed $INSTALL_DIR/$APP_NAME.app"

# Register the input source and enable it, so a first install needs no
# logging out; on a first install it also becomes the current input source.
# Open applications reload the input source list from these caches, and
# switching to a newly registered input method can crash them; drop them so
# they rebuild it.
cache="$(getconf DARWIN_USER_CACHE_DIR 2>/dev/null || true)"
if [[ -n "$cache" ]]; then
    rm -f "$cache"com.apple.IntlDataCache.le* "$cache"*/com.apple.IntlDataCache.le* 2>/dev/null || true
fi
status=0
"$INSTALL_DIR/$APP_NAME.app/Contents/MacOS/$BIN" --register || status=$?
case $status in
0) ;;
3)
    # Exit code NEEDS_USER: registered, but only the user can enable it.
    open "x-apple.systempreferences:com.apple.Keyboard-Settings.extension" 2>/dev/null || true
    echo "to enable it: in System Settings > Keyboard > Input Sources, press Edit, then +, choose Chinese, Traditional, then Square Input, and add it" >&2
    ;;
*)
    echo "could not register the input source: log out and back in, then add Square Input in System Settings > Keyboard > Input Sources" >&2
    ;;
esac