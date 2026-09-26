#!/usr/bin/env bash
# Check that the Wayfern build Donut downloads can run inside a package's
# sandbox: its libraries resolve there, and it starts headless and answers on
# a DevTools port, which is how Donut drives it.
#
#   wayfern-check.sh flatpak|snap <archive> <output-dir>
#
# <archive> is the Wayfern Linux .tar.xz. It is unpacked into the app's own
# data directory, the one place both sandboxes let the app execute files from.
# XDG_RUNTIME_DIR must point to a directory owned by the current user.
set -euo pipefail

kind="$1"
archive="$2"
out="$3"
mkdir -p "$out"

case "$kind" in
  flatpak)
    app=com.donutbrowser.DonutBrowser
    dir="$HOME/.var/app/$app/data/wayfern-check"
    in_sandbox() { flatpak run --command=sh "$app" -c "$1"; }
    ;;
  snap)
    dir="$HOME/snap/donutbrowser/common/wayfern-check"
    in_sandbox() { snap run --shell donutbrowser -c "$1"; }
    ;;
  *)
    echo "usage: $0 flatpak|snap <archive> <output-dir>" >&2
    exit 2
    ;;
esac

: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR must be set}"
if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
  bus="$XDG_RUNTIME_DIR/bus"
  rm -f "$bus"
  dbus-daemon --session --address="unix:path=$bus" --nofork --nopidfile &
  dbus_pid=$!
  trap 'kill "$dbus_pid" 2>/dev/null || true' EXIT
  export DBUS_SESSION_BUS_ADDRESS="unix:path=$bus"
  for _ in $(seq 1 50); do [ -S "$bus" ] && break; sleep 0.1; done
fi

rm -rf "$dir"
mkdir -p "$dir"
tar -xJf "$archive" -C "$dir"
chrome="$(find "$dir" -type f -name chrome -perm -u+x | head -n 1)"
if [ -z "$chrome" ]; then
  echo "::error::The archive has no chrome executable."
  exit 1
fi
echo "Wayfern executable: $chrome"

in_sandbox "/lib64/ld-linux-x86-64.so.2 --list '$chrome'" >"$out/wayfern-libraries.txt" 2>&1 || true
if grep "not found" "$out/wayfern-libraries.txt" || ! grep -q "=>" "$out/wayfern-libraries.txt"; then
  echo "::error::Wayfern's libraries do not all resolve inside the sandbox."
  cat "$out/wayfern-libraries.txt"
  exit 1
fi
echo "Every Wayfern library resolves inside the sandbox."

# Donut asks the user to accept the Wayfern terms before the first launch and
# then runs this same command. This runner is thrown away after the job.
in_sandbox "'$chrome' --accept-terms-and-conditions" >"$out/wayfern-terms.log" 2>&1

port=9333
in_sandbox "exec '$chrome' --headless=new --no-sandbox --disable-gpu --disable-dev-shm-usage \
  --no-first-run --user-data-dir='$dir/profile' --remote-debugging-address=127.0.0.1 \
  --remote-debugging-port=$port about:blank" >"$out/wayfern-output.log" 2>&1 &
wayfern_pid=$!

answered=0
for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:$port/json/version" >"$out/wayfern-version.json" 2>/dev/null; then
    answered=1
    break
  fi
  kill -0 "$wayfern_pid" 2>/dev/null || break
  sleep 1
done

kill "$wayfern_pid" 2>/dev/null || true
pkill -f "$dir/" 2>/dev/null || true

if [ "$answered" -ne 1 ]; then
  echo "::error::Wayfern did not answer on its DevTools port."
  tail -n 40 "$out/wayfern-output.log" || true
  exit 1
fi
cat "$out/wayfern-version.json"
