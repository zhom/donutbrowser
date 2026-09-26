#!/usr/bin/env bash
# Start a packaged Donut Browser on a virtual display and check that it stays
# up. .github/workflows/linux-packages.yml runs it for the Flatpak and the Snap.
#
#   smoke-test.sh <output-dir> <seconds> <command> [args...]
#
# XDG_RUNTIME_DIR must point to a directory owned by the current user. The
# script starts a private session bus at $XDG_RUNTIME_DIR/bus, the path both
# sandboxes let the app reach, and an Xvfb display. It fails when the app exits
# before <seconds> have passed or prints a Rust panic. The app's output and a
# screenshot are left in <output-dir>. A screenshot with almost no colours
# (a blank window) is reported as a warning.
set -euo pipefail

out="$1"
seconds="$2"
shift 2
mkdir -p "$out"

: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR must be set}"
bus="$XDG_RUNTIME_DIR/bus"
rm -f "$bus"
dbus-daemon --session --address="unix:path=$bus" --nofork --nopidfile &
dbus_pid=$!
export DBUS_SESSION_BUS_ADDRESS="unix:path=$bus"

display_number=99
Xvfb ":$display_number" -screen 0 1600x1000x24 -nolisten tcp >"$out/xvfb.log" 2>&1 &
xvfb_pid=$!
export DISPLAY=":$display_number"

trap 'kill "$xvfb_pid" "$dbus_pid" 2>/dev/null || true' EXIT

for _ in $(seq 1 100); do
  if [ -S "$bus" ] && [ -S "/tmp/.X11-unix/X$display_number" ]; then
    break
  fi
  sleep 0.1
done

echo "Starting: $*"
"$@" >"$out/app-output.log" 2>&1 &
app_pid=$!

sleep "$seconds"

failed=0
if kill -0 "$app_pid" 2>/dev/null; then
  echo "The app is still running after ${seconds}s."
else
  code=0
  wait "$app_pid" || code=$?
  echo "::error::The app exited within ${seconds}s with exit code $code."
  failed=1
fi

if command -v import >/dev/null 2>&1; then
  import -window root "$out/screenshot.png" || true
  colours="$(identify -format '%k' "$out/screenshot.png" 2>/dev/null || echo 0)"
  echo "The screenshot has $colours colours."
  if [ "$colours" -lt 64 ]; then
    echo "::warning::The screenshot has only $colours colours. The window may be blank."
  fi
fi

kill "$app_pid" 2>/dev/null || true
for _ in $(seq 1 50); do
  kill -0 "$app_pid" 2>/dev/null || break
  sleep 0.1
done
kill -9 "$app_pid" 2>/dev/null || true

if grep -n "panicked at" "$out/app-output.log"; then
  echo "::error::The app panicked."
  failed=1
fi

echo "--- app output (last 100 lines)"
tail -n 100 "$out/app-output.log" || true
exit "$failed"
