//! The paired device store.
//!
//! A device is a phone that may connect. The store keeps the pairing secret
//! of every device, so the server can authenticate it, and the capabilities
//! it was granted.
//!
//! The file is written atomically with mode `0600` inside a `0700`
//! directory, so no other local account can read the pairing secrets.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Capability bits, mirrored from the protocol package.
pub const CAP_VIEW: u8 = 1 << 0;
pub const CAP_CONTROL: u8 = 1 << 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceEntry {
    /// Hex identifier, the same one the phone uses.
    pub id: String,
    /// Name the phone reported.
    #[serde(default)]
    pub name: String,
    /// Hex pairing secret.
    pub psk: String,
    /// Capability bits granted to the device.
    #[serde(default = "default_caps")]
    pub caps: u8,
    /// False until the phone completed its first handshake.
    #[serde(default)]
    pub paired: bool,
    /// A revoked device cannot connect, even with the right secret.
    #[serde(default)]
    pub revoked: bool,
    /// RFC 3339 timestamp of the pairing.
    #[serde(default)]
    pub created_at: String,
}

fn default_caps() -> u8 {
    CAP_VIEW | CAP_CONTROL
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceStore {
    #[serde(default)]
    pub devices: Vec<DeviceEntry>,
}

impl DeviceStore {
    pub fn load(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(DeviceStore::default());
        }
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Writes the store atomically, with owner only permissions.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            set_mode(parent, 0o700)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        let temp = path.with_extension("json.tmp");
        write_private(&temp, text.as_bytes())?;
        std::fs::rename(&temp, path)?;
        set_mode(path, 0o600)
    }

    pub fn find(&self, id: &str) -> Option<&DeviceEntry> {
        self.devices.iter().find(|entry| entry.id == id)
    }

    pub fn upsert(&mut self, entry: DeviceEntry) {
        match self
            .devices
            .iter_mut()
            .find(|existing| existing.id == entry.id)
        {
            Some(existing) => *existing = entry,
            None => self.devices.push(entry),
        }
    }

    pub fn paired_count(&self) -> usize {
        self.devices.iter().filter(|entry| entry.paired).count()
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> DeviceEntry {
        DeviceEntry {
            id: id.to_string(),
            name: "phone".to_string(),
            psk: "aa".repeat(32),
            caps: CAP_VIEW,
            paired: false,
            revoked: false,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn a_store_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("kvm-test-{}", std::process::id()));
        let path = dir.join("devices.json");
        let store = DeviceStore {
            devices: vec![entry("aa")],
        };
        store.save(&path).unwrap();

        let loaded = DeviceStore::load(&path).unwrap();
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.find("aa").unwrap().name, "phone");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_upsert_replaces_the_entry() {
        let mut store = DeviceStore {
            devices: vec![entry("aa")],
        };
        let mut updated = entry("aa");
        updated.paired = true;
        store.upsert(updated);
        assert_eq!(store.devices.len(), 1);
        assert!(store.find("aa").unwrap().paired);
    }

    #[cfg(unix)]
    #[test]
    fn the_store_file_is_not_readable_by_other_users() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("kvm-mode-{}", std::process::id()));
        let path = dir.join("devices.json");
        DeviceStore::default().save(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_dir_all(&dir).ok();
    }
}
