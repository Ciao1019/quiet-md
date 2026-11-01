#!/bin/zsh
# Retake the 开始指南 screenshots: the app on the desktop picture, with the
# system's corners and shadow and 48pt of wallpaper around it. The window
# floats in the middle of the main screen for about a minute (it never takes
# focus); a borderless wallpaper window behind it hides whatever else is open.
# usage: shared-mock/guides/shots/shots.sh   (after cargo build --release)
set -e
cd "$(dirname "$0")/../../.."
work=$(mktemp -d /tmp/quiet-shots.XXXXXX)
python3 shared-mock/guides/build.py "$work/lib" >/dev/null
sed 's/^SHOT \(.*\)$/deskshot \1 48/' shared-mock/guides/shots/steps.txt > "$work/steps.txt"
swiftc -O shared-mock/guides/backdrop.swift -o "$work/backdrop" 2>/dev/null
wall=$(swift -e 'import AppKit; print(NSWorkspace.shared.desktopImageURL(for: NSScreen.screens[0])!.path)' 2>/dev/null)
sips -s format png "$wall" --out "$work/wall.png" >/dev/null
# Covers both window sizes in steps.txt plus their margins.
"$work/backdrop" 78 56 1196 796 180 "$work/wall.png" &
bd=$!
sleep 1.5
FDL_NOPARK=1 python3 tools/harness/run.py "$work/steps.txt" "$work/lib" "$work/run" >/dev/null 2>&1 || true
kill $bd 2>/dev/null || true
for n in overview home search theme toolbar slash block-menu; do
  [ -f "$work/run/$n.png" ] || { echo "shots.sh: $n.png missing (see $work/run/log)" >&2; exit 1; }
  cp "$work/run/$n.png" "shared-mock/guides/assets/$n.png"
done
echo "$work"
