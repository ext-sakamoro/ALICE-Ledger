//! `FIX 4.4` (Financial Information eXchange) protocol parser subset.
//!
//! Handles the wire representation `tag=value` records separated by `SOH`
//! (`0x01`), the required envelope fields (`8` = `BeginString`, `9` =
//! `BodyLength`, `10` = `CheckSum`), and typed access to the subset of tags
//! required by SPACID / MiFID-II reporting scenarios:
//!
//! - `NewOrderSingle` (`35=D`)
//! - `ExecutionReport` (`35=8`)
//!
//! The parser does not perform semantic validation beyond structural
//! integrity and checksum verification.

use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Field separator and constants
// ---------------------------------------------------------------------------

/// `SOH` byte separating `tag=value` fields.
pub const SOH: u8 = 0x01;

/// `BeginString` value indicating `FIX 4.4`.
pub const FIX_44_BEGIN_STRING: &str = "FIX.4.4";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// `FIX` parser errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixError {
    /// A field lacked the required `tag=value` structure.
    MalformedField(String),
    /// A required envelope field was missing.
    MissingField(u32),
    /// `BodyLength` did not match the counted bytes.
    BodyLengthMismatch { expected: usize, actual: usize },
    /// `CheckSum` did not match the computed checksum.
    ChecksumMismatch { expected: u32, actual: u32 },
    /// A field tag could not be parsed as an integer.
    TagNotInteger(String),
}

// ---------------------------------------------------------------------------
// FixMessage
// ---------------------------------------------------------------------------

/// A parsed `FIX` message keyed by tag number.
///
/// `BTreeMap` chosen for deterministic iteration during hashing / logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixMessage {
    pub fields: BTreeMap<u32, String>,
}

impl FixMessage {
    /// Empty message.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fields: BTreeMap::new(),
        }
    }

    /// Read a field.
    #[must_use]
    pub fn get(&self, tag: u32) -> Option<&str> {
        self.fields.get(&tag).map(String::as_str)
    }

    /// Insert or overwrite a field.
    pub fn insert(&mut self, tag: u32, value: impl Into<String>) {
        self.fields.insert(tag, value.into());
    }

    /// Convenience: `35` = `MsgType`.
    #[must_use]
    pub fn msg_type(&self) -> Option<&str> {
        self.get(35)
    }
}

impl Default for FixMessage {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a raw `FIX` wire buffer, validating `BodyLength` and `CheckSum`.
///
/// # Errors
///
/// See [`FixError`].
pub fn parse(bytes: &[u8]) -> Result<FixMessage, FixError> {
    let msg = parse_fields(bytes)?;

    // Envelope validation.
    let begin = msg.get(8).ok_or(FixError::MissingField(8))?.to_owned();
    let body_len_str = msg.get(9).ok_or(FixError::MissingField(9))?;
    let checksum_str = msg.get(10).ok_or(FixError::MissingField(10))?;

    let body_len: usize = body_len_str
        .parse()
        .map_err(|_| FixError::MalformedField(body_len_str.to_owned()))?;
    let expected_checksum: u32 = checksum_str
        .parse()
        .map_err(|_| FixError::MalformedField(checksum_str.to_owned()))?;

    // BodyLength is the count of bytes from just after `9=<n>\x01` to the
    // byte immediately preceding `10=`.
    let (body_bytes, checksum_bytes) = split_for_verification(bytes)?;
    if body_bytes.len() != body_len {
        return Err(FixError::BodyLengthMismatch {
            expected: body_len,
            actual: body_bytes.len(),
        });
    }
    let actual_checksum = checksum_bytes.iter().map(|b| u32::from(*b)).sum::<u32>() % 256;
    if actual_checksum != expected_checksum {
        return Err(FixError::ChecksumMismatch {
            expected: expected_checksum,
            actual: actual_checksum,
        });
    }
    // Sanity: BeginString must be non-empty.
    if begin.is_empty() {
        return Err(FixError::MissingField(8));
    }
    Ok(msg)
}

fn parse_fields(bytes: &[u8]) -> Result<FixMessage, FixError> {
    let mut msg = FixMessage::new();
    for chunk in bytes.split(|b| *b == SOH) {
        if chunk.is_empty() {
            continue;
        }
        let s = std::str::from_utf8(chunk)
            .map_err(|_| FixError::MalformedField(String::from_utf8_lossy(chunk).into_owned()))?;
        let (tag_str, value) = s
            .split_once('=')
            .ok_or_else(|| FixError::MalformedField(s.to_owned()))?;
        let tag: u32 = tag_str
            .parse()
            .map_err(|_| FixError::TagNotInteger(tag_str.to_owned()))?;
        msg.insert(tag, value);
    }
    Ok(msg)
}

fn split_for_verification(bytes: &[u8]) -> Result<(&[u8], &[u8]), FixError> {
    // The body starts after `9=<n>\x01` and ends just before `10=`.
    // We split by finding the first SOH after tag 9 and then the position
    // of the last `10=` (with leading SOH).
    let mut body_start = None;
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx..].starts_with(b"9=") {
            let after_eq = idx + 2;
            if let Some(off) = bytes[after_eq..].iter().position(|b| *b == SOH) {
                body_start = Some(after_eq + off + 1);
                break;
            }
        }
        idx += 1;
    }
    let body_start = body_start.ok_or(FixError::MissingField(9))?;

    let mut checksum_start = None;
    for i in (0..bytes.len()).rev() {
        if bytes[i..].starts_with(b"10=") && (i == 0 || bytes[i - 1] == SOH) {
            checksum_start = Some(i);
            break;
        }
    }
    let checksum_start = checksum_start.ok_or(FixError::MissingField(10))?;

    Ok((&bytes[body_start..checksum_start], &bytes[..checksum_start]))
}

