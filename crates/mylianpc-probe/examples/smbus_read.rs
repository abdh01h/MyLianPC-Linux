use mylianpc_hardware::smbus::Smbus;

fn main() {
    for bus_number in [2u32, 3, 4] {
        match Smbus::open(bus_number) {
            Ok(bus) => {
                for address in [0x19u8, 0x1Bu8] {
                    match bus.read_byte_data(address, 0x43) {
                        Ok(value) => println!(
                            "i2c-{bus_number} 0x{address:02X} reg 0x43 = 0x{value:02X}"
                        ),
                        Err(error) => println!(
                            "i2c-{bus_number} 0x{address:02X} reg 0x43 error: {error}"
                        ),
                    }
                }
            }
            Err(error) => println!("open i2c-{bus_number}: {error}"),
        }
    }
}
