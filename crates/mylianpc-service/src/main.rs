//! MyLianPC service.
//!
//! The only process allowed to touch hardware. It owns the Fusion2 controller,
//! the HydroShift II, the Corsair DIMMs and the cooling loop, and serves the UI
//! over a Unix domain socket.
//!
//! Development:
//!   cargo run -p mylianpc-service              # serve on $XDG_RUNTIME_DIR/mylianpc/
//!   cargo run -p mylianpc-service -- --once    # print the read-only inventory
//!   cargo run -p mylianpc-service -- --status  # print status without connecting

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use mylianpc_core::ipc::socket_path;
use mylianpc_hardware::inventory::Inventory;

use mylianpc_service::manager::DeviceManager;
use mylianpc_service::paths::ServicePaths;
use mylianpc_service::server::serve_connection;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--once" || arg == "--inventory") {
        println!("{}", serde_json::to_string_pretty(&Inventory::collect())?);
        return Ok(());
    }

    let paths = ServicePaths::resolve();
    paths.ensure()?;
    let manager = Arc::new(Mutex::new(DeviceManager::new(paths.clone())?));

    if args.iter().any(|arg| arg == "--status") {
        let manager = manager.lock().expect("service state");
        println!("{}", serde_json::to_string_pretty(&manager.status())?);
        println!("\n{}", manager.diagnostics());
        return Ok(());
    }

    let path = socket_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    if path.exists() {
        // Refuse to steal a socket from a running service.
        if UnixStream::connect(&path).is_ok() {
            anyhow::bail!(
                "another mylianpc-service is already listening on {}",
                path.display()
            );
        }
        std::fs::remove_file(&path)
            .with_context(|| format!("removing stale socket {}", path.display()))?;
    }
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("binding {}", path.display()))?;
    listener.set_nonblocking(true)?;
    let mut permissions = std::fs::metadata(&path)?.permissions();
    permissions.set_mode(0o660);
    std::fs::set_permissions(&path, permissions)?;
    eprintln!("mylianpc-service listening on {}", path.display());

    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, Arc::clone(&stop))?;
    }

    // Development aid: serve IPC and sensors only. No device loop runs, so
    // starting this mode never initialises or writes a device. A write request
    // from a client still goes through the normal adapter code.
    let ipc_only = args.iter().any(|arg| arg == "--ipc-only");
    let handles = if ipc_only {
        eprintln!("ipc-only mode: background device loops are disabled");
        vec![spawn_loop(
            "sensors",
            Arc::clone(&stop),
            Arc::clone(&manager),
            500,
            |manager| manager.tick_sensors(),
        )]
    } else {
        vec![
            spawn_loop("sensors", Arc::clone(&stop), Arc::clone(&manager), 500, |manager| {
                manager.tick_sensors();
            }),
            spawn_loop("cooling", Arc::clone(&stop), Arc::clone(&manager), 2000, |manager| {
                manager.tick_cooling();
            }),
            spawn_loop("watch", Arc::clone(&stop), Arc::clone(&manager), 5000, |manager| {
                manager.tick_watch();
            }),
            spawn_loop("fast", Arc::clone(&stop), Arc::clone(&manager), 50, |manager| {
                manager.tick_fast();
            }),
            spawn_loop("persist", Arc::clone(&stop), Arc::clone(&manager), 30_000, |manager| {
                manager.save_network(false);
            }),
        ]
    };

    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _address)) => {
                let manager = Arc::clone(&manager);
                std::thread::spawn(move || {
                    if let Err(error) = serve_connection(stream, manager) {
                        eprintln!("client error: {error:#}");
                    }
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => eprintln!("accept failed: {error}"),
        }
    }

    eprintln!("mylianpc-service stopping");
    for handle in handles {
        let _ = handle.join();
    }
    manager
        .lock()
        .expect("service state")
        .shutdown();
    let _ = std::fs::remove_file(&path);
    Ok(())
}

fn spawn_loop(
    name: &'static str,
    stop: Arc<AtomicBool>,
    manager: Arc<Mutex<DeviceManager>>,
    interval_ms: u64,
    tick: impl Fn(&mut DeviceManager) + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name(format!("mylianpc-{name}"))
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let sleep = match manager.lock() {
                    Ok(mut manager) => {
                        tick(&mut manager);
                        if name == "sensors" {
                            manager.settings.sensor_interval_ms.max(500)
                        } else {
                            interval_ms
                        }
                    }
                    Err(error) => {
                        eprintln!("{name} loop: service state lock poisoned: {error}");
                        interval_ms
                    }
                };
                let deadline = std::time::Instant::now() + Duration::from_millis(sleep);
                while !stop.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        })
        .expect("spawning loop thread")
}

#[allow(dead_code)]
fn write_log(paths: &ServicePaths, message: &str) {
    let path = paths.logs().join("service.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{} {message}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}
