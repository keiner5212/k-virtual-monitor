//! The pairing blob.
//!
//! Pairing happens once per phone. The desktop shows a blob as a QR code and
//! the phone scans it. The blob carries the pairing secret, the device id of
//! the server and the address to connect to. The six digit safety code derived
//! from the same secret is shown on both devices, so the user can confirm
//! they paired with the right peer even if the QR was scanned by someone else.

use crate::crypto::{Psk, PSK_LEN};
use crate::error::ProtocolError;
use crate::limits::{MAX_NAME_LEN, SAS_DIGITS};

const PREFIX: &str = "kvm1:";
const VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingBlob {
    pub device_id: [u8; 16],
    pub psk: [u8; PSK_LEN],
    pub server_name: String,
    pub host: String,
    pub port: u16,
}

impl PairingBlob {
    pub fn new(device_id: [u8; 16], psk: Psk, server_name: &str, host: &str, port: u16) -> Self {
        PairingBlob {
            device_id,
            psk: *psk.as_bytes(),
            server_name: server_name.to_string(),
            host: host.to_string(),
            port,
        }
    }

    /// Encodes the blob as `kvm1:` followed by the hex of the binary form.
    pub fn encode(&self) -> String {
        let mut bytes = Vec::new();
        bytes.push(VERSION);
        bytes.extend_from_slice(&self.device_id);
        bytes.extend_from_slice(&self.psk);
        bytes.extend_from_slice(&self.port.to_be_bytes());
        for value in [&self.server_name, &self.host] {
            let raw = value.as_bytes();
            let len = raw.len().min(MAX_NAME_LEN);
            bytes.push(len as u8);
            bytes.extend_from_slice(&raw[..len]);
        }
        let mut out = String::with_capacity(PREFIX.len() + bytes.len() * 2);
        out.push_str(PREFIX);
        for byte in bytes {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    pub fn decode(text: &str) -> Result<Self, ProtocolError> {
        let hex = text
            .strip_prefix(PREFIX)
            .ok_or(ProtocolError::Handshake("pairing blob has a wrong prefix"))?;
        if hex.len() % 2 != 0 {
            return Err(ProtocolError::Handshake("pairing blob is not hex"));
        }
        let bytes = hex_decode(hex).ok_or(ProtocolError::Handshake("pairing blob is not hex"))?;
        let mut pos = 0usize;
        let version = *bytes
            .get(pos)
            .ok_or(ProtocolError::Handshake("truncated blob"))?;
        pos += 1;
        if version != VERSION {
            return Err(ProtocolError::Handshake("unsupported pairing blob version"));
        }
        let device_id: [u8; 16] = bytes
            .get(pos..pos + 16)
            .ok_or(ProtocolError::Handshake("truncated blob"))?
            .try_into()
            .expect("16 bytes");
        pos += 16;
        let psk: [u8; PSK_LEN] = bytes
            .get(pos..pos + PSK_LEN)
            .ok_or(ProtocolError::Handshake("truncated blob"))?
            .try_into()
            .expect("32 bytes");
        pos += PSK_LEN;
        let port = u16::from_be_bytes(
            bytes
                .get(pos..pos + 2)
                .ok_or(ProtocolError::Handshake("truncated blob"))?
                .try_into()
                .expect("2 bytes"),
        );
        pos += 2;
        let server_name = read_string(&bytes, &mut pos)?;
        let host = read_string(&bytes, &mut pos)?;
        Ok(PairingBlob {
            device_id,
            psk,
            server_name,
            host,
            port,
        })
    }

    pub fn psk(&self) -> Psk {
        Psk::from_bytes(self.psk)
    }

    /// The safety code, the same digits on both devices.
    pub fn safety_code(&self) -> u32 {
        crate::crypto::safety_code(&self.psk())
    }
}

fn read_string(bytes: &[u8], pos: &mut usize) -> Result<String, ProtocolError> {
    let len = *bytes
        .get(*pos)
        .ok_or(ProtocolError::Handshake("truncated blob"))? as usize;
    *pos += 1;
    let raw = bytes
        .get(*pos..*pos + len)
        .ok_or(ProtocolError::Handshake("truncated blob"))?;
    *pos += len;
    String::from_utf8(raw.to_vec()).map_err(|_| ProtocolError::Handshake("blob is not utf-8"))
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        if pair.len() != 2 {
            return None;
        }
        let high = (pair[0] as char).to_digit(16)?;
        let low = (pair[1] as char).to_digit(16)?;
        out.push((high * 16 + low) as u8);
    }
    Some(out)
}

/// Formats a safety code as two groups of three digits.
pub fn format_safety_code(code: u32) -> String {
    let digits = format!("{:0width$}", code, width = SAS_DIGITS as usize);
    format!("{} {}", &digits[..3], &digits[3..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_round_trip() {
        let blob = PairingBlob::new(
            [9u8; 16],
            Psk::from_bytes([4u8; 32]),
            "desktop",
            "192.168.1.10",
            28252,
        );
        let encoded = blob.encode();
        assert!(encoded.starts_with(PREFIX));
        assert_eq!(PairingBlob::decode(&encoded).unwrap(), blob);
    }

    #[test]
    fn a_wrong_prefix_is_rejected() {
        assert!(PairingBlob::decode("kvm2:00").is_err());
    }

    #[test]
    fn safety_code_is_formatted_in_two_groups() {
        assert_eq!(format_safety_code(1234), "001 234");
    }
}
