#!/bin/sh
set -eu

binary=$1
shift
# Cargo uses this runner for tests and CLI tools too.
if [ "${binary##*/}" != crabhosts-desktop ]; then
    exec "$binary" "$@"
fi

# Run inside a real bundle so Launch Services can give the Dock its icon.
# Keep the dev identity separate from the signed app that installs the helper.
root=$(cd "$(dirname "$0")/.." && pwd)
contents="$(dirname "$binary")/CrabHosts Dev.app/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources"
stage=$(mktemp -d "$contents/.stage.XXXXXX")
trap 'rm -rf "$stage"' EXIT
plutil -replace CFBundleIdentifier -string dev.crabhosts.desktop.dev \
    -o "$stage/Info.plist" "$root/apps/desktop/macos/Info.plist"
plutil -replace CFBundleDisplayName -string "CrabHosts Dev" "$stage/Info.plist"
cp "$root/apps/desktop/macos/CrabHosts.icns" "$stage/CrabHosts.icns"
ln "$binary" "$stage/CrabHosts"
mv -f "$stage/Info.plist" "$contents/Info.plist"
mv -f "$stage/CrabHosts.icns" "$contents/Resources/CrabHosts.icns"
# Repeated launches may already refer to this same executable inode.
if ! [ "$stage/CrabHosts" -ef "$contents/MacOS/CrabHosts" ]; then
    mv -f "$stage/CrabHosts" "$contents/MacOS/CrabHosts"
fi
rm -rf "$stage"
trap - EXIT
exec "$contents/MacOS/CrabHosts" "$@"
