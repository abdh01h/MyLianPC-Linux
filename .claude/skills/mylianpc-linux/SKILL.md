---
name: mylianpc-linux
description: Use when working in the MyLianPC-Linux repository — building or testing, porting a device adapter, adding a lighting effect, LCD theme or fan-curve behaviour, or touching the service/UI. Covers the read-only rule, hardware safety invariants, the Rust toolchain quirk and the reference Windows source.
---

# MyLianPC Linux development

This is a **native Rust reimplementation** of MyLianPC for Linux, for the same PC build:
GIGABYTE B850 AORUS ELITE WIFI7, Lian Li HydroShift II LCD-C, Corsair Vengeance RGB DDR5 and
Lian Li Vector V100R. It is not a port of the WPF binary and it does not run under Wine.

The reference Windows source is at `../MyLianPC` (on the mounted Windows Desktop). That code is
the source of truth for protocols, effect maths and safety behaviour.

## Non-negotiables

- **No simulation layer.** Every control maps to a real device write.
- **No hardware writes may be run until the user explicitly approves them.** The service owns the
  lighting/LCD/pump/fan write paths; the probe remains read-only (inventory, Fusion2 `0x60`/`0x61`,
  HydroShift `GetVersion`/`GetParams`, Corsair device-info detection). Never run a lighting, LCD or
  cooling write to "test" it: use unit tests and pure encoder/decoder checks.
- **The service will be the only process that touches hardware.** The UI must talk to it over
  IPC and must never open `/dev/hidraw*`, `/dev/bus/usb` or `/dev/i2c-*` itself.
- **Advertise, then offer.** An adapter reports the exact zones, effects, fans and capabilities
  it supports; the UI offers only those.
- **Validate on the service side.** IPC requests are re-validated server-side.
- **Keep the safety invariants:**
  - curves reach 100% by 90 °C;
  - duty never drops below the curve minimum;
  - the pump never goes below 1600 RPM;
  - a missing or implausible sensor means 100%;
  - the setpoint is repeated every 5 s (firmware falls back after ~10 s);
  - stopping the service restores safe defaults.
- **SMBus:** never write SPD addresses `0x50`–`0x57`. Only the Corsair RGB controller addresses
  are in scope.
- **No vendor software** (L-Connect 3, GCC, Corsair iCUE) may run while this service owns the
  devices. On Linux they normally do not exist, but do not start them to "compare".

## Toolchain

- Rust is installed with **rustup** in `~/.cargo` (`rustc`/`cargo` 1.99.x). `cargo` is not on the
  default PATH in non-login shells: `export PATH="$HOME/.cargo/bin:$PATH"`.
- The system toolchain is installed: gcc 15.2, GTK4 4.22.4, libusb 1.0.29, i2c-tools. Cargo
  uses the default `cc` linker; `.cargo/config.toml` no longer overrides it.
- udev rules are installed at `/etc/udev/rules.d/99-mylianpc.rules`. `hidraw7/8` and the
  HydroShift USB node are group `mylianpc`, mode 0660. The user is in that group, but the
  current shell only gets it after a re-login; use `sg mylianpc -c '…'` until then.
- Source lives at `/home/abdh/Desktop/MyLianPC-Linux` (ext4). Do not build on the mounted NTFS
  Windows Desktop: it is read-only at its root and has file-permission quirks.

## Verified hardware facts (2026-11-07, Ubuntu 26.04, kernel 7.0)

