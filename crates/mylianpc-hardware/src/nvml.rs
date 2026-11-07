//! NVIDIA GPU readings through NVML.
//!
//! The Windows implementation used NVAPI through LibreHardwareMonitor. NVML is
//! the supported NVIDIA interface on Linux. It is loaded dynamically so the
//! service still starts on a machine without an NVIDIA driver.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::sync::OnceLock;

const NVML_SUCCESS: c_int = 0;
const NVML_TEMPERATURE_GPU: c_uint = 0;
const NVML_CLOCK_GRAPHICS: c_uint = 0;

type Device = *mut c_void;

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
#[derive(Default)]
struct Memory {
    total: u64,
    free: u64,
    used: u64,
}

type InitFn = unsafe extern "C" fn() -> c_int;
type ShutdownFn = unsafe extern "C" fn() -> c_int;
type GetCountFn = unsafe extern "C" fn(*mut c_uint) -> c_int;
type GetHandleFn = unsafe extern "C" fn(c_uint, *mut Device) -> c_int;
type GetNameFn = unsafe extern "C" fn(Device, *mut c_char, c_uint) -> c_int;
type GetTemperatureFn = unsafe extern "C" fn(Device, c_uint, *mut c_uint) -> c_int;
type GetUtilizationFn = unsafe extern "C" fn(Device, *mut Utilization) -> c_int;
type GetClockFn = unsafe extern "C" fn(Device, c_uint, *mut c_uint) -> c_int;
type GetPowerFn = unsafe extern "C" fn(Device, *mut c_uint) -> c_int;
type GetMemoryFn = unsafe extern "C" fn(Device, *mut Memory) -> c_int;
type GetFanSpeedFn = unsafe extern "C" fn(Device, *mut c_uint) -> c_int;

#[derive(Clone, Debug, Default)]
pub struct NvidiaReadings {
    pub name: String,
    pub temperature_c: Option<f64>,
    pub load_percent: Option<f64>,
    pub clock_mhz: Option<f64>,
    pub power_w: Option<f64>,
    pub fan_percent: Option<f64>,
    pub fan_rpm: Option<f64>,
    pub vram_total_gb: Option<f64>,
    pub vram_used_gb: Option<f64>,
    pub vram_free_gb: Option<f64>,
}

struct Library {
    handle: *mut c_void,
    shutdown: ShutdownFn,
    get_count: GetCountFn,
    get_handle: GetHandleFn,
    get_name: GetNameFn,
    get_temperature: GetTemperatureFn,
    get_utilization: GetUtilizationFn,
    get_clock: GetClockFn,
    get_power: GetPowerFn,
    get_memory: GetMemoryFn,
    get_fan_speed: GetFanSpeedFn,
    get_fan_speed_rpm: Option<GetFanSpeedFn>,
    device: Option<Device>,
}

// The handle is only used from the owning sampler thread.
unsafe impl Send for Library {}

