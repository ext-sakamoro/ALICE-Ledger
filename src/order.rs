/*
    ALICE-Ledger
    Copyright (C) 2026 Moroya Sakamoto
*/

//! Order types for the limit order book.
//!
//! Prices are stored as i64 ticks. One tick equals one unit of the smallest
//! representable price increment, matching the Fix128 deterministic arithmetic
//! used by ALICE-Sync. This guarantees bit-exact results across all platforms.

/// Unique order identifier. Monotonically increasing per matching session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct OrderId(pub u64);

/// Side of the market.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Buyer — willing to pay up to `price` ticks.
    Bid,
    /// Seller — willing to accept at least `price` ticks.
    Ask,
}

/// Classification of an order's execution behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    /// Rests on the book at a specific price.
    Limit,
    /// Executes immediately at the best available price; no resting.
    Market,
    /// Becomes a limit order once `stop_price` is touched.
    StopLimit {
        /// Activation threshold in ticks.
        stop_price: i64,
    },
}

/// Time-in-force policy governing how long an order remains active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInForce {
    /// Good-till-cancelled: rests until explicitly cancelled.
    GTC,
    /// Immediate-or-cancel: fill what is available, cancel the remainder.
    IOC,
    /// Fill-or-kill: must fill entirely or be cancelled with zero execution.
    FOK,
    /// Good-till-date: expires at the given nanosecond epoch timestamp.
    GTD {
        /// Expiry time as nanoseconds since the Unix epoch.
        expiry_ns: u64,
    },
}

/// A single order submitted to the matching engine.
#[derive(Debug, Clone)]
pub struct Order {
    /// Unique identifier assigned at submission time.
    pub id: OrderId,
    /// Whether this order is a buy (Bid) or sell (Ask).
    pub side: Side,
    /// Execution classification of this order.
    pub order_type: OrderType,
    /// Limit price in ticks. Ignored for `OrderType::Market`.
    pub price: i64,
    /// Total quantity requested, in base-asset lots.
    pub quantity: u64,
    /// Quantity already matched and executed.
    pub filled_quantity: u64,
    /// Submission time as nanoseconds since the Unix epoch.
    pub timestamp_ns: u64,
    /// Lifetime policy applied to any unexecuted remainder.
    pub time_in_force: TimeInForce,
}

impl Order {
    /// Quantity that has not yet been matched.
    #[inline(always)]
    pub fn remaining(&self) -> u64 {
        self.quantity - self.filled_quantity
    }

    /// Returns `true` when the entire requested quantity has been matched.
    #[inline(always)]
    pub fn is_filled(&self) -> bool {
        self.filled_quantity >= self.quantity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_order(qty: u64, filled: u64) -> Order {
        Order {
            id: OrderId(1),
            side: Side::Bid,
            order_type: OrderType::Limit,
            price: 1000,
            quantity: qty,
            filled_quantity: filled,
            timestamp_ns: 0,
            time_in_force: TimeInForce::GTC,
        }
    }

    #[test]
    fn test_remaining() {
        let o = make_order(100, 40);
        assert_eq!(o.remaining(), 60);
    }

    #[test]
    fn test_is_filled_partial() {
        let o = make_order(100, 50);
        assert!(!o.is_filled());
    }

    #[test]
    fn test_is_filled_complete() {
        let o = make_order(100, 100);
        assert!(o.is_filled());
    }

    #[test]
    fn test_remaining_zero_when_filled() {
        let o = make_order(50, 50);
        assert_eq!(o.remaining(), 0);
    }
}
