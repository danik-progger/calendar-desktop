#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

app=target/release/Calendar.app
dmg=Calendar-macOS-arm64.dmg
image=$(mktemp -d)
trap 'rm -rf "$image"' EXIT

if [[ -n "${APPLE_CERTIFICATE:-}" ]]; then
  : "${APPLE_CERTIFICATE_PASSWORD:?Set the certificate password}"
  : "${APPLE_ID:?Set the Apple account email}"
  : "${APPLE_PASSWORD:?Set the Apple app-specific password}"
  : "${APPLE_TEAM_ID:?Set the Apple team ID}"
  keychain="$image/release.keychain-db"
  trap 'security delete-keychain "$keychain"; rm -rf "$image"' EXIT
  password=$(openssl rand -hex 24)
  printf '%s' "$APPLE_CERTIFICATE" | openssl base64 -d -A > "$image/certificate.p12"
  security create-keychain -p "$password" "$keychain"
  security unlock-keychain -p "$password" "$keychain"
  security import "$image/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain"
  codesign --force --options runtime --timestamp --sign 'Developer ID Application' --keychain "$keychain" "$app"
fi

codesign --verify --strict "$app"
mkdir "$image/volume"
ditto "$app" "$image/volume/Calendar.app"
ln -s /Applications "$image/volume/Applications"
hdiutil create -ov -format UDZO -volname Calendar -srcfolder "$image/volume" "$dmg"
hdiutil verify "$dmg"

if [[ -n "${APPLE_CERTIFICATE:-}" ]]; then
  xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" --wait --output-format json > "$image/notarization.json"
  jq -e '.status == "Accepted"' "$image/notarization.json"
  xcrun stapler staple "$dmg"
fi
