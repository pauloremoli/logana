    # Desktop integration for the GUI and full distributions.
    case "$(uname -s)" in
      Linux)
        _data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
        ensure mkdir -p "$_data_dir/applications" "$_data_dir/icons/hicolor/256x256/apps"
        printf '%s' '@PNG@' | base64 -d > "$_data_dir/icons/hicolor/256x256/apps/logana.png" || return 1
        # Desktop Exec uses its own quoting rules, including doubled percent signs.
        _desktop_exec=$(printf '%s' "$_install_dir/logana-gui" | sed 's/\\/\\\\\\\\/g; s/"/\\\\"/g; s/`/\\\\`/g; s/\$/\\\\$/g; s/%/%%/g')
        ensure cat > "$_data_dir/applications/logana.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Logana
Comment=Explore and analyze logs
Exec="$_desktop_exec" %F
Icon=logana
Terminal=false
Categories=Development;Utility;
StartupWMClass=logana
DESKTOP
        _desktop_dir=$(xdg-user-dir DESKTOP 2>/dev/null || printf '%s' "$HOME/Desktop")
        if [ -d "$_desktop_dir" ] && [ "$_desktop_dir" != "$HOME" ]; then
            ensure cp "$_data_dir/applications/logana.desktop" "$_desktop_dir/logana.desktop"
            ensure chmod +x "$_desktop_dir/logana.desktop"
        fi
        if command -v update-desktop-database >/dev/null 2>&1; then
            ignore update-desktop-database "$_data_dir/applications"
        fi
        ;;
      Darwin)
        _app="$HOME/Applications/Logana.app"
        ensure mkdir -p "$_app/Contents/MacOS" "$_app/Contents/Resources"
        ensure cp "$_install_dir/logana-gui" "$_app/Contents/MacOS/logana-gui"
        printf '%s' '@ICNS@' | base64 -D > "$_app/Contents/Resources/logana.icns" || return 1
        ensure cat > "$_app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Logana</string>
<key>CFBundleIdentifier</key><string>com.pauloremoli.logana</string>
<key>CFBundleExecutable</key><string>logana-gui</string>
<key>CFBundleIconFile</key><string>logana.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>@VERSION@</string>
<key>CFBundleShortVersionString</key><string>@VERSION@</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
        if [ -d "$HOME/Desktop" ]; then
            ensure ln -sfn "$_app" "$HOME/Desktop/Logana.app"
        fi
        ;;
    esac
