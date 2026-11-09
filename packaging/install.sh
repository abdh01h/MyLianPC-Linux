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

if [[ ! -w /usr ]]; then
    echo "/usr is not writable. Immutable/read-only systems such as SteamOS need a writable root or an overlay." >&2
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
# The uninstaller stays available without the source checkout.
install -Dm755 "$ROOT/packaging/uninstall.sh" /usr/local/bin/mylianpc-uninstall
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

# ------------------------------------------------------- kernel prerequisites
# The Corsair DRAM RGB controller is reached through i2c-dev on the AMD PIIX4
# SMBus. Some distributions (notably Arch) do not load i2c-dev at boot, so
# /dev/i2c-* would never appear. Load both modules now and persist them.
install -d /etc/modules-load.d
printf '%s\n' \
    '# MyLianPC: SMBus character devices for the Corsair DRAM RGB controller.' \
    'i2c-dev' \
    'i2c-piix4' \
    > /etc/modules-load.d/mylianpc.conf
chmod 0644 /etc/modules-load.d/mylianpc.conf
modprobe i2c-dev 2>/dev/null || true
modprobe i2c-piix4 2>/dev/null || true

udevadm control --reload-rules
udevadm trigger

systemctl daemon-reload
systemctl enable --now mylianpc.service
# An upgrade replaces the binary of an already running service; restart it so
# the new build is actually in use.
systemctl try-restart mylianpc.service

# Report the one kernel setting the installer cannot change safely: the AMD
# SMBus only lets i2c-piix4 claim the controller when the firmware has not
# reserved its resources. Never fail the install over this; everything except
# the Corsair RAM RGB works without it.
report_kernel_setup() {
    local warnings=0
    if [[ ! -e /sys/module/i2c_piix4 ]] ||
        [[ ! -e /sys/bus/pci/devices/0000:00:14.0/driver ]]; then
        warnings=1
        printf '\nwarning: the AMD SMBus is not bound, so Corsair RAM RGB cannot be controlled.\n' >&2
        printf '  This board needs the kernel parameter acpi_enforce_resources=lax.\n' >&2
        printf '  GRUB: add it to GRUB_CMDLINE_LINUX_DEFAULT in /etc/default/grub, then run\n' >&2
        printf '        sudo grub-mkconfig -o /boot/grub/grub.cfg\n' >&2
        printf '  systemd-boot: append it to the "options" line of /boot/loader/entries/*.conf.\n' >&2
        printf '  Limine: append it to the entry cmdline. Then reboot.\n' >&2
    fi
    if ! compgen -G '/dev/i2c-*' >/dev/null; then
        warnings=1
        printf '\nwarning: no /dev/i2c-* devices exist. Run "sudo modprobe i2c-dev"\n' >&2
        printf '  and check "mylianpc-probe ram".\n' >&2
    fi
    if ! command -v iw >/dev/null 2>&1; then
        printf '\nnote: install "iw" (Arch: sudo pacman -S iw) to show Wi-Fi SSID, signal\n' >&2
        printf '  and channel on the dashboard. Everything else works without it.\n' >&2
    fi
    if (( warnings == 0 )); then
        printf '\nSMBus setup looks good (i2c_piix4 bound, /dev/i2c-* present).\n'
    fi
}
report_kernel_setup

echo
echo "MyLianPC installed."
echo "  service : systemctl status mylianpc.service"
echo "  window  : mylianpc-ui (or the MyLianPC desktop entry)"
echo "  update  : sudo mylianpc-update"
echo "  remove  : sudo mylianpc-uninstall"
if [[ -n "${SUDO_USER:-}" ]]; then
    echo "  log out and back in once so $SUDO_USER receives the mylianpc group."
fi
