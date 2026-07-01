//! `Ed25519`-signed order envelope and MiFID-II style audit trail skeleton.
//!
//! Wraps the existing order primitives with a cryptographic envelope suitable
//! for regulatory reporting: each order carries an `Ed25519` signature over
//! its canonical byte representation, and the sequence of accepted orders is
//! captured in an append-only [`OrderAuditLog`] whose head hash can be
//! anchored into an external ledger.

use alice_blockchain::{hash_data, Hash, KeyPair, PublicKey, Signature};

// ---------------------------------------------------------------------------
// OrderSide / OrderPayload
// ---------------------------------------------------------------------------

/// Buy or sell side of an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    /// Raw byte tag used inside the canonical payload.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Buy => 1,
            Self::Sell => 2,
        }
    }
}

/// Signed-order payload — plain data structure without cryptographic material.
#[derive(Debug, Clone)]
pub struct OrderPayload {
    pub order_id: u64,
    pub trader_id: String,
    pub instrument: String,
    pub side: OrderSide,
    pub quantity: u64,
    pub limit_price_micros: u64,
    pub timestamp_unix: u64,
    pub venue: String,
}

impl OrderPayload {
    /// Canonical byte serialisation used for hashing and signing.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(128);
        buf.extend_from_slice(&self.order_id.to_le_bytes());
        push_len(&mut buf, self.trader_id.as_bytes());
        push_len(&mut buf, self.instrument.as_bytes());
        buf.push(self.side.tag());
        buf.extend_from_slice(&self.quantity.to_le_bytes());
        buf.extend_from_slice(&self.limit_price_micros.to_le_bytes());
        buf.extend_from_slice(&self.timestamp_unix.to_le_bytes());
        push_len(&mut buf, self.venue.as_bytes());
        buf
    }

    /// `SHA-256` digest.
    #[must_use]
    pub fn digest(&self) -> Hash {
        hash_data(&self.canonical_bytes())
    }
}

// ---------------------------------------------------------------------------
// SignedOrder
// ---------------------------------------------------------------------------

/// An [`OrderPayload`] committed to by an `Ed25519` signature.
#[derive(Debug, Clone)]
pub struct SignedOrder {
    pub payload: OrderPayload,
    pub trader_public: PublicKey,
    pub signature: Signature,
}

impl SignedOrder {
    /// Sign an order with the trader's key pair.
    #[must_use]
    pub fn sign(payload: OrderPayload, trader: &KeyPair) -> Self {
        let signature = trader.sign(&payload.canonical_bytes());
        Self {
            payload,
            trader_public: trader.public(),
            signature,
        }
    }

    /// Verify the signature.
    #[must_use]
    pub fn verify(&self) -> bool {
        self.trader_public
            .verify(&self.payload.canonical_bytes(), &self.signature)
    }

    /// Verify against an expected trader key.
    #[must_use]
    pub fn verify_by(&self, expected: &PublicKey) -> bool {
        &self.trader_public == expected && self.verify()
    }
}

// ---------------------------------------------------------------------------
// AuditEventKind
// ---------------------------------------------------------------------------

/// Regulatory audit event categories aligned with `MiFID-II` `RTS 22`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditEventKind {
    OrderAccepted,
    OrderRejected,
    OrderPartiallyFilled,
    OrderFullyFilled,
    OrderCancelled,
    OrderExpired,
}

impl AuditEventKind {
    /// Byte tag used inside canonical byte layouts.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::OrderAccepted => 1,
            Self::OrderRejected => 2,
            Self::OrderPartiallyFilled => 3,
            Self::OrderFullyFilled => 4,
            Self::OrderCancelled => 5,
            Self::OrderExpired => 6,
        }
    }
}

// ---------------------------------------------------------------------------
// AuditEvent
// ---------------------------------------------------------------------------

/// One entry in the [`OrderAuditLog`].
#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub sequence: u64,
    pub kind: AuditEventKind,
    pub order_digest: Hash,
    pub prev_hash: Hash,
    pub timestamp_unix: u64,
}

impl AuditEvent {
    fn canonical_bytes(&self) -> [u8; 8 + 1 + 32 + 32 + 8] {
        let mut buf = [0u8; 8 + 1 + 32 + 32 + 8];
        buf[..8].copy_from_slice(&self.sequence.to_le_bytes());
        buf[8] = self.kind.tag();
        buf[9..41].copy_from_slice(&self.order_digest.0);
        buf[41..73].copy_from_slice(&self.prev_hash.0);
        buf[73..].copy_from_slice(&self.timestamp_unix.to_le_bytes());
        buf
    }