| Item | Value |
|---|---|
| Board | GIGABYTE B850 AORUS ELITE WIFI7 |
| Fusion2 motherboard RGB | USB `048d:5711`, at `1-6`; HID interfaces `1.0` and `1.1`; `hidraw7` is not the vendor collection, **`hidraw8` is** (usage page `0xFF89`, usage `0xCC`, report ID `0xCC`, 64-byte feature reports). Info query verified: `IT5711-GIGABYTE V1.0.29.6`, firmware `1.0.29.6`, chip `0x5711C000`, support flags `0x03`, strip detect `1` |
| HydroShift II LCD-C | USB `1cbe:a021`, at `1-7`, serial `3a2847f9f4c6c503`; interface class `ff`, **no driver bound**; bulk EP `0x01` OUT and `0x81` IN, 512-byte packets |
| AMD SMBus | PCI `00:14.0` (`1022:790b`). Booting with `acpi_enforce_resources=lax` bound `piix4_smbus`; adapters are `i2c-2` (port 0), `i2c-3` (port 2) and `i2c-4` (port 1 at `0xb20`). Corsair RGB controllers are at `0x19` and `0x1B` on **i2c-2**: `1B1C:0900`, firmware `1.1.9`; the first read reported protocol 1, the approved write test reported protocol 4 |
| hwmon | `k10temp`, `amdgpu`, `nvme` (×2), `gigabyte_wmi`; **no `it87`**, so motherboard fan control is unavailable (monitor-only) |
| Device permissions | udev rules installed; `hidraw7/8` and the HydroShift USB node are group `mylianpc`, 0660. Use `sg mylianpc -c '…'` in the current shell |

`mylianpc-probe` is read-only and safe to run. Use `sg mylianpc -c '…'` (or `sudo`) to read the
device nodes.

## Current state (2026-11-07)

- **137 unit tests pass** (`cargo test --workspace`); `cargo clippy --workspace --all-targets`
  is clean.
- Full write paths exist for all three adapters and the cooling loop; the service owns them. The
  UI is a complete thin client with Dashboard, Lighting, LCD, Profiles, Cooling, Devices,
  Settings and About pages (About last in the sidebar).
- **The hardware write paths have not been exercised against the devices.** Only read-only
  operations were run during development: Fusion2 `0x60`/`0x61` identification, HydroShift
  `GetVersion`/`GetParams` (`mylianpc-probe hydroshift`), Corsair device-info detection, sensor
  sampling and status. Do not run lighting/LCD/fan writes without the user's explicit approval.
- The HydroShift USB transport is a raw `usbfs` implementation (`USBDEVFS_BULK`), verified
  read-only: firmware `lianlih2_0003_0020`, coolant 30 °C, pump 2107 RPM, three fans ≈1170 RPM,
  three lighting fans. The service keeps a background IN reader pending.
- Fusion2 identification still verified: `IT5711-GIGABYTE V1.0.29.6`, firmware `1.0.29.6`, chip
  `0x5711C000`, support flags `0x03`, strip detect `1`, byte order `0x00010002` for the headers.
- Corsair modules are detected at `0x19`/`0x1B`, firmware `1.1.9`, protocol **4** (normal).
  `CorsairDram::set_static_colors`, `set_effect`, `check_health` and recovery are implemented and
  wired to the service. The SMBus write sequence tolerates a busy module: after `0x82`, sleep 5 ms
  and keep polling `0x30`; a NAK during that poll is expected, not fatal.
- Root-only sensors verified with `sudo … mylianpc-service --status`: RAPL CPU power, SMBIOS
  memory speed `6000 MT/s` with EXPO active, NVMe temperature and 100 % remaining life, NVML GPU
  temperature/load/clock/power/VRAM.
- The LCD renderer is pure Rust (`tiny-skia` + `ab_glyph`/`fontdb` + `image`); all 21 themes
  render in tests and the preview PNG is verified.
- Development helpers: `--status` samples sensors without connecting devices; `--ipc-only` serves
  IPC and sensors without any device loop. `MYLIANPC_PAGE=<page>` selects a UI page for
  screenshots.
- The installed service listens on `/run/mylianpc/mylianpc.sock`. `socket_path()` checks that path
  before `$XDG_RUNTIME_DIR`, because the desktop UI does not inherit the systemd unit's
  environment. `packaging/install.sh` installs the unit, udev rules, desktop entry, binaries and
  the `mylianpc-update` command. The root `install.sh` is the one-command installer/updater:
  it installs build dependencies and Rust, keeps a source checkout in `/opt/mylianpc-src`,
  builds and calls `packaging/install.sh`. Running it again (or `sudo mylianpc-update`) updates
  the installation.
