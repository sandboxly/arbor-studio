#!/bin/sh
# Puts "ArborStudio.app" into a disk image: open it, drag the app onto the Applications folder.
#
#   macos/dmg.sh <ArborStudio.app> <output .dmg>
#
# With MACOS_SIGN_IDENTITY set the disk image is signed too (it can then be notarised and
# stapled as a whole).
set -eu

APP="$1"
DMG="$2"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

ditto "$APP" "$STAGE/ArborStudio.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "ArborStudio" -srcfolder "$STAGE" -fs HFS+ -format UDZO -imagekey zlib-level=9 -ov "$DMG" >/dev/null

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$DMG"
fi
hdiutil verify "$DMG" >/dev/null
echo "$DMG"