    /// Head hash of this event = `SHA-256(canonical_bytes)`.
    #[must_use]
    pub fn head(&self) -> Hash {
        hash_data(&self.canonical_bytes())
    }
}

// ---------------------------------------------------------------------------
// OrderAuditLog
// ---------------------------------------------------------------------------

/// Append-only hash-chain of order lifecycle events.
#[derive(Debug, Clone, Default)]
pub struct OrderAuditLog {
    events: Vec<AuditEvent>,
    head: Hash,
}

impl OrderAuditLog {
    /// Empty log.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            events: Vec::new(),
            head: Hash([0u8; 32]),
        }
    }

    /// Append an event describing an outcome for `order`.
    pub fn append(&mut self, order: &SignedOrder, kind: AuditEventKind, timestamp_unix: u64) {
        let event = AuditEvent {
            sequence: self.events.len() as u64,
            kind,
            order_digest: order.payload.digest(),
            prev_hash: self.head,
            timestamp_unix,
        };
        self.head = event.head();
        self.events.push(event);
    }

    /// The current head hash.
    #[must_use]
    pub const fn head(&self) -> Hash {
        self.head
    }

    /// All events, in insertion order.
    #[must_use]
    pub fn events(&self) -> &[AuditEvent] {
        &self.events
    }

    /// Verify chain integrity by recomputing every event's head hash.
    #[must_use]
    pub fn verify(&self) -> bool {
        let mut prev = Hash::zero();
        for e in &self.events {
            if e.prev_hash != prev {
                return false;
            }
            prev = e.head();
        }
        prev == self.head
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn push_len(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    buf.extend_from_slice(bytes);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(id: u64) -> OrderPayload {
        OrderPayload {
            order_id: id,
            trader_id: "trader-1".into(),
            instrument: "XJPY".into(),
            side: OrderSide::Buy,
            quantity: 100,
            limit_price_micros: 100_000_000,
            timestamp_unix: 1_720_000_000,
            venue: "OSE".into(),
        }
    }

    #[test]
    fn signed_order_verifies() {
        let kp = KeyPair::from_seed([1u8; 32]);
        let so = SignedOrder::sign(payload(1), &kp);
        assert!(so.verify());
    }

    #[test]
    fn tampering_price_breaks_signature() {
        let kp = KeyPair::from_seed([1u8; 32]);
        let mut so = SignedOrder::sign(payload(1), &kp);
        so.payload.limit_price_micros += 1;
        assert!(!so.verify());
    }

    #[test]
    fn verify_by_expected_trader_rejects_other() {
        let a = KeyPair::from_seed([1u8; 32]);
        let b = KeyPair::from_seed([2u8; 32]);
        let so = SignedOrder::sign(payload(1), &a);
        assert!(so.verify_by(&a.public()));
        assert!(!so.verify_by(&b.public()));
    }

    #[test]
    fn empty_log_head_is_zero() {
        let log = OrderAuditLog::new();
        assert_eq!(log.head(), Hash::zero());
        assert!(log.verify());
    }

    #[test]
    fn log_head_advances_across_events() {
        let kp = KeyPair::from_seed([1u8; 32]);
        let so = SignedOrder::sign(payload(1), &kp);
        let mut log = OrderAuditLog::new();
        log.append(&so, AuditEventKind::OrderAccepted, 1_720_000_010);
        let h1 = log.head();
        log.append(&so, AuditEventKind::OrderFullyFilled, 1_720_000_020);
        assert_ne!(h1, log.head());
        assert!(log.verify());
    }

    #[test]
    fn tampering_middle_event_breaks_verification() {
        let kp = KeyPair::from_seed([1u8; 32]);
        let so = SignedOrder::sign(payload(1), &kp);
        let mut log = OrderAuditLog::new();
        log.append(&so, AuditEventKind::OrderAccepted, 1_720_000_010);
        log.append(&so, AuditEventKind::OrderFullyFilled, 1_720_000_020);
        log.events[0].timestamp_unix = 0;
        assert!(!log.verify());
    }

    #[test]
    fn side_tag_is_distinct() {
        assert_ne!(OrderSide::Buy.tag(), OrderSide::Sell.tag());
    }

    #[test]
    fn kind_tag_is_distinct_across_variants() {
        let mut seen: std::collections::HashSet<u8> = std::collections::HashSet::new();
        for k in [
            AuditEventKind::OrderAccepted,
            AuditEventKind::OrderRejected,
            AuditEventKind::OrderPartiallyFilled,
            AuditEventKind::OrderFullyFilled,
            AuditEventKind::OrderCancelled,
            AuditEventKind::OrderExpired,
        ] {
            assert!(seen.insert(k.tag()));
        }
    }
}
