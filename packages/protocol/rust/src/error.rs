//! Errors surfaced by the protocol layer.
//!
//! Every error carries the operation that failed and the input that caused it,
//! so a session can be closed with a reason code instead of a generic string.

use std::fmt;

use crate::frame::MessageType;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("io error while {operation}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("unexpected end of stream while reading {what}")]
    UnexpectedEof { what: &'static str },

    #[error("bad magic: expected KM, got {0:02x?}")]
    BadMagic([u8; 2]),

    #[error("unsupported protocol version {0}, this build speaks {1}")]
    BadVersion(u8, u8),

    #[error("unknown message type {0}")]
    UnknownType(u8),

    #[error("payload of {kind} is {actual} bytes, limit is {limit}")]
    PayloadTooLarge {
        kind: &'static str,
        actual: usize,
        limit: usize,
    },

    #[error("payload of {kind} is {actual} bytes, expected {expected}")]
    PayloadSize {
        kind: &'static str,
        actual: usize,
        expected: usize,
    },

    #[error("sequence {received} is not the expected {expected}")]
    BadSequence { expected: u64, received: u64 },

    #[error("authentication tag mismatch on {kind}")]
    TagMismatch { kind: MessageType },

    #[error("proof mismatch on {kind}")]
    ProofMismatch { kind: MessageType },

    #[error("unknown device {0:?}")]
    UnknownDevice([u8; 16]),

    #[error("device {0:?} is not allowed to {1}")]
    NotAuthorized([u8; 16], &'static str),

    #[error("handshake failed: {0}")]
    Handshake(&'static str),

    #[error("protocol violation: {0}")]
    Violation(&'static str),
}

impl ProtocolError {
    pub fn io(operation: &'static str, source: std::io::Error) -> Self {
        ProtocolError::Io { operation, source }
    }
}

/// A short, stable code the client can branch on without parsing text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ErrorCode {
    BadRequest = 1,
    AuthFailed = 2,
    UnknownDevice = 3,
    NotAuthorized = 4,
    ProtocolViolation = 5,
    RateLimited = 6,
    ServerBusy = 7,
    Internal = 8,
    UnsupportedCodec = 9,
}

impl From<ErrorCode> for u16 {
    fn from(code: ErrorCode) -> Self {
        code as u16
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ErrorCode::BadRequest => "BAD_REQUEST",
            ErrorCode::AuthFailed => "AUTH_FAILED",
            ErrorCode::UnknownDevice => "UNKNOWN_DEVICE",
            ErrorCode::NotAuthorized => "NOT_AUTHORIZED",
            ErrorCode::ProtocolViolation => "PROTOCOL_VIOLATION",
            ErrorCode::RateLimited => "RATE_LIMITED",
            ErrorCode::ServerBusy => "SERVER_BUSY",
            ErrorCode::Internal => "INTERNAL",
            ErrorCode::UnsupportedCodec => "UNSUPPORTED_CODEC",
        };
        f.write_str(name)
    }
}
