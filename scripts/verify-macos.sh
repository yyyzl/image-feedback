#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || exit 1
DMG="${1:?Pass the DMG to verify}"
DMG="$(cd "$(dirname "$DMG")" && pwd)/$(basename "$DMG")"
OUTPUT="${2:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/imagefeedback-macos-verification}"
mkdir -p "$OUTPUT"
OUTPUT="$(cd "$OUTPUT" && pwd)"
WORK="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/imagefeedback-install.XXXXXX")"
MOUNT="$WORK/mounted"
mkdir -p "$MOUNT" "$WORK/Applications"
MOUNTED=0
cleanup(){ if [[ "$MOUNTED" == 1 ]]; then hdiutil detach "$MOUNT" >/dev/null 2>&1 || true; fi; }
trap cleanup EXIT
hdiutil verify "$DMG"
hdiutil attach -readonly -nobrowse -mountpoint "$MOUNT" "$DMG"
MOUNTED=1
[[ "$(readlink "$MOUNT/Applications")" == /Applications ]]
codesign --verify --deep --strict "$MOUNT/Image Feedback.app"
ditto "$MOUNT/Image Feedback.app" "$WORK/Applications/Image Feedback.app"
hdiutil detach "$MOUNT"
MOUNTED=0
APP="$WORK/Applications/Image Feedback.app"
BIN="$APP/Contents/MacOS/ImageFeedback"
test -x "$BIN"
test -s "$APP/Contents/Resources/AppIcon.icns"
test -s "$APP/Contents/Resources/licenses/THIRD_PARTY_LICENSES.html"
xcrun lipo "$BIN" -verify_arch arm64 x86_64
codesign --verify --deep --strict "$APP"
cd "$WORK"
"$BIN" --version
for MODE in smoke-test delivery-smoke dialog-smoke; do
  DIR="$OUTPUT/$MODE"
  SLINT_BACKEND=winit-software "$BIN" --editor "--$MODE" "$DIR"
  case "$MODE" in smoke-test) REPORT=smoke-report.json;; delivery-smoke) REPORT=delivery-report.json;; dialog-smoke) REPORT=dialog-report.json;; esac
  test -s "$DIR/$REPORT"
  python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));assert d["result"]=="passed",d' "$DIR/$REPORT"
done
# Verify normal .app entry creates an accessory resident and a regular editor.
# These processes have no image loaded and are owned solely by this test.
SLINT_BACKEND=winit-software "$BIN" > "$OUTPUT/default-launch.log" 2>&1 &
TRAY_PID=$!
sleep 3
kill -0 "$TRAY_PID"
CHILD="$(pgrep -P "$TRAY_PID" | head -1 || true)"
[[ -n "$CHILD" ]] || { echo 'Normal app entry did not create an editor' >&2; cat "$OUTPUT/default-launch.log"; kill "$TRAY_PID" || true; exit 1; }
kill -0 "$CHILD"
kill -TERM "$CHILD"
kill -TERM "$TRAY_PID"
wait "$TRAY_PID" || true
python3 - "$OUTPUT" <<'PY'
import json,pathlib,platform,sys
out=pathlib.Path(sys.argv[1])
(out/'installation-report.json').write_text(json.dumps({'result':'passed','host_arch':platform.machine(),'universal_slices':['arm64','x86_64'],'dmg_detached_before_launch':True,'native_chrome_checked_by_smoke':True,'appkit_quit_guard_checked_by_smoke':True,'normal_resident_and_editor_launch':True,'signing':'ad-hoc; not Apple-notarized','limitations':'Scene events, not physical traffic-light clicks or Gatekeeper approval.'},indent=2))
PY
echo 'Copied .app runs after DMG detach, outside the build tree.'
