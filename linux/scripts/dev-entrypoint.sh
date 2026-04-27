#!/usr/bin/env bash
set -euo pipefail

# Start a virtual X server, a tiny window manager, and a VNC bridge so the GTK
# window is visible from the host on vnc://localhost:5900.

if [ -z "${DISPLAY:-}" ]; then
    export DISPLAY=:99
fi

if ! pgrep -x Xvfb >/dev/null 2>&1; then
    # Clean stale lock files from previous runs (container restart preserves /tmp).
    rm -f /tmp/.X*-lock /tmp/.X11-unix/X* 2>/dev/null
    Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp +extension RANDR &
    # Give Xvfb a moment to come up.
    for _ in $(seq 1 50); do
        if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
            break
        fi
        sleep 0.05
    done
fi

if ! pgrep -x fluxbox >/dev/null 2>&1; then
    fluxbox >/dev/null 2>&1 &
fi

if ! pgrep -x x11vnc >/dev/null 2>&1; then
    # macOS Screen Sharing requires a password, so set a fixed dev one.
    VNC_PASS="${VNC_PASSWORD:-iris}"
    mkdir -p /root/.vnc
    x11vnc -storepasswd "$VNC_PASS" /root/.vnc/passwd >/dev/null 2>&1
    x11vnc -display "$DISPLAY" -forever -shared -rfbport 5900 \
        -rfbauth /root/.vnc/passwd -bg -quiet \
        -noxdamage -noxrecord -noxfixes >/dev/null 2>&1
fi

# A session bus is required for GApplication's single-instance behaviour.
# Pin it to a fixed path so every `docker exec` shell sees the same bus
# instead of auto-launching its own.
DBUS_SOCK=/tmp/iris-dbus.sock
DBUS_ADDR="unix:path=$DBUS_SOCK"
if ! pgrep -f "dbus-daemon.*$DBUS_SOCK" >/dev/null 2>&1; then
    rm -f "$DBUS_SOCK"
    dbus-daemon --session --fork --address="$DBUS_ADDR" --nopidfile --nosyslog
fi
export DBUS_SESSION_BUS_ADDRESS="$DBUS_ADDR"

# Make the address visible to interactive shells started via `docker exec`.
cat >/etc/profile.d/iris-dbus.sh <<EOF
export DBUS_SESSION_BUS_ADDRESS="$DBUS_ADDR"
EOF
chmod 0644 /etc/profile.d/iris-dbus.sh

if [ "$#" -eq 0 ]; then
    exec bash
fi
exec "$@"
