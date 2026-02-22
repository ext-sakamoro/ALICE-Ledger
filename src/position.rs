/*
    ALICE-Ledger
    Copyright (C) 2026 Moroya Sakamoto
*/

//! Position tracking and mark-to-market P&L calculation.
//!
//! All prices and P&L values are stored in i64 ticks, consistent with the
//! deterministic fixed-point arithmetic used throughout ALICE-Ledger.
//!
//! Average entry price is maintained using an exact integer calculation to
//! avoid floating-point drift across thousands of fills.

use std::collections::HashMap;

use crate::book::Fill;
use crate::order::Side;

// ---------------------------------------------------------------------------
// Position
// ---------------------------------------------------------------------------

/// Open position and P&L state for a single instrument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    /// FNV-derived hash of the instrument symbol, used as the map key.
    pub symbol_hash: u64,
    /// Signed position size: positive = long, negative = short, zero = flat.
    pub net_quantity: i64,
    /// Weighted-average entry price in ticks, scaled by `PRICE_SCALE`.
    pub avg_entry_price: i64,
    /// P&L from positions that have been fully closed, in ticks.
    pub realized_pnl: i64,
    /// P&L from currently open positions at the last mark-to-market price.
    pub unrealized_pnl: i64,
    /// Total number of fills applied to this position (for audit).
    pub trade_count: u64,
}

impl Position {
    /// Create a flat (zero) position for the given symbol hash.
    #[inline(always)]
    pub fn new(symbol_hash: u64) -> Self {
        Self {
            symbol_hash,
            net_quantity: 0,
            avg_entry_price: 0,
            realized_pnl: 0,
            unrealized_pnl: 0,
            trade_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Position tracker
// ---------------------------------------------------------------------------

/// Tracks open positions across multiple instruments.
pub struct PositionTracker {
    /// Map from symbol hash to open position state.
    positions: HashMap<u64, Position>,
}

impl PositionTracker {
    /// Create an empty position tracker.
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
        }
    }

    /// Retrieve a position by symbol hash.
    #[inline(always)]
    pub fn get(&self, symbol_hash: u64) -> Option<&Position> {
        self.positions.get(&symbol_hash)
    }

    /// Apply a fill to the position for `symbol_hash`.
    ///
    /// `side` is the taker's side: `Bid` increases the long position,
    /// `Ask` increases the short position (decreases net quantity).
    pub fn apply_fill(&mut self, symbol_hash: u64, fill: &Fill, side: Side) {
        let pos = self
            .positions
            .entry(symbol_hash)
            .or_insert_with(|| Position::new(symbol_hash));

        pos.trade_count += 1;

        let signed_qty: i64 = match side {
            Side::Bid => fill.quantity as i64,
            Side::Ask => -(fill.quantity as i64),
        };

        let prev_net = pos.net_quantity;
        let new_net = prev_net + signed_qty;

        if prev_net == 0 {
            // Opening a new position from flat.
            pos.avg_entry_price = fill.price;
            pos.net_quantity = new_net;
            return;
        }

        let same_direction = (prev_net > 0 && signed_qty > 0) || (prev_net < 0 && signed_qty < 0);

        if same_direction {
            // Adding to an existing position — update weighted average entry.
            // avg = (prev_net * prev_avg + delta * fill_price) / new_net
            // Use i128 to prevent overflow during the multiplication.
            let numerator = (prev_net as i128)
                .saturating_mul(pos.avg_entry_price as i128)
                .saturating_add((signed_qty as i128).saturating_mul(fill.price as i128));
            let denominator = new_net as i128;
            // Denominator is guaranteed non-zero when same_direction and new_net != 0.
            pos.avg_entry_price = (numerator / denominator) as i64;
            pos.net_quantity = new_net;
        } else {
            // Reducing or flipping the position.
            let close_qty = prev_net.unsigned_abs().min(fill.quantity);
            let close_qty_i64 = close_qty as i64;

            // Realized P&L from the closed portion.
            // For longs: pnl_per_lot = fill_price - avg_entry
            // For shorts: pnl_per_lot = avg_entry - fill_price
            let pnl_per_lot = if prev_net > 0 {
                fill.price - pos.avg_entry_price
            } else {
                pos.avg_entry_price - fill.price
            };
            pos.realized_pnl += pnl_per_lot * close_qty_i64;

            if new_net == 0 {
                // Fully closed — flatten the position.
                pos.avg_entry_price = 0;
                pos.unrealized_pnl = 0;
            } else if (prev_net > 0 && new_net < 0) || (prev_net < 0 && new_net > 0) {
                // Position flipped — the excess quantity opens in the new direction.
                pos.avg_entry_price = fill.price;
            }
            // If partially closed without flipping, avg_entry_price remains unchanged.
            pos.net_quantity = new_net;
        }
    }

    /// Revalue the unrealized P&L for a position at `current_price`.
    ///
    /// Unrealized P&L = net_quantity * (current_price - avg_entry_price).
    /// A positive value means the position is profitable; negative means a loss.
    #[inline(always)]
    pub fn mark_to_market(&mut self, symbol_hash: u64, current_price: i64) {
        if let Some(pos) = self.positions.get_mut(&symbol_hash) {
            if pos.net_quantity == 0 {
                pos.unrealized_pnl = 0;
                return;
            }
            // Use i128 to avoid overflow on large positions.
            let pnl = (pos.net_quantity as i128)
                .saturating_mul((current_price - pos.avg_entry_price) as i128);
            pos.unrealized_pnl = pnl as i64;
        }
    }
}

impl Default for PositionTracker {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::Fill;
    use crate::order::OrderId;

