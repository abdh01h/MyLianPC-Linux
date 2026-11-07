#!/usr/bin/env bash
# Remove the installed program. Profiles and the network state in
# /var/lib/mylianpc are kept unless --purge is given.
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "Run this script with sudo." >&2
    exit 1
fi

systemctl disable --now mylianpc.service 2>/dev/null || true

rm -f /usr/bin/mylianpc-service /usr/bin/mylianpc-ui /usr/bin/mylianpc-probe
rm -f /etc/systemd/system/mylianpc.service
rm -f /etc/udev/rules.d/99-mylianpc.rules
rm -f /usr/share/applications/mylianpc.desktop
rm -f /usr/share/icons/hicolor/scalable/apps/mylianpc.svg

if [[ "${1:-}" == "--purge" ]]; then
    rm -rf /var/lib/mylianpc /run/mylianpc
fi

udevadm control --reload-rules || true
systemctl daemon-reload || true
echo "MyLianPC removed."
