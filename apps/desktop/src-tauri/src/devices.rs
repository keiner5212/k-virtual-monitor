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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// When the pairing was created, as `unix:<seconds>`.
    #[serde(default)]
    pub created_at: String,
    /// Unix seconds when a pending pairing stops being usable. A pending
    /// entry always has one.
    #[serde(default)]
    pub expires_at: u64,
    /// Frame width this phone asked for. Zero means the server default.
    #[serde(default)]
    pub width: u16,
    /// Frame height this phone asked for. Zero means the server default.
    #[serde(default)]
    pub height: u16,
    /// Frame rate this phone asked for. Zero means the server default.
    #[serde(default)]
    pub fps: u16,
    /// JPEG quality for this phone. Zero means the server default.
    #[serde(default)]
    pub jpeg_quality: u8,
}

impl DeviceEntry {
    /// A pending pairing that ran out of time. A pending entry without an
    /// expiry was written by an older build and is dropped as well.
    pub fn is_expired(&self, now: u64) -> bool {
        !self.paired && now >= self.expires_at
    }
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

    /// Reads the file again. The file is the source of truth: a pairing code
    /// created by the command line has to be seen by a running server.
    pub fn reload(&mut self, path: &Path) -> io::Result<bool> {
        if !path.exists() {
            return Ok(false);
        }
        let loaded = Self::load(path)?;
        if loaded.devices == self.devices {
            return Ok(false);
        }
        self.devices = loaded.devices;
        Ok(true)
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

    /// The device that owns this pairing secret.
    pub fn find_by_psk(&self, psk: &str) -> Option<&DeviceEntry> {
        self.devices.iter().find(|entry| entry.psk == psk)
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

    /// Removes a device. The pairing secret goes with it.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|entry| entry.id != id);
        before != self.devices.len()
    }

    /// The pending pairing, if there is one.
    pub fn pending(&self) -> Option<&DeviceEntry> {
        self.devices
            .iter()
            .find(|entry| !entry.paired && !entry.revoked)
    }

    /// Removes the pending pairing, if there is one. Only one code is alive
    /// at a time, so a new one replaces the old.
    pub fn remove_pending(&mut self) -> Option<DeviceEntry> {
        let index = self
            .devices
            .iter()
            .position(|entry| !entry.paired && !entry.revoked)?;
        Some(self.devices.remove(index))
    }

    /// Drops the entries that cannot be used any more: the pending pairings
    /// that ran out of time and the revoked ones.
    pub fn prune(&mut self, now: u64) -> Vec<DeviceEntry> {
        let mut dropped = Vec::new();
        let mut kept = Vec::with_capacity(self.devices.len());
        for entry in self.devices.drain(..) {
            if entry.revoked || entry.is_expired(now) {
                dropped.push(entry);
            } else {
                kept.push(entry);
            }
        }
        self.devices = kept;
        dropped
    }

    pub fn paired_count(&self) -> usize {
        self.devices.iter().filter(|entry| entry.paired).count()
    }
}

/// The hex form of the bytes.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decodes a hex string. A corrupt value is an error, not a zero byte: a
/// device with a broken secret must not be able to connect.
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok())
        .collect()
}

/// The current time in unix seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
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
            expires_at: 0,
            width: 0,
            height: 0,
            fps: 0,
            jpeg_quality: 0,
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

    #[test]
    fn a_pending_entry_that_ran_out_of_time_is_dropped() {
        let mut store = DeviceStore {
            devices: vec![entry("aa")],
        };
        store.devices[0].expires_at = 100;
        let dropped = store.prune(100);
        assert_eq!(dropped.len(), 1);
        assert!(store.devices.is_empty());
    }

    #[test]
    fn a_paired_entry_does_not_expire() {
        let mut store = DeviceStore {
            devices: vec![entry("aa")],
        };
        store.devices[0].paired = true;
        store.devices[0].expires_at = 100;
        assert!(store.prune(1000).is_empty());
        assert_eq!(store.devices.len(), 1);
    }

    #[test]
    fn only_one_pending_pairing_is_kept() {
        let mut store = DeviceStore {
            devices: vec![entry("aa")],
        };
        store.remove_pending();
        assert!(store.devices.is_empty());
        assert!(store.pending().is_none());
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