// ---------------------------------------------------------------------------
// Serialisation helpers
// ---------------------------------------------------------------------------

/// Serialise a `FixMessage` on the wire and compute `BodyLength` +
/// `CheckSum`. `BeginString` (`8`) must already be present; `9` and `10`
/// are overwritten.
#[must_use]
pub fn serialize(msg: &FixMessage) -> Vec<u8> {
    let mut cloned = msg.clone();
    let begin = cloned
        .get(8)
        .map_or_else(|| FIX_44_BEGIN_STRING.to_owned(), String::from);
    cloned.fields.remove(&9);
    cloned.fields.remove(&10);

    // Serialise the body first (tags in BTree order).
    let mut body = Vec::new();
    for (tag, value) in &cloned.fields {
        if *tag == 8 {
            continue;
        }
        push_field(&mut body, *tag, value);
    }
    let body_len = body.len();

    let mut out = Vec::with_capacity(body_len + 32);
    push_field(&mut out, 8, &begin);
    push_field(&mut out, 9, &body_len.to_string());
    out.extend_from_slice(&body);
    // CheckSum covers the bytes so far.
    let cs: u32 = out.iter().map(|b| u32::from(*b)).sum::<u32>() % 256;
    push_field(&mut out, 10, &format!("{cs:03}"));
    out
}

fn push_field(buf: &mut Vec<u8>, tag: u32, value: &str) {
    buf.extend_from_slice(tag.to_string().as_bytes());
    buf.push(b'=');
    buf.extend_from_slice(value.as_bytes());
    buf.push(SOH);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_new_order_single() -> FixMessage {
        let mut m = FixMessage::new();
        m.insert(8, FIX_44_BEGIN_STRING);
        m.insert(35, "D");
        m.insert(49, "SENDER");
        m.insert(56, "TARGET");
        m.insert(11, "ORD-001");
        m.insert(55, "AAPL");
        m.insert(54, "1");
        m.insert(38, "100");
        m.insert(40, "2");
        m.insert(44, "150.25");
        m
    }

    #[test]
    fn serialize_then_parse_roundtrip() {
        let msg = sample_new_order_single();
        let wire = serialize(&msg);
        let parsed = parse(&wire).expect("parse should succeed");
        assert_eq!(parsed.msg_type(), Some("D"));
        assert_eq!(parsed.get(11), Some("ORD-001"));
        assert_eq!(parsed.get(38), Some("100"));
    }

    #[test]
    fn body_length_is_computed_correctly() {
        let msg = sample_new_order_single();
        let wire = serialize(&msg);
        let parsed = parse(&wire).unwrap();
        // Manually compute the count of body bytes and compare.
        let body_len: usize = parsed.get(9).unwrap().parse().unwrap();
        let (body, _) = split_for_verification(&wire).unwrap();
        assert_eq!(body.len(), body_len);
    }

    #[test]
    fn checksum_mismatch_is_reported() {
        let msg = sample_new_order_single();
        let mut wire = serialize(&msg);
        // Corrupt one byte of the payload.
        let idx = wire.iter().position(|b| *b == b'D').unwrap();
        wire[idx] = b'F';
        let err = parse(&wire).unwrap_err();
        assert!(matches!(err, FixError::ChecksumMismatch { .. }));
    }

    #[test]
    fn missing_begin_string_is_reported() {
        let wire = b"9=10\x0135=D\x0110=056\x01";
        let err = parse(wire).unwrap_err();
        assert!(matches!(err, FixError::MissingField(8)));
    }

    #[test]
    fn malformed_field_is_reported() {
        let wire = b"8=FIX.4.4\x019=5\x01hello\x0110=100\x01";
        let err = parse(wire).unwrap_err();
        assert!(matches!(err, FixError::MalformedField(_)));
    }

    #[test]
    fn execution_report_parses() {
        let mut m = FixMessage::new();
        m.insert(8, FIX_44_BEGIN_STRING);
        m.insert(35, "8");
        m.insert(37, "EXEC-001");
        m.insert(17, "FILL-001");
        m.insert(150, "F"); // Fill
        m.insert(39, "2"); // Filled
        m.insert(55, "AAPL");
        m.insert(38, "100");
        m.insert(31, "150.25");
        m.insert(32, "100");
        let wire = serialize(&m);
        let parsed = parse(&wire).unwrap();
        assert_eq!(parsed.msg_type(), Some("8"));
        assert_eq!(parsed.get(150), Some("F"));
    }

    #[test]
    fn insert_and_get_roundtrip() {
        let mut m = FixMessage::new();
        m.insert(999, "value");
        assert_eq!(m.get(999), Some("value"));
    }

    #[test]
    fn parse_rejects_body_length_mismatch() {
        // Craft a wire where BodyLength claims 99 but the body is short.
        let wire = b"8=FIX.4.4\x019=99\x0135=D\x0110=100\x01";
        let err = parse(wire).unwrap_err();
        assert!(matches!(
            err,
            FixError::BodyLengthMismatch { .. } | FixError::ChecksumMismatch { .. }
        ));
    }
}
