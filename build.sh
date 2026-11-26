#!/bin/sh
set -eu
cd "$(dirname "$0")"
profile=release
if [ "${1:-}" = "--debug" ]; then
    profile=debug
    cargo build --locked
else
    cargo build --locked --release
fi
case "$(uname -s)" in
    Darwin)
        app="target/$profile/Calendar.app"
        mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
        cp "target/$profile/calendar-gpui" "$app/Contents/MacOS/Calendar"
        cp assets/app-icon/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
        cp assets/fonts/OFL.txt "$app/Contents/Resources/Mulish-OFL.txt"
        cp assets/app-icon/Info.plist "$app/Contents/Info.plist"
        codesign --force --sign - "$app"
        printf '%s\n' "Built $app"
        ;;
    Linux)
        app="target/$profile/Calendar"
        mkdir -p "$app"
        cp "target/$profile/calendar-gpui" "$app/calendar-gpui"
        cp assets/app-icon/calendar.png "$app/calendar.png"
        cp assets/fonts/OFL.txt "$app/Mulish-OFL.txt"
        destination="$(cd "$app" && pwd)"
        printf '[Desktop Entry]\nType=Application\nName=Calendar\nExec="%s/calendar-gpui"\nIcon=%s/calendar.png\nTerminal=false\nCategories=Office;Calendar;\n' "$destination" "$destination" > "$app/Calendar.desktop"
        chmod +x "$app/Calendar.desktop"
        printf '%s\n' "Built $app"
        ;;
    *) printf '%s\n' 'App packaging supports macOS and Linux.' >&2; exit 1 ;;
esac
