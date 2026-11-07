# MyLianPC Linux

A native Linux reimplementation of [MyLianPC](../MyLianPC) for the same PC build:
**GIGABYTE B850 AORUS ELITE WIFI7 + Lian Li HydroShift II LCD-C + Corsair Vengeance RGB DDR5
+ Lian Li Vector V100R**.

This is not the Windows binary under Wine. It is a Rust service and a GTK4 window that talk to the
hardware through Linux interfaces: `hidraw`, raw `usbfs`, `i2c-dev`, `hwmon`, NVML and RAPL.

## What it does

| Area | Features |
|---|---|
| Lighting | Motherboard ARGB headers (case strip and fans), chipset accent, LED_C, both RAM modules and the pump ring from one page. Static, breathing, flash, colour cycle, rainbow, rainbow wave, wave and gradient; one colour by default with **+ Add colour**; native colour picker, palette, brightness and speed |
| Cooler LCD | 21 themes, image and animated GIF/WebP media, colours, fonts, text size, °C/°F, 12/24-hour clock, gauge style, metric selection, brightness, rotation, refresh rate, live preview and push |
| Cooling | Pump and radiator-fan curves from coolant, CPU or GPU temperature; Silent, Case fans, Balanced, Performance and Maximum presets; custom curves; hysteresis; failsafe 100% by 90 °C; the radiator fan LEDs follow the pump ring by default |
| Monitoring | CPU and GPU temperature, load, clock and power; VRAM; memory usage, temperature, speed and EXPO/XMP; NVMe temperature and remaining life; coolant, pump and radiator-fan RPM; live network details and daily traffic totals |
| Profiles | Four example profiles (Work, Silent, Night Mode, Maximum Cooling; **Work is the default**), create, duplicate, rename, delete, import, export one or all profiles as JSON, activate; one shared profile list |
| Service | systemd service owns the devices from boot; the window needs no root rights; every request is re-validated server-side |

Motherboard fan-header control is **not available** on this Linux build: the board's ITE IT8696E
Super I/O has no `it87` driver, so those headers remain under BIOS control and are not advertised.
Everything else the Windows application drives is implemented.

### Window

The GTK4 window has a persistent sidebar (Dashboard, Lighting, LCD, Profiles, Cooling, Devices,
Settings, About), a live service/profile footer and `Ctrl+1`…`Ctrl+8` page shortcuts. About is
the last entry and carries the version, licence and GitHub links. It opens at 1150×800, can be
resized down to roughly 880 px wide, and reflows cards/actions to one column when narrow. It
follows the system light/dark theme live. Cards and surfaces are square; buttons and inputs keep
a small 4 px radius.

Every control has an immediate preview: the Lighting page renders the selected effect and LED
strip, Cooling plots the curve with the current temperature marker, and the LCD page shows the real
panel preview automatically and live-updates from unsaved form values (theme, colours, fonts,
rotation and brightness) while you edit. Colour fields use the native GTK colour picker, with a
hex entry and quick-colour shortcuts.

The visual system is documented in [docs/design-system.md](docs/design-system.md) and implemented
in [style.css](crates/mylianpc-ui/src/style.css); the same tokens are available as JSON in
[assets/design-tokens.json](assets/design-tokens.json).

## Install or update in one command

On a supported distribution this installs the build dependencies and Rust, builds the project,
installs the service, window, probe, udev rules, systemd unit and desktop entry, and enables the
service:

```bash
curl -fsSL https://raw.githubusercontent.com/abdh01h/MyLianPC-Linux/HEAD/install.sh | sudo bash
```

Running it again updates an existing installation. After the first install the same script is
available locally, so the shorter form also works:

```bash
sudo mylianpc-update
```

The updater fetches the latest source into `/opt/mylianpc-src` (override with
`MYLIANPC_SRC_DIR=…`), rebuilds and restarts the service. Close and reopen the window afterwards
so the new UI binary is used. **Log out and back in once** after the first install so your user
receives the `mylianpc` group.

### Supported distributions

| Distribution | Minimum | Notes |
|---|---|---|
| Ubuntu | 24.04 LTS | GTK 4.14; Ubuntu 22.04 is too old. Linux Mint 22 and Pop!_OS 24.04 follow Ubuntu |
| Debian | 13 (Trixie) | GTK 4.18; Debian 12 ships GTK 4.8 and is not supported |
| Fedora | 40 | GTK 4.14; immutable variants need layering |
| Arch Linux / EndeavourOS | current | rolling GTK 4 release |
| openSUSE | Tumbleweed | GTK 4.14+; Leap may be too old |

The installer supports `apt`, `dnf`, `pacman` and `zypper`. Other distributions work when GTK 4.10
or newer, Rust 1.85 or newer, systemd and the kernel drivers listed below are available. The
window runs on both Wayland and X11. This is a hardware-specific application: it supports the
exact PC build described above, not arbitrary RGB or cooler hardware.

## Build from source

