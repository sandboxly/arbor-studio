#!/bin/sh
# Wraps the arbor_studio program into a double-clickable "ArborStudio.app".
#
#   macos/bundle.sh <program> <output folder> [version]
#
# With MACOS_SIGN_IDENTITY set (a "Developer ID Application: ..." certificate in the keychain) the
# app is signed for distribution (hardened runtime, secure timestamp, ready for notarisation);
# without it, it gets an ad-hoc signature, which runs on this Mac and, after the usual
# "allow anyway" step, on others.
set -eu

BIN="$1"
OUT="$2"
VERSION="${3:-0.2.0}"
BUNDLE_ID="${MACOS_BUNDLE_ID:-io.github.arbor-studio}"
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="$OUT/ArborStudio.app"

if [ -e "$APP" ]; then
    echo "$APP already exists; pick an empty output folder" >&2
    exit 1
fi
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/arbor_studio"
chmod +x "$APP/Contents/MacOS/arbor_studio"
sed -e "s/__VERSION__/$VERSION/g" -e "s/__BUNDLE_ID__/$BUNDLE_ID/g" "$HERE/Info.plist" > "$APP/Contents/Info.plist"
if [ -f "$HERE/AppIcon.icns" ]; then
    cp "$HERE/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
    /usr/libexec/PlistBuddy -c "Add :CFBundleIconFile string AppIcon" "$APP/Contents/Info.plist"
fi

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    codesign --force --options runtime --timestamp --sign "$MACOS_SIGN_IDENTITY" "$APP"
else
    codesign --force --sign - "$APP"
fi
codesign --verify --strict "$APP"
echo "$APP"
