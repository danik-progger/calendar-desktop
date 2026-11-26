#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

package=$(mktemp -d)
trap 'rm -rf "$package"' EXIT
install -Dm755 target/release/calendar-gpui "$package/usr/bin/calendar-gpui"
install -Dm644 assets/app-icon/calendar.png "$package/usr/share/icons/hicolor/512x512/apps/dev.calendar.gpui.png"
install -Dm644 assets/fonts/OFL.txt "$package/usr/share/doc/calendar-gpui/Mulish-OFL.txt"
install -Dm644 packaging/calendar.desktop "$package/usr/share/applications/dev.calendar.gpui.desktop"

version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml)
dependencies=$(cd "$package" && mkdir debian && printf 'Source: calendar-gpui\n\nPackage: calendar-gpui\nArchitecture: any\n' > debian/control &&
  dpkg-shlibdeps -O -eusr/bin/calendar-gpui)
rm "$package/debian/control"
rmdir "$package/debian"
mkdir "$package/DEBIAN"
cat > "$package/DEBIAN/control" <<EOF
Package: calendar-gpui
Version: $version+ci.${GITHUB_RUN_NUMBER:-0}
Architecture: amd64
Maintainer: Calendar developers
Depends: ${dependencies#shlibs:Depends=}, libvulkan1, libfontconfig1
Description: Calendar for desktop
EOF
dpkg-deb --root-owner-group --build "$package" Calendar-Linux-amd64.deb
