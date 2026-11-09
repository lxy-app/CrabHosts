#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_DIR="$ROOT/dist/CrabHosts.app"
CONTENTS="$APP_DIR/Contents"
IDENTITY="${CRABHOSTS_CODESIGN_IDENTITY:-}"
TEAM_ID="${CRABHOSTS_TEAM_ID:-}"

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "macOS packaging is supported only on Darwin." >&2
    exit 1
fi
if [[ -z "$IDENTITY" || -z "$TEAM_ID" ]]; then
    cat >&2 <<'EOF'
Set both signing variables before packaging, for example:
  export CRABHOSTS_CODESIGN_IDENTITY="Apple Development: Your Name (TEAMID)"
  export CRABHOSTS_TEAM_ID="TEAMID"

The Helper and its clients require a real Apple signing identity; ad-hoc signing is not accepted.
EOF
    exit 2
fi
if [[ ! "$TEAM_ID" =~ ^[A-Z0-9]{10}$ ]]; then
    echo "CRABHOSTS_TEAM_ID must be a 10-character Apple Team ID." >&2
    exit 2
fi
CERT_SUBJECT="$(security find-certificate -c "$IDENTITY" -p | openssl x509 -noout -subject)"
CERT_TEAM_ID="$(printf '%s\n' "$CERT_SUBJECT" | sed -E 's#(^|.*/)OU=([^,/]+).*#\2#')"
if [[ "$CERT_TEAM_ID" != "$TEAM_ID" ]]; then
    echo "CRABHOSTS_TEAM_ID=$TEAM_ID does not match certificate subject OU=$CERT_TEAM_ID." >&2
    echo "Certificate subject: $CERT_SUBJECT" >&2
    exit 2
fi

TIMESTAMP_ARGS=(--timestamp)
if [[ "${CRABHOSTS_CODESIGN_TIMESTAMP:-}" == "none" ]]; then
    TIMESTAMP_ARGS=(--timestamp=none)
fi

export MACOSX_DEPLOYMENT_TARGET=13.0
export CRABHOSTS_TEAM_ID
cargo build --release \
    -p crabhosts-desktop \
    -p crabhosts-cli \
    -p crabhosts-macos-helper

rm -rf "$APP_DIR"
mkdir -p \
    "$CONTENTS/MacOS" \
    "$CONTENTS/Resources" \
    "$CONTENTS/SharedSupport" \
    "$CONTENTS/Library/LaunchServices" \
    "$CONTENTS/Library/LaunchDaemons"
cp "$ROOT/target/release/crabhosts-desktop" "$CONTENTS/MacOS/CrabHosts"
# CrabHosts and crabhosts differ only by case and collide on default APFS volumes.
cp "$ROOT/target/release/crabhosts" "$CONTENTS/SharedSupport/crabhosts"
cp "$ROOT/target/release/crabhosts-macos-helper" \
    "$CONTENTS/Library/LaunchServices/dev.crabhosts.hosts-helper"
cp "$ROOT/apps/desktop/macos/Info.plist" "$CONTENTS/Info.plist"
cp "$ROOT/apps/desktop/macos/CrabHosts.icns" "$CONTENTS/Resources/CrabHosts.icns"
cp "$ROOT/apps/macos-helper/launchd/dev.crabhosts.hosts-helper.plist" \
    "$CONTENTS/Library/LaunchDaemons/dev.crabhosts.hosts-helper.plist"
if [[ "$CONTENTS/MacOS/CrabHosts" -ef "$CONTENTS/SharedSupport/crabhosts" ]]; then
    echo "Desktop and CLI unexpectedly resolve to the same file." >&2
    exit 1
fi
chmod 755 \
    "$CONTENTS/MacOS/CrabHosts" \
    "$CONTENTS/SharedSupport/crabhosts" \
    "$CONTENTS/Library/LaunchServices/dev.crabhosts.hosts-helper"

sign_executable() {
    local path="$1"
    local identifier="$2"
    codesign --force --options runtime --sign "$IDENTITY" \
        --identifier "$identifier" "${TIMESTAMP_ARGS[@]}" "$path"
}

sign_executable "$CONTENTS/Library/LaunchServices/dev.crabhosts.hosts-helper" \
    "dev.crabhosts.hosts-helper"
sign_executable "$CONTENTS/SharedSupport/crabhosts" "dev.crabhosts.cli"
sign_executable "$APP_DIR" "dev.crabhosts.desktop"

verify_executable() {
    local path="$1"
    local identifier="$2"
    codesign --verify --strict --verbose=2 \
        -R="identifier \"$identifier\" and anchor apple generic and certificate leaf[subject.OU] = \"$TEAM_ID\"" \
        "$path"
}

verify_executable "$CONTENTS/Library/LaunchServices/dev.crabhosts.hosts-helper" \
    "dev.crabhosts.hosts-helper"
verify_executable "$CONTENTS/SharedSupport/crabhosts" "dev.crabhosts.cli"
verify_executable "$APP_DIR" "dev.crabhosts.desktop"
plutil -lint "$CONTENTS/Info.plist" \
    "$CONTENTS/Library/LaunchDaemons/dev.crabhosts.hosts-helper.plist"

cat <<EOF
Created and signed: $APP_DIR
CLI for AI Harness: $CONTENTS/SharedSupport/crabhosts

Opening the app does not install the Helper automatically. Use the explicit Helper install
action in the desktop UI, then approve the launchd daemon in macOS System Settings if asked.
EOF
