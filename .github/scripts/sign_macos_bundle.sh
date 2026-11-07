#!/usr/bin/env bash
set -euo pipefail

APP="${1:?usage: sign_macos_bundle.sh /path/to/Capin.app}"
IDENTIFIER="io.github.fynhq.capin"

if [ ! -d "$APP/Contents/MacOS" ]; then
  echo "not a macOS app bundle: $APP" >&2
  exit 2
fi
if ! command -v codesign >/dev/null 2>&1; then
  echo "codesign is unavailable" >&2
  exit 2
fi

# Sign nested executables first, then the bundle. SMAppService records the
# code-signing identity, so this identifier must stay stable across builds.
while IFS= read -r -d '' binary; do
  codesign --force --sign - "$binary"
done < <(find "$APP/Contents/MacOS" -type f -print0)

INFO_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist")"
if [ "$INFO_ID" != "$IDENTIFIER" ]; then
  echo "Info.plist CFBundleIdentifier is '$INFO_ID', expected '$IDENTIFIER'" >&2
  exit 1
fi

codesign --force --sign - --identifier "$IDENTIFIER" "$APP"
codesign --verify --deep --strict "$APP"

ACTUAL_ID="$(codesign -dv --verbose=4 "$APP" 2>&1 | sed -n 's/^Identifier=//p' | head -n 1)"
if [ "$ACTUAL_ID" != "$IDENTIFIER" ]; then
  echo "signed identifier is '$ACTUAL_ID', expected '$IDENTIFIER'" >&2
  exit 1
fi

echo "signed $APP with ad-hoc identity $IDENTIFIER"
