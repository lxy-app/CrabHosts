#!/usr/bin/env bash
set -euo pipefail

# Builds the signed .app via scripts/package-macos.sh, then produces a
# notarized, stapled disk image ready for distribution.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_DIR="$ROOT/dist/CrabHosts.app"
IDENTITY="${CRABHOSTS_CODESIGN_IDENTITY:-}"
TEAM_ID="${CRABHOSTS_TEAM_ID:-}"
NOTARY_PROFILE="${CRABHOSTS_NOTARY_PROFILE:-CrabHosts}"

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "macOS packaging is supported only on Darwin." >&2
    exit 1
fi
if [[ -z "$IDENTITY" || -z "$TEAM_ID" ]]; then
    cat >&2 <<'EOF'
Set both signing variables before packaging, for example:
  export CRABHOSTS_CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
  export CRABHOSTS_TEAM_ID="TEAMID"

Notarization also needs a stored notarytool profile (default keychain profile name: CrabHosts):
  xcrun notarytool store-credentials CrabHosts --team-id TEAMID
EOF
    exit 2
fi
if [[ ! "$IDENTITY" =~ ^Developer\ ID\ Application: ]]; then
    echo "Notarized distribution requires a Developer ID Application identity, got: $IDENTITY" >&2
    exit 2
fi

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
    "$ROOT/apps/desktop/macos/Info.plist")"
DMG="$ROOT/dist/CrabHosts-$VERSION-macos-arm64.dmg"
NOTARIZATION_ZIP="$ROOT/dist/CrabHosts-$VERSION-macos-arm64-notarization.zip"

submit_and_wait() {
    local path="$1"
    local output
    output="$(xcrun notarytool submit "$path" --keychain-profile "$NOTARY_PROFILE" --wait 2>&1)" || {
        printf '%s\n' "$output" >&2
        echo "notarytool submit failed for $path" >&2
        exit 1
    }
    printf '%s\n' "$output"
    if ! printf '%s\n' "$output" | grep -q 'status: Accepted'; then
        echo "Apple did not accept $path; see the log above or run: xcrun notarytool log <id> --keychain-profile $NOTARY_PROFILE" >&2
        exit 1
    fi
}

# 1. Build and sign the app (helper, CLI, and outer app).
CRABHOSTS_CODESIGN_IDENTITY="$IDENTITY" \
CRABHOSTS_TEAM_ID="$TEAM_ID" \
    "$ROOT/scripts/package-macos.sh"

codesign --verify --strict --verbose=2 "$APP_DIR"

# 2. Notarize the app itself so the copy inside the disk image keeps an
#    attached ticket and passes Gatekeeper offline.
rm -f "$NOTARIZATION_ZIP"
ditto -c -k --keepParent "$APP_DIR" "$NOTARIZATION_ZIP"
submit_and_wait "$NOTARIZATION_ZIP"
xcrun stapler staple "$APP_DIR"
xcrun stapler validate "$APP_DIR"

# 3. Build the disk image with a drag-to-Applications layout.
STAGE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/crabhosts-dmg.XXXXXX")"
MOUNT_DIR=""
cleanup() {
    if [[ -n "$MOUNT_DIR" ]]; then
        hdiutil detach "$MOUNT_DIR" -quiet >/dev/null 2>&1 || true
    fi
    rm -rf "$STAGE_DIR"
}
trap cleanup EXIT

cp -R "$APP_DIR" "$STAGE_DIR/CrabHosts.app"
ln -s /Applications "$STAGE_DIR/Applications"

rm -f "$DMG"
echo "Creating disk image: $DMG"
hdiutil create -volname "CrabHosts $VERSION" \
    -srcfolder "$STAGE_DIR" -ov -format UDZO "$DMG"
codesign --force --timestamp --sign "$IDENTITY" "$DMG"

# 4. Notarize the disk image, then staple the ticket to the DMG.
submit_and_wait "$DMG"
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"

# 5. Verify Gatekeeper acceptance for both the DMG and the app inside it.
spctl --assess --type open --context context:primary-signature --verbose=4 "$DMG"
MOUNT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/crabhosts-mount.XXXXXX")"
hdiutil attach "$DMG" -mountpoint "$MOUNT_DIR" -nobrowse -quiet
spctl --assess --type execute --verbose=4 "$MOUNT_DIR/CrabHosts.app"
codesign --verify --strict --verbose=2 "$MOUNT_DIR/CrabHosts.app"
xcrun stapler validate "$MOUNT_DIR/CrabHosts.app"
shasum -a 256 "$DMG"

cat <<EOF
Created notarized installer: $DMG
Notarization submission: $NOTARIZATION_ZIP
EOF
