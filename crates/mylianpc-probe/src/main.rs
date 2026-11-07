//! Read-only hardware probe.
//!
//! This tool never writes to a device. It exists so the Linux port can be
//! validated against the real machine before any LED, LCD or fan command is
//! implemented.

use anyhow::Result;
use clap::{Parser, Subcommand};
use mylianpc_hardware::inventory::{self, Inventory, UsbDevice};

#[derive(Parser)]
#[command(
    name = "mylianpc-probe",
    version,
    about = "Read-only MyLianPC hardware probe (never writes to hardware)"
)]
struct Cli {
    /// Print JSON instead of a human-readable report.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Full read-only inventory (default).
    Inventory,
    /// USB devices, interfaces and endpoints.
    Usb,
    /// HID raw devices.
    Hid,
    /// I2C adapters and the AMD SMBus controller state.
    I2c,
    /// hwmon sensors.
    Sensors,
    /// Fusion2 identification query (no lighting change).
    Fusion2,
    /// HydroShift version and telemetry query (read-only, no wake or LCD change).
    Hydroshift,
    /// Corsair DRAM RGB detection over SMBus (read-only).
    Ram,
    /// Only the devices MyLianPC drives.
    Targets,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Inventory) {
        Command::Inventory => {
            let inventory = Inventory::collect();
            if cli.json {
                print_json(&inventory)
            } else {
                print_inventory(&inventory);
                Ok(())
            }
        }
        Command::Usb => {
            let devices = inventory::usb_devices();
            if cli.json {
                print_json(&devices)
            } else {
                print_usb(&devices);
                Ok(())
            }
        }
        Command::Hid => {
            let devices = inventory::hidraw_devices();
            if cli.json {
                print_json(&devices)
            } else {
                print_hid(&devices);
                Ok(())
            }
        }
        Command::I2c => {
            let adapters = inventory::i2c_adapters();
            let smbus = inventory::smbus_status();
            if cli.json {
                let value = serde_json::json!({ "adapters": adapters, "smbus": smbus });
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                print_i2c(&adapters, &smbus);
            }
            Ok(())
        }
        Command::Sensors => {
            let devices = inventory::hwmon_devices();
            if cli.json {
                print_json(&devices)
            } else {
                print_sensors(&devices);
                Ok(())
            }
        }
        Command::Fusion2 => {
            let hid = mylianpc_hardware::fusion2::find_vendor_hidraw()?;
            let info = mylianpc_hardware::fusion2::query_info(&hid, 0x5711)?;
            if cli.json {
                print_json(&info)
            } else {
                print_fusion2(&info);
                Ok(())
            }
        }
        Command::Hydroshift => {
            let mut device = mylianpc_hardware::hydroshift_device::HydroShiftDevice::open()?;
            let (version, telemetry) = device.query_only()?;
            if cli.json {
                let value = serde_json::json!({
                    "firmware": version,
                    "telemetry": telemetry,
                });
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                println!("HydroShift II read-only query (no LCD, ring or fan write)");
                println!("  firmware : {}", version.as_deref().unwrap_or("?"));
                match telemetry {
                    Some(telemetry) => {
                        println!("  coolant  : {} °C", telemetry.coolant_c);
                        println!(
                            "  fans     : {} / {} / {} rpm",
                            telemetry.fan_rpm[0], telemetry.fan_rpm[1], telemetry.fan_rpm[2]
                        );
                        println!("  pump     : {} rpm", telemetry.pump_rpm);
                        println!("  MAC      : {}", telemetry.mac);
                        println!(
                            "  lighting fans: {}",
                            telemetry
                                .lighting_fan_count
                                .map(|count| count.to_string())
                                .unwrap_or_else(|| "not reported".to_string())
                        );
                    }
                    None => println!("  telemetry: no plausible reply"),
                }
            }
            Ok(())
        }
        Command::Ram => {
            let buses = mylianpc_hardware::corsair::detect_all();
            if cli.json {
                print_json(&buses)
            } else {
                print_ram(&buses);
                Ok(())
            }
        }
        Command::Targets => {
            let inventory = Inventory::collect();
            if cli.json {
                print_json(&inventory.targets)
            } else {
                print_targets(&inventory);
                Ok(())
            }
        }
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn mark(present: bool) -> &'static str {
    if present {
        "OK"
    } else {
        "MISSING"
    }
}

fn print_inventory(inv: &Inventory) {
    println!("MyLianPC Linux probe - READ ONLY (no hardware writes)");
    println!();
    println!(
        "Board    : {} {} / {}",
        inv.dmi.board_vendor.as_deref().unwrap_or("?"),
        inv.dmi.board_name.as_deref().unwrap_or("?"),
        inv.dmi.product_name.as_deref().unwrap_or("?")
    );
    println!(
        "BIOS     : {}",
        inv.dmi.bios_version.as_deref().unwrap_or("?")
    );
    println!();
    print_targets(inv);
    println!();
    println!("USB devices : {}", inv.usb.len());
    println!("HID raw     : {}", inv.hidraw.len());
    println!("I2C adapters: {}", inv.i2c.len());
    println!("hwmon       : {}", inv.hwmon.len());
    println!();
    println!("Run `mylianpc-probe usb|hid|i2c|sensors` for details, or --json.");
}

