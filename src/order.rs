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
    #[must_use]
    pub const fn remaining(&self) -> u64 {
        self.quantity - self.filled_quantity
    }

    /// Returns `true` when the entire requested quantity has been matched.
    #[inline(always)]
    #[must_use]
    pub const fn is_filled(&self) -> bool {
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

    // --- OrderId ---

    #[test]
    fn order_id_equality() {
        assert_eq!(OrderId(42), OrderId(42));
        assert_ne!(OrderId(1), OrderId(2));
    }

    #[test]
    fn order_id_ordering() {
        assert!(OrderId(1) < OrderId(2));
        assert!(OrderId(100) > OrderId(99));
        assert!(OrderId(0) < OrderId(u64::MAX));
    }

    #[test]
    fn order_id_zero_is_valid() {
        let o = Order {
            id: OrderId(0),
            side: Side::Bid,
            order_type: OrderType::Limit,
            price: 500,
            quantity: 1,
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::GTC,
        };
        assert_eq!(o.id, OrderId(0));
        assert_eq!(o.remaining(), 1);
    }

    #[test]
    fn order_id_max_is_valid() {
        let id = OrderId(u64::MAX);
        assert_eq!(id, OrderId(u64::MAX));
    }

    // --- Side ---

    #[test]
    fn side_bid_is_not_ask() {
        assert_ne!(Side::Bid, Side::Ask);
    }

    #[test]
    fn side_clone_copy() {
        let s = Side::Ask;
        let s2 = s;
        assert_eq!(s, s2);
    }

    // --- OrderType ---

    #[test]
    fn order_type_limit_equality() {
        assert_eq!(OrderType::Limit, OrderType::Limit);
    }

    #[test]
    fn order_type_market_equality() {
        assert_eq!(OrderType::Market, OrderType::Market);
    }

    #[test]
    fn order_type_stop_limit_stores_price() {
        let ot = OrderType::StopLimit { stop_price: 9999 };
        if let OrderType::StopLimit { stop_price } = ot {
            assert_eq!(stop_price, 9999);
        } else {
            panic!("expected StopLimit");
        }
    }

    #[test]
    fn order_type_stop_limit_negative_price() {
        // 負の stop_price も有効（ショート向けシナリオ）
        let ot = OrderType::StopLimit { stop_price: -1 };
        if let OrderType::StopLimit { stop_price } = ot {
            assert_eq!(stop_price, -1);
        } else {
            panic!("expected StopLimit");
        }
    }

    // --- TimeInForce ---

    #[test]
    fn tif_gtc_equality() {
        assert_eq!(TimeInForce::GTC, TimeInForce::GTC);
    }

    #[test]
    fn tif_ioc_equality() {
        assert_eq!(TimeInForce::IOC, TimeInForce::IOC);
    }

    #[test]
    fn tif_fok_equality() {
        assert_eq!(TimeInForce::FOK, TimeInForce::FOK);
    }

    #[test]
    fn tif_gtd_stores_expiry() {
        let tif = TimeInForce::GTD {
            expiry_ns: 1_000_000,
        };
        if let TimeInForce::GTD { expiry_ns } = tif {
            assert_eq!(expiry_ns, 1_000_000);
        } else {
            panic!("expected GTD");
        }
    }

    #[test]
    fn tif_gtd_expiry_zero_edge() {
        // expiry_ns = 0 は Unix epoch 開始時点 = 即時期限切れ扱い
        let tif = TimeInForce::GTD { expiry_ns: 0 };
        if let TimeInForce::GTD { expiry_ns } = tif {
            assert_eq!(expiry_ns, 0);
        } else {
            panic!("expected GTD");
        }
    }

    #[test]
    fn tif_gtd_expiry_max() {
        let tif = TimeInForce::GTD {
            expiry_ns: u64::MAX,
        };
        if let TimeInForce::GTD { expiry_ns } = tif {
            assert_eq!(expiry_ns, u64::MAX);
        } else {
            panic!("expected GTD");
        }
    }

    // --- remaining / is_filled エッジケース ---

    #[test]
    fn remaining_quantity_one() {
        let o = make_order(1, 0);
        assert_eq!(o.remaining(), 1);
        assert!(!o.is_filled());
    }

    #[test]
    fn remaining_quantity_one_filled() {
        let o = make_order(1, 1);
        assert_eq!(o.remaining(), 0);
        assert!(o.is_filled());
    }

    #[test]
    fn is_filled_when_filled_exceeds_quantity() {
        // filled_quantity > quantity の場合も is_filled() = true であること
        let o = make_order(10, 11);
        assert!(o.is_filled());
    }

    #[test]
    fn order_clone_is_independent() {
        let original = make_order(100, 0);
        let mut cloned = original.clone();
        cloned.filled_quantity = 50;
        assert_eq!(original.filled_quantity, 0);
        assert_eq!(cloned.filled_quantity, 50);
    }
}
