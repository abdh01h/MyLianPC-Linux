#!/usr/bin/env bash
# Install MyLianPC Linux from a release build.
#
#   cargo build --release
#   sudo packaging/install.sh
#
# The service runs as root (it needs the HID, USB and SMBus nodes); the desktop
# window runs as the logged-in user and talks to the service over the Unix
# socket. Both use the `mylianpc` group.
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "Run this script with sudo." >&2
    exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/release"

for binary in mylianpc-service mylianpc-ui mylianpc-probe; do
    if [[ ! -x "$BIN/$binary" ]]; then
        echo "Missing $BIN/$binary. Build it first with: cargo build --release" >&2
        exit 1
    fi
done

getent group mylianpc >/dev/null || groupadd --system mylianpc

install -Dm755 "$BIN/mylianpc-service" /usr/bin/mylianpc-service
install -Dm755 "$BIN/mylianpc-ui" /usr/bin/mylianpc-ui
install -Dm755 "$BIN/mylianpc-probe" /usr/bin/mylianpc-probe
# The one-command updater re-fetches /opt/mylianpc-src, rebuilds and reinstalls.
install -Dm755 "$ROOT/install.sh" /usr/local/bin/mylianpc-update
install -Dm644 "$ROOT/packaging/mylianpc.service" /etc/systemd/system/mylianpc.service
install -Dm644 "$ROOT/packaging/99-mylianpc.rules" /etc/udev/rules.d/99-mylianpc.rules
# The desktop entry must be named after the GtkApplication id: KDE and GNOME
# resolve the window icon, the task grouping and notifications through it.
install -Dm644 "$ROOT/packaging/io.github.mylianpc.MyLianPC.desktop" \
    /usr/share/applications/io.github.mylianpc.MyLianPC.desktop
# Older installs used a name that did not match the app id (wrong window icon).
rm -f /usr/share/applications/mylianpc.desktop
install -Dm644 "$ROOT/packaging/mylianpc.svg" /usr/share/icons/hicolor/scalable/apps/mylianpc.svg

# The desktop user must be in the group to open the socket and (for the probe)
# the device nodes. The group change takes effect at the next login.
if [[ -n "${SUDO_USER:-}" ]]; then
    usermod -aG mylianpc "$SUDO_USER"
fi

udevadm control --reload-rules
udevadm trigger

systemctl daemon-reload
systemctl enable --now mylianpc.service
# An upgrade replaces the binary of an already running service; restart it so
# the new build is actually in use.
systemctl try-restart mylianpc.service

echo
echo "MyLianPC installed."
echo "  service : systemctl status mylianpc.service"
echo "  window  : mylianpc-ui (or the MyLianPC desktop entry)"
echo "  update  : sudo mylianpc-update"
if [[ -n "${SUDO_USER:-}" ]]; then
    echo "  log out and back in once so $SUDO_USER receives the mylianpc group."
fi