    fn fill(price: i64, qty: u64) -> Fill {
        Fill {
            maker_id: OrderId(0),
            taker_id: OrderId(1),
            price,
            quantity: qty,
            timestamp_ns: 0,
        }
    }

    const SYM: u64 = 0xDEAD_BEEF;

    // --- opening and adding to a long ---

    #[test]
    fn open_long_position() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Bid);

        let pos = tracker.get(SYM).unwrap();
        assert_eq!(pos.net_quantity, 10);
        assert_eq!(pos.avg_entry_price, 1000);
        assert_eq!(pos.realized_pnl, 0);
        assert_eq!(pos.trade_count, 1);
    }

    #[test]
    fn add_to_long_updates_average_entry() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Bid);
        tracker.apply_fill(SYM, &fill(1010, 10), Side::Bid);

        let pos = tracker.get(SYM).unwrap();
        assert_eq!(pos.net_quantity, 20);
        // Weighted average: (10*1000 + 10*1010) / 20 = 1005
        assert_eq!(pos.avg_entry_price, 1005);
        assert_eq!(pos.trade_count, 2);
    }

    // --- partial close of a long ---

    #[test]
    fn partial_close_long_realizes_pnl() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Bid); // long 10 @ 1000
        tracker.apply_fill(SYM, &fill(1010, 5), Side::Ask); // sell 5 @ 1010

        let pos = tracker.get(SYM).unwrap();
        assert_eq!(pos.net_quantity, 5); // 5 lots remain long
        assert_eq!(pos.avg_entry_price, 1000); // unchanged for remaining lots
        assert_eq!(pos.realized_pnl, 50); // 5 * (1010 - 1000)
    }

    // --- full close of a long ---

    #[test]
    fn full_close_long_flattens_position() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Bid);
        tracker.apply_fill(SYM, &fill(1020, 10), Side::Ask);

        let pos = tracker.get(SYM).unwrap();
        assert_eq!(pos.net_quantity, 0);
        assert_eq!(pos.avg_entry_price, 0);
        assert_eq!(pos.realized_pnl, 200); // 10 * (1020 - 1000)
        assert_eq!(pos.unrealized_pnl, 0);
    }

    // --- short position ---

    #[test]
    fn open_short_position() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Ask);

        let pos = tracker.get(SYM).unwrap();
        assert_eq!(pos.net_quantity, -10);
        assert_eq!(pos.avg_entry_price, 1000);
        assert_eq!(pos.realized_pnl, 0);
    }

    #[test]
    fn close_short_with_profit() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Ask); // short 10 @ 1000
        tracker.apply_fill(SYM, &fill(990, 10), Side::Bid); // cover @ 990

        let pos = tracker.get(SYM).unwrap();
        assert_eq!(pos.net_quantity, 0);
        // Profit: 10 * (1000 - 990) = 100
        assert_eq!(pos.realized_pnl, 100);
    }

    // --- position flip ---

    #[test]
    fn flip_long_to_short() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Bid); // long 10
        tracker.apply_fill(SYM, &fill(1005, 15), Side::Ask); // sell 15

        let pos = tracker.get(SYM).unwrap();
        // After closing 10 longs (realized 5*10=50) and opening 5 shorts.
        assert_eq!(pos.net_quantity, -5);
        assert_eq!(pos.avg_entry_price, 1005); // new short entered at 1005
        assert_eq!(pos.realized_pnl, 50);
    }

    // --- mark to market ---

    #[test]
    fn mark_to_market_long_profit() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Bid);
        tracker.mark_to_market(SYM, 1010);

        let pos = tracker.get(SYM).unwrap();
        assert_eq!(pos.unrealized_pnl, 100); // 10 * (1010 - 1000)
    }

    #[test]
    fn mark_to_market_long_loss() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Bid);
        tracker.mark_to_market(SYM, 990);

        let pos = tracker.get(SYM).unwrap();
        assert_eq!(pos.unrealized_pnl, -100); // 10 * (990 - 1000)
    }

    #[test]
    fn mark_to_market_short_profit() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Ask);
        tracker.mark_to_market(SYM, 980);

        let pos = tracker.get(SYM).unwrap();
        // -10 * (980 - 1000) = -10 * -20 = 200
        assert_eq!(pos.unrealized_pnl, 200);
    }

    #[test]
    fn mark_to_market_flat_is_zero() {
        let mut tracker = PositionTracker::new();
        tracker.apply_fill(SYM, &fill(1000, 10), Side::Bid);
        tracker.apply_fill(SYM, &fill(1010, 10), Side::Ask);
        tracker.mark_to_market(SYM, 1020);

        let pos = tracker.get(SYM).unwrap();
        assert_eq!(pos.unrealized_pnl, 0);
    }

    // --- missing symbol ---

    #[test]
    fn get_unknown_symbol_returns_none() {
        let tracker = PositionTracker::new();
        assert!(tracker.get(0xCAFE_BABE).is_none());
    }
}