fn print_targets(inv: &Inventory) {
    println!("Target devices");
    println!("  Fusion2 (048d:5711)      : {}", mark(inv.targets.fusion2.is_some()));
    if let Some(device) = &inv.targets.fusion2 {
        println!("    {}", describe_usb(device));
    }
    println!("  HydroShift (1cbe:a021)   : {}", mark(inv.targets.hydroship.is_some()));
    if let Some(device) = &inv.targets.hydroship {
        println!("    {}", describe_usb(device));
        if let Some(serial) = &device.serial {
            println!("    serial: {serial}");
        }
        for interface in &device.interfaces {
            for endpoint in &interface.endpoints {
                println!(
                    "    endpoint 0x{:02x} {} max packet {}",
                    endpoint.address, endpoint.kind, endpoint.max_packet
                );
            }
        }
    }
    println!(
        "  Corsair USB (1b1c:0901)  : {} (RAM is expected on SMBus)",
        mark(inv.targets.corsair_usb.is_some())
    );
    println!("  AMD SMBus (00:14.0)      : {}", mark(inv.targets.smbus_ready));
    if !inv.targets.smbus_ready {
        println!("    {}", inv.smbus.detail);
    }
    println!(
        "  IT8696E (it87 hwmon)     : {}",
        mark(inv.targets.it87.is_some())
    );
    if inv.targets.it87.is_none() {
        println!("    no it87 driver loaded; motherboard fan control is unavailable (monitor-only).");
    }
}

fn describe_usb(device: &UsbDevice) -> String {
    format!(
        "{} bus {} dev {}  {}",
        device.id(),
        device.bus,
        device.dev,
        device.product.as_deref().unwrap_or("(no product string)")
    )
}

fn print_usb(devices: &[UsbDevice]) {
    println!("USB devices (read-only)");
    for device in devices {
        println!(
            "{}  bus {:03} dev {:03}  {}  [{}]",
            device.id(),
            device.bus,
            device.dev,
            device.product.as_deref().unwrap_or("(no product string)"),
            device.serial.as_deref().unwrap_or("no serial")
        );
        for interface in &device.interfaces {
            println!(
                "  interface {} class {:02x}/{:02x}/{:02x} driver {}",
                interface.number,
                interface.class,
                interface.subclass,
                interface.protocol,
                interface.driver.as_deref().unwrap_or("(none)")
            );
            for endpoint in &interface.endpoints {
                println!(
                    "    endpoint 0x{:02x} {} max packet {} interval {}",
                    endpoint.address, endpoint.kind, endpoint.max_packet, endpoint.interval
                );
            }
        }
    }
}

fn print_hid(devices: &[inventory::HidRawDevice]) {
    println!("HID raw devices (read-only)");
    for device in devices {
        println!(
            "{}  {}  id {}  vid:pid {}:{}",
            device.node,
            device.name,
            device.hid_id,
            device.vid.as_deref().unwrap_or("?"),
            device.pid.as_deref().unwrap_or("?")
        );
        println!("  phys {}  uniq {}", device.phys, device.uniq);
        match &device.open_error {
            None => println!("  open: yes"),
            Some(error) => println!("  open: no ({error})"),
        }
    }
}

fn print_i2c(adapters: &[inventory::I2cAdapter], smbus: &inventory::SmbusStatus) {
    println!("I2C adapters");
    for adapter in adapters {
        println!(
            "  i2c-{:<2} {:<45} {}",
            adapter.number,
            adapter.name,
            if adapter.readable {
                "readable"
            } else {
                "permission denied"
            }
        );
    }
    println!();
    println!("SMBus controller {}", smbus.pci_address);
    println!("  PCI id       : {}", smbus.pci_id.as_deref().unwrap_or("?"));
    println!(
        "  driver       : {}",
        smbus.driver.as_deref().unwrap_or("(none bound)")
    );
    println!(
        "  i2c_piix4    : {}",
        if smbus.module_loaded {
            "loaded"
        } else {
            "not loaded"
        }
    );
    println!("  detail       : {}", smbus.detail);
}

fn print_fusion2(info: &mylianpc_hardware::fusion2::Fusion2Info) {
    println!("Fusion2 identification (read-only query, no lighting change)");
    println!("  hidraw       : {}", info.hidraw);
    println!("  product id   : 0x{:04X}", info.product_id);
    println!("  description  : {}", info.description);
    println!("  firmware     : {}", info.firmware);
    println!("  chip id      : 0x{:04X}", info.chip_id);
    println!("  device num   : {}", info.device_num);
    println!("  strip detect : {}", info.strip_detect);
    println!("  support flags: 0x{:02X}", info.support_flags);
    println!(
        "  byte order   : {}",
        info.byte_order
            .iter()
            .map(|value| format!("0x{value:08X}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

fn print_ram(buses: &[mylianpc_hardware::corsair::BusModules]) {
    println!("Corsair DRAM RGB detection (read-only; no recovery write)");
    if buses.is_empty() {
        println!("  no Corsair RGB controllers found on the SMBus PIIX4 adapters");
    }
    for bus in buses {
        println!("  i2c-{}  {}", bus.bus, bus.adapter);
        for module in &bus.modules {
            println!(
                "    0x{:02X}: {:04X}:{:04X} firmware {} protocol {}{}",
                module.address,
                module.vid,
                module.pid,
                module.firmware,
                module.protocol_version,
                if module.stalled {
                    "  STALLED (recovery write not performed)"
                } else {
                    ""
                }
            );
        }
    }
}

fn print_sensors(devices: &[inventory::HwmonDevice]) {
    println!("hwmon sensors (read-only)");
    for device in devices {
        println!("{}  ({})", device.name, device.path);
        for sensor in &device.sensors {
            println!(
                "  {:<12} {:<24} {:>10.2} {}",
                sensor.kind, sensor.label, sensor.value, sensor.unit
            );
        }
    }
}
