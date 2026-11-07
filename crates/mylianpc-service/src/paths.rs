//! Machine-wide data folder.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ServicePaths {
    pub root: PathBuf,
}

impl ServicePaths {
    pub fn resolve() -> Self {
        if let Ok(dir) = std::env::var("MYLIANPC_DATA_DIR") {
            if !dir.trim().is_empty() {
                return Self {
                    root: PathBuf::from(dir),
                };
            }
        }
        if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
            if !dir.trim().is_empty() {
                return Self {
                    root: PathBuf::from(dir).join("mylianpc"),
                };
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            if !home.trim().is_empty() {
                return Self {
                    root: PathBuf::from(home)
                        .join(".local")
                        .join("share")
                        .join("mylianpc"),
                };
            }
        }
        Self {
            root: PathBuf::from("/var/lib/mylianpc"),
        }
    }

    pub fn profiles(&self) -> PathBuf {
        self.root.join("profiles.json")
    }

    pub fn devices(&self) -> PathBuf {
        self.root.join("devices.json")
    }

    pub fn network(&self) -> PathBuf {
        self.root.join("network.json")
    }

    pub fn media(&self) -> PathBuf {
        self.root.join("media")
    }

    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(self.media())?;
        std::fs::create_dir_all(self.logs())?;
        Ok(())
    }
}