- Installed and running on this machine (2026-11-07): `mylianpc.service` enabled/active, the
  **Work** profile active, all three devices connected. Radiator fan lighting is **on** by default
  and follows the pump ring (verified: "applied to the ring and 3 radiator fans").
- The Corsair adapter re-reads each module's verified device-info block before every write and
  recovers a module that slipped back into the abnormal firmware mode. This makes "Re-apply"
  fix stale RAM colours immediately instead of waiting for the 30 s health check. A module whose
  info block cannot be read is left untouched and retried.
- LCD edits use `lcd-preview-draft` so the preview follows unsaved form values; brightness and
  rotation are applied to the preview only, `Save and push` reports real panel acknowledgements.
  Draft settings and media names are sanitised server-side. The separate "push saved layout"
  button was removed.
- The combined ring+fans upload is acknowledged but sometimes ignored while the firmware switches
  animations. After a successful fan upload the adapter queues two extra sends, serviced by the
  fast loop 800 ms apart, so one Apply is enough and the IPC response is not held for seconds.
- Lighting has one colour by default; `+ Add colour`, `Remove last` and `Use one colour` manage the
  stops, and `Apply to every zone` is the first, primary action.
- The window opens at 1150×800, is resizable down to ~880 px, and reflows previews/cards/actions to
  one column when narrow.
- UI IPC is asynchronous: dashboard/footer/cooling refreshes, LCD draft previews and hardware
  apply actions run on worker threads and return to the GTK main context, so the window does not
  freeze while the service samples sensors or applies a profile. A window-level capture key
  controller makes every focused GtkDropDown change with Up/Down (wrapping). The LED strip preview caches a
  single representative frame instead of regenerating all 48 animation frames on every paint.
- The window was redesigned with a token-based design system: sidebar navigation, system light/dark
  themes (live), `Ctrl+1`…`Ctrl+8` shortcuts, live LED-strip / fan-curve / LCD previews and native
  colour pickers. The sidebar order is Dashboard, Lighting, LCD, Profiles, Cooling, Devices,
  Settings, About. The colour chooser forces a 450×460 size request (`set_show_editor(true)` plus
  `set_size_request`), the smallest measured size that shows the full editor without scrollbars,
  because GTK 4.22 ignores `set_default_size` and maps the dialog too small. Profiles has an
  **Export all…** action that writes the whole `ProfileDocument` as pretty JSON. About is a
  standalone page with the application name, window/service versions, GPL-3.0-or-later licence
  summary and GitHub links.
- The UI uses square surfaces (`border-radius: 0` on cards, rows, pills and previews) with a small
  4 px radius reserved for buttons and inputs, and a darkened teal primary (`#00a884`, hover
  `#00916f`) so buttons are not too bright. Breeze paints a closed dropdown's internal list item
  with its bright blue `:selected` colour; `dropdown row/listitem { background: transparent }`
  keeps the control flat. Popup hover/selected rows use `@text_primary` so they are white on dark
  and dark on light. The window clears GTK's initial focus after mapping, and focus outlines use
  the teal accent instead of Breeze blue. The project is licensed GPL-3.0-or-later, with the full
  text in `LICENSE`.
  Tokens live in `crates/mylianpc-ui/src/style.css` and `assets/design-tokens.json`; the spec is
  `docs/design-system.md`; shared component helpers are `crates/mylianpc-ui/src/ui_kit.rs`.

## Layout