Rust is installed with rustup in `~/.cargo`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace
cargo build --release
```

The system needs the GTK4 development packages (4.10 or newer for the file dialogs), `libusb` is
not required, and `image`/`tiny-skia`/`fontdb` are compiled in as Rust dependencies. Cairo, Pango
and fontconfig are used indirectly by GTK for the window; the LCD renderer is CPU-only and does
not need a display.

To install a checkout manually:

```bash
cargo build --release
sudo packaging/install.sh
```

The script installs the service, the window, the probe, the udev rules, the systemd unit, the
desktop entry and the `mylianpc-update` command, adds the desktop user to the `mylianpc` group,
and enables the service. **Log out and back in once** so the group membership applies to the
desktop session (or use `sg mylianpc -c '…'` in a terminal until then).

The installed service listens on `/run/mylianpc/mylianpc.sock`; the window discovers that path
automatically. A checkout/development session can override it with `MYLIANPC_SOCKET`.

The udev rules grant the group access to:

* the Fusion2 HID collection `048d:5711`;
* the HydroShift II `1cbe:a021` USB device;
* the AMD `SMBus PIIX4` adapters (`i2c-2`, `i2c-3`, `i2c-4` here) used for the Corsair DIMMs.

Nothing else is granted.

## Run from a checkout

```bash
# read-only probes (no lighting, LCD or fan write)
cargo run -p mylianpc-probe -- inventory
cargo run -p mylianpc-probe -- fusion2
cargo run -p mylianpc-probe -- ram
sg mylianpc -c 'cargo run -p mylianpc-probe -- hydroshift'   # firmware/telemetry query

# status report without connecting the devices (safe)
cargo run -p mylianpc-service -- --status

# the real service and the window
sg mylianpc -c 'cargo run -p mylianpc-service'
cargo run -p mylianpc-ui
```

In a development session both sides can share an explicit socket:
`MYLIANPC_SOCKET=/tmp/mylianpc-dev.sock`. The UI never opens a device node; it only talks to the
service.

`mylianpc-service --once` prints the read-only inventory JSON. `--status` samples the sensors and
prints adapter status without initialising the cooler, the HID controller or the DIMMs.
`--ipc-only` serves the socket and sensor loop but starts no device loop, so it never initialises
a device on startup; it is useful for working on the window without touching hardware.

## Safety rules

These carry over from the Windows application and are not negotiable:

- **No simulation layer.** Every control maps to a real device write.
- **The service is the only process that touches hardware.** The UI uses IPC and never opens a
  device node.
- Fan curves reach 100 % by 90 °C; a missing or implausible sensor commands 100 %; a curve never
  commands below its minimum; the pump never goes below 1600 RPM.
- The cooler's firmware falls back to its own curve about ten seconds after the last pump/fan
  command, so the setpoint is repeated every five seconds.
- Stopping the service sets the pump and fans to a safe fixed speed.
- SMBus writes to the SPD address range `0x50`–`0x57` are refused in code. Only the Corsair RGB
  controller addresses are used.
- A module that stops answering is never written to; a stalled module is recovered only from a
  verified device-info block.
- Adapters advertise their real zones, effects and fans; the UI offers only those.

## Device notes

* **Fusion2 (048D:5711)** — HID feature reports, report id `0xCC`, 64 bytes. The vendor collection
  is `hidraw8` on this board (usage page `0xFF89`, usage `0xCC`). Effects are committed once per
  batch; per-LED rainbow/wave/gradient are streamed and are not stored in flash.
* **HydroShift II (1CBE:A021)** — bulk endpoints `0x01`/`0x81`, 512-byte packets, DES-CBC encrypted
  headers and TinyUZ-compressed ring animations. A background reader keeps one IN transfer pending
  so replies never fill the controller's buffer.
* **Corsair Vengeance RGB DDR5** — detected at `0x19`/`0x1B` on `i2c-2`; firmware `1.1.9`,
  protocol 4. Static colours are read back after writing; a module that acknowledges but keeps its
  old colours is recovered and rewritten.
* **Sensors** — `k10temp`, `amdgpu`/NVML, `spd5118`, `nvme`, RAPL, `/proc` and SMBIOS type 17.
  The service must run as root to read the NVMe SMART log, SMBIOS memory speed and RAPL energy.
* **Realtek NIC hwmon** exposes only its own temperature; it is not mapped to a metric.

## Layout

```
crates/
  mylianpc-core/       colours, effects, fan curves, profiles, IPC, settings, network model
  mylianpc-hardware/   usbfs/hidraw/i2c transports, Fusion2, HydroShift, Corsair, sensors,
                       NVML, NVMe SMART, RAPL, SMBIOS memory, Linux network info
  mylianpc-lcd/        480×480 themes, media decoding, JPEG/PNG encoding
  mylianpc-service/    systemd service, adapters, loops, IPC server
  mylianpc-ui/         GTK4 window (Dashboard, Lighting, LCD, Profiles, Cooling, Devices,
                       Settings, About)
  mylianpc-probe/      read-only hardware probe CLI
install.sh             one-command install/update entry point
packaging/             udev rules, systemd unit, install/uninstall, desktop entry, icon
```

## Reference

The Windows implementation remains in `../MyLianPC` and is the source of truth for protocols,
effect maths and safety behaviour. Ports are deterministic; the C# file is named in each module's
doc comment.

## License

GPL-3.0-or-later. The full text is in [LICENSE](LICENSE).

You may run, study, share and modify the program for any purpose. If you distribute the program
or a modified version, it must remain under the GPL and the corresponding source must be
available: nobody can take the project, update it and publish it as closed-source software.
There is no warranty. Note that the GPL itself does not forbid charging for copies; it forbids
turning distributed copies or updates into proprietary software. If you need a licence that
forbids commercial distribution entirely, that would be a non-free licence and would no longer
be GPL-compatible.
