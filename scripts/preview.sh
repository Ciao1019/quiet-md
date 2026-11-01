#!/bin/zsh
# Replace the running hand-test build (dist-preview) with target/release. It
# runs as dist-preview/quiet.md.app (so the menu bar says quiet.md) on the
# app's own default library, ~/Documents/Quiet.md. The app initializes the
# bundled 开始指南 on first launch, just like an installed build.
# Only this checkout's own binaries are stopped:
# pkill matches this checkout's absolute path. Log: /tmp/quiet-md-preview.log
set -e
cd "$(dirname "$0")/.."
home="${QUIET_MD_HOME:-$HOME/Documents/Quiet.md}"
# An empty path would put the library in the current directory.
[ -n "$home" ] || { echo "preview.sh: no preview library" >&2; exit 1; }
export QUIET_MD_HOME="$home"
app="$PWD/dist-preview/quiet.md.app"
bin="$app/Contents/MacOS/quiet_md"
pkill -f "^$bin" || true
sleep 1
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp assets/branding/app-icon.png "$app/Contents/Resources/app-icon.png"
cp target/release/quiet_md "$bin"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>quiet.md</string>
  <key>CFBundleDisplayName</key><string>quiet.md</string>
  <key>CFBundleIdentifier</key><string>local.quietmd.preview</string>
  <key>CFBundleExecutable</key><string>quiet_md</string>
  <key>CFBundleIconFile</key><string>app-icon.png</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSDocumentsFolderUsageDescription</key><string>quiet.md 把文档库放在「文稿」里的 quiet.md 文件夹。</string>
</dict></plist>
PLIST
codesign --force --sign - "$app" >/dev/null 2>&1 || true
(QUIET_MD_HOME="$QUIET_MD_HOME" nohup "$bin" >/tmp/quiet-md-preview.log 2>&1 &)
sleep 2; pgrep -fl "^$bin"