| Change | Start here |
|---|---|
| Colours, hex parsing, brightness, HSV | `crates/mylianpc-core/src/color.rs` |
| Effects, zone validation, animation frames | `crates/mylianpc-core/src/effect.rs` |
| Fan curves, presets, hysteresis, failsafe | `crates/mylianpc-core/src/fan.rs` |
| Profiles, LCD settings, atomic JSON store | `crates/mylianpc-core/src/profile.rs` |
| Device/zone/apply models, effect fitting | `crates/mylianpc-core/src/device.rs` |
| Device settings / user preferences | `crates/mylianpc-core/src/settings.rs` |
| Network model and traffic maths | `crates/mylianpc-core/src/network.rs` |
| IPC request/response contract | `crates/mylianpc-core/src/ipc.rs` |
| Metric keys | `crates/mylianpc-core/src/metrics.rs` |
| USB/HID/I2C/hwmon discovery | `crates/mylianpc-hardware/src/inventory.rs` |
| Sensor mapping, NVML/RAPL/NVMe/SMBIOS | `crates/mylianpc-hardware/src/sensors.rs` |
| HydroShift packet format | `crates/mylianpc-hardware/src/hydroshift2.rs` |
| HydroShift USB transport | `crates/mylianpc-hardware/src/hydroshift_device.rs`, `usbfs.rs` |
| TinyUZ | `crates/mylianpc-hardware/src/tinyuz.rs` |
| Fusion2 packet builders/controller | `crates/mylianpc-hardware/src/fusion2.rs` |
| SMBus transport | `crates/mylianpc-hardware/src/smbus.rs` |
| Corsair DRAM detection/writes/recovery | `crates/mylianpc-hardware/src/corsair.rs` |
| NVML / RAPL / NVMe / SMBIOS | `nvml.rs`, `rapl.rs`, `nvme.rs`, `smbios.rs` |
| Linux network information | `crates/mylianpc-hardware/src/network.rs` |
| LCD themes/media/rendering | `crates/mylianpc-lcd/` |
| Adapters and service loops | `crates/mylianpc-service/src/adapters.rs`, `manager.rs` |
| IPC server and request validation | `crates/mylianpc-service/src/server.rs` |
| GTK4 UI pages | `crates/mylianpc-ui/src/pages/` |
| Read-only CLI | `crates/mylianpc-probe/src/main.rs` |
| udev rules, unit, install scripts | `packaging/`, root `install.sh` |

## Porting rules

- Port deterministically from the C# file named in each module's doc comment. Do not re-derive
  packet layouts, rounding, or effect phase maths.
- Keep generated animations at most `MAX_FRAMES` (48) and seamless.
- Every ported behaviour gets a unit test. The original suite is the model; the important ones
  are colour parsing, effect frame counts, fan-curve validation/evaluation, hysteresis and
  failsafe.
- When adding a protocol, add a pure encoder/decoder module with tests before any transport.
- Do not add a runtime plugin loader; adapters are compiled in, as in the original.

## Verification

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace
cargo run -p mylianpc-probe -- inventory
cargo run -p mylianpc-probe -- targets
sg mylianpc -c 'cargo run -p mylianpc-probe -- fusion2'
sg mylianpc -c 'cargo run -p mylianpc-probe -- hydroshift'   # read-only version/telemetry
sudo -n env MYLIANPC_DATA_DIR=/tmp/mylianpc-test \
  cargo run -p mylianpc-service -- --status                  # root-only sensors
```

`--status` and `--ipc-only` never initialise a device. Lighting, LCD, pump and fan writes are
implemented but must not be run for verification without the user's explicit approval. Say so
explicitly instead of claiming a device works.

## Windows reference files

- `../MyLianPC/src/MyLianPC.Core/Lighting/{RgbColor,LightingModels,EffectFrames}.cs`
- `../MyLianPC/src/MyLianPC.Core/Cooling/{FanCurve,CoolingPresets}.cs`
- `../MyLianPC/src/MyLianPC.Hardware/` — Fusion2, Corsair DRAM, HydroShift transports
- `../MyLianPC/docs/COMPATIBILITY.md` — per-device verified behaviour and failure modes
- `../MyLianPC/.claude/skills/mylianpc/SKILL.md` — the original project's hardware story