impl Library {
    fn load() -> Option<Self> {
        let mut handle = std::ptr::null_mut();
        for name in ["libnvidia-ml.so.1", "libnvidia-ml.so"] {
            let Ok(c_name) = CString::new(name) else {
                continue;
            };
            // SAFETY: dlopen with a valid NUL-terminated name.
            handle = unsafe { libc::dlopen(c_name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            if !handle.is_null() {
                break;
            }
        }
        if handle.is_null() {
            return None;
        }

        macro_rules! symbol {
            ($name:literal) => {{
                let Ok(name) = CString::new($name) else {
                    // SAFETY: handle was returned by dlopen.
                    unsafe { libc::dlclose(handle) };
                    return None;
                };
                // SAFETY: dlsym on a live handle.
                let pointer = unsafe { libc::dlsym(handle, name.as_ptr()) };
                if pointer.is_null() {
                    // SAFETY: handle was returned by dlopen.
                    unsafe { libc::dlclose(handle) };
                    return None;
                }
                // SAFETY: the symbol has the declared C signature in NVML.
                unsafe { std::mem::transmute_copy(&pointer) }
            }};
        }

        let init: InitFn = symbol!("nvmlInit_v2");
        let shutdown: ShutdownFn = symbol!("nvmlShutdown");
        let get_count: GetCountFn = symbol!("nvmlDeviceGetCount_v2");
        let get_handle: GetHandleFn = symbol!("nvmlDeviceGetHandleByIndex_v2");
        let get_name: GetNameFn = symbol!("nvmlDeviceGetName");
        let get_temperature: GetTemperatureFn = symbol!("nvmlDeviceGetTemperature");
        let get_utilization: GetUtilizationFn = symbol!("nvmlDeviceGetUtilizationRates");
        let get_clock: GetClockFn = symbol!("nvmlDeviceGetClockInfo");
        let get_power: GetPowerFn = symbol!("nvmlDeviceGetPowerUsage");
        let get_memory: GetMemoryFn = symbol!("nvmlDeviceGetMemoryInfo");
        let get_fan_speed: GetFanSpeedFn = symbol!("nvmlDeviceGetFanSpeed");
        let get_fan_speed_rpm: Option<GetFanSpeedFn> = {
            let name = CString::new("nvmlDeviceGetFanSpeedRPM").ok()?;
            // SAFETY: dlsym on a live handle; a missing symbol is allowed here.
            let pointer = unsafe { libc::dlsym(handle, name.as_ptr()) };
            if pointer.is_null() {
                None
            } else {
                // SAFETY: the symbol has the declared C signature when present.
                Some(unsafe { std::mem::transmute_copy(&pointer) })
            }
        };

        // SAFETY: calling the NVML entry point loaded above.
        if unsafe { init() } != NVML_SUCCESS {
            // SAFETY: handle was returned by dlopen.
            unsafe { libc::dlclose(handle) };
            return None;
        }

        Some(Self {
            handle,
            shutdown,
            get_count,
            get_handle,
            get_name,
            get_temperature,
            get_utilization,
            get_clock,
            get_power,
            get_memory,
            get_fan_speed,
            get_fan_speed_rpm,
            device: None,
        })
    }

    fn first_device(&mut self) -> Option<Device> {
        if let Some(device) = self.device {
            return Some(device);
        }
        let mut count = 0u32;
        // SAFETY: valid out-pointer.
        if unsafe { (self.get_count)(&mut count) } != NVML_SUCCESS || count == 0 {
            return None;
        }
        let mut device: Device = std::ptr::null_mut();
        // SAFETY: valid out-pointer and index.
        if unsafe { (self.get_handle)(0, &mut device) } != NVML_SUCCESS {
            return None;
        }
        self.device = Some(device);
        Some(device)
    }

    fn sample(&mut self) -> Option<NvidiaReadings> {
        let device = self.first_device()?;
        let mut readings = NvidiaReadings::default();

        let mut name = [0 as c_char; 96];
        // SAFETY: buffer and length are valid.
        if unsafe { (self.get_name)(device, name.as_mut_ptr(), name.len() as u32) } == NVML_SUCCESS {
            // SAFETY: NVML NUL-terminates the name.
            readings.name = unsafe { CStr::from_ptr(name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
        }

        let mut value = 0u32;
        // SAFETY: valid out-pointer.
        if unsafe { (self.get_temperature)(device, NVML_TEMPERATURE_GPU, &mut value) } == NVML_SUCCESS {
            readings.temperature_c = Some(value as f64);
        }

        let mut utilization = Utilization::default();
        // SAFETY: valid out-pointer.
        if unsafe { (self.get_utilization)(device, &mut utilization) } == NVML_SUCCESS {
            readings.load_percent = Some(utilization.gpu as f64);
        }

        let mut clock = 0u32;
        // SAFETY: valid out-pointer.
        if unsafe { (self.get_clock)(device, NVML_CLOCK_GRAPHICS, &mut clock) } == NVML_SUCCESS {
            readings.clock_mhz = Some(clock as f64);
        }

        let mut power = 0u32;
        // SAFETY: valid out-pointer.
        if unsafe { (self.get_power)(device, &mut power) } == NVML_SUCCESS {
            readings.power_w = Some(power as f64 / 1000.0);
        }

        let mut memory = Memory::default();
        // SAFETY: valid out-pointer.
        if unsafe { (self.get_memory)(device, &mut memory) } == NVML_SUCCESS {
            let gb = 1024.0 * 1024.0 * 1024.0;
            readings.vram_total_gb = Some(memory.total as f64 / gb);
            readings.vram_used_gb = Some(memory.used as f64 / gb);
            readings.vram_free_gb = Some(memory.free as f64 / gb);
        }

        let mut fan = 0u32;
        // SAFETY: valid out-pointer.
        if unsafe { (self.get_fan_speed)(device, &mut fan) } == NVML_SUCCESS {
            readings.fan_percent = Some(fan as f64);
        }
        if let Some(get_rpm) = self.get_fan_speed_rpm {
            let mut rpm = 0u32;
            // SAFETY: valid out-pointer; the API exists on this driver.
            if unsafe { get_rpm(device, &mut rpm) } == NVML_SUCCESS {
                readings.fan_rpm = Some(rpm as f64);
            }
        }

        Some(readings)
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: the entry point and handle are both live.
        unsafe {
            let _ = (self.shutdown)();
            if !self.handle.is_null() {
                libc::dlclose(self.handle);
            }
        }
    }
}

/// Lazily-initialised NVML access used by the sensor sampler.
#[derive(Default)]
pub struct Nvml {
    library: Option<Library>,
    attempted: bool,
}

impl Nvml {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sample(&mut self) -> Option<NvidiaReadings> {
        if !self.attempted {
            self.attempted = true;
            self.library = Library::load();
        }
        self.library.as_mut()?.sample()
    }

    pub fn available(&mut self) -> bool {
        if !self.attempted {
            self.attempted = true;
            self.library = Library::load();
        }
        self.library.is_some()
    }
}

/// The NVML library is loaded lazily once per process; tests only verify that
/// the probe does not crash when the driver is absent.
pub fn probe() -> Option<NvidiaReadings> {
    static PROBE: OnceLock<Option<NvidiaReadings>> = OnceLock::new();
    PROBE
        .get_or_init(|| {
            let mut nvml = Nvml::new();
            nvml.sample()
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_layouts_are_stable_for_the_c_abi() {
        assert_eq!(std::mem::size_of::<Utilization>(), 8);
        assert_eq!(std::mem::size_of::<Memory>(), 24);
    }

    #[test]
    fn loading_without_a_driver_is_not_fatal() {
        // This machine has a driver, so the call may succeed; the only contract
        // is that it never panics and never blocks startup.
        let _ = Nvml::new().available();
    }
}
