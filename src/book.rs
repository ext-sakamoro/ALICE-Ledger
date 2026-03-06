/*
    ALICE-Ledger
    Copyright (C) 2026 Moroya Sakamoto
*/

//! Limit order book (LOB) with price-time priority matching.
//!
//! Bids are stored in a `BTreeMap<Reverse<i64>, PriceLevel>` so that the
//! highest bid is always at the front (smallest key = largest price).
//! Asks are stored in a `BTreeMap<i64, PriceLevel>` so that the lowest ask
//! is always at the front.
//!
//! Within a price level, orders are stored in a `VecDeque` in arrival order,
//! giving strict FIFO (first-in, first-out) time priority.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::order::{Order, OrderId, OrderType, Side, TimeInForce};

// ---------------------------------------------------------------------------
// Fill record
// ---------------------------------------------------------------------------

/// Record of a matched trade between a passive maker and an aggressive taker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fill {
    /// ID of the resting (passive) order that was matched against.
    pub maker_id: OrderId,
    /// ID of the incoming (aggressive) order that triggered the match.
    pub taker_id: OrderId,
    /// Execution price in ticks (always the maker's limit price).
    pub price: i64,
    /// Quantity matched in this fill event.
    pub quantity: u64,
    /// Timestamp of the fill as nanoseconds since the Unix epoch.
    pub timestamp_ns: u64,
}

// ---------------------------------------------------------------------------
// Price level
// ---------------------------------------------------------------------------

/// All resting orders at a single price point, maintained in FIFO order.
#[derive(Debug)]
pub struct PriceLevel {
    /// Price of every order in this level, in ticks.
    pub price: i64,
    /// Orders sorted by arrival time (front = oldest = highest priority).
    pub orders: VecDeque<Order>,
    /// Aggregate unexecuted quantity across all orders in this level.
    pub total_quantity: u64,
}

impl PriceLevel {
    /// Create an empty price level at the given tick price.
    #[inline(always)]
    #[must_use]
    pub const fn new(price: i64) -> Self {
        Self {
            price,
            orders: VecDeque::new(),
            total_quantity: 0,
        }
    }

    /// Append an order to the back of the FIFO queue.
    #[inline(always)]
    pub fn push(&mut self, order: Order) {
        self.total_quantity += order.remaining();
        self.orders.push_back(order);
    }

    /// Returns `true` when no resting orders remain at this level.
    #[inline(always)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Order book
// ---------------------------------------------------------------------------

/// Dual-sided limit order book with price-time priority matching.
pub struct OrderBook {
    /// Bids sorted by descending price (highest bid first).
    bids: BTreeMap<Reverse<i64>, PriceLevel>,
    /// Asks sorted by ascending price (lowest ask first).
    asks: BTreeMap<i64, PriceLevel>,
    /// Fast lookup from order ID to (side, price) for O(log n) cancellation.
    order_index: HashMap<OrderId, (Side, i64)>,
}

impl OrderBook {
    /// Create a new, empty order book.
    #[inline(always)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            order_index: HashMap::new(),
        }
    }

    /// Best bid price (highest) in ticks, or `None` if the bid side is empty.
    #[inline(always)]
    #[must_use]
    pub fn best_bid(&self) -> Option<i64> {
        self.bids.keys().next().map(|r| r.0)
    }

    /// Best ask price (lowest) in ticks, or `None` if the ask side is empty.
    #[inline(always)]
    #[must_use]
    pub fn best_ask(&self) -> Option<i64> {
        self.asks.keys().next().copied()
    }

    /// Bid-ask spread in ticks, or `None` when either side is empty.
    #[inline(always)]
    #[must_use]
    pub fn spread(&self) -> Option<i64> {
        match (self.best_ask(), self.best_bid()) {
            (Some(ask), Some(bid)) => Some(ask - bid),
            _ => None,
        }
    }

    /// Return up to `levels` price levels on `side` as `(price, quantity)` pairs.
    ///
    /// Bids are returned in descending price order; asks in ascending price order.
    #[must_use]
    pub fn depth(&self, side: Side, levels: usize) -> Vec<(i64, u64)> {
        match side {
            Side::Bid => self
                .bids
                .iter()
                .take(levels)
                .map(|(r, lvl)| (r.0, lvl.total_quantity))
                .collect(),
            Side::Ask => self
                .asks
                .iter()
                .take(levels)
                .map(|(p, lvl)| (*p, lvl.total_quantity))
                .collect(),
        }
    }

    /// Submit an order for matching and, if a remainder exists, resting on the book.
    ///
    /// Returns the list of fills produced by this submission.
    ///
    /// GTD orders whose `expiry_ns` is less than or equal to `now_ns` are
    /// rejected immediately (zero fills, never rested). Pass `now_ns = 0` to
    /// skip the expiry check (useful in tests that do not care about time).
    pub fn insert(&mut self, order: Order) -> Vec<Fill> {
        self.insert_with_time(order, 0)
    }

    /// Variant of [`Self::insert`] that applies the GTD expiry check against
    /// `now_ns` (nanoseconds since the Unix epoch).
    ///
    /// If the order is GTD and `expiry_ns <= now_ns`, the order is silently
    /// discarded and an empty fill list is returned.
    pub fn insert_with_time(&mut self, mut order: Order, now_ns: u64) -> Vec<Fill> {
        // --- GTD pre-check: reject already-expired orders ---
        if let TimeInForce::GTD { expiry_ns } = order.time_in_force {
            if now_ns > 0 && expiry_ns <= now_ns {
                return Vec::new();
            }
        }

        let mut fills = Vec::new();

        // --- attempt to match against the opposite side ---
        match order.side {
            Side::Bid => self.match_bid(&mut order, &mut fills),
            Side::Ask => self.match_ask(&mut order, &mut fills),
        }

        // --- handle unexecuted remainder according to order type and TIF ---
        if !order.is_filled() {
            match order.order_type {
                OrderType::Market => {
                    // Market orders never rest; any unexecuted quantity is discarded.
                }
                OrderType::Limit | OrderType::StopLimit { .. } => {
                    match order.time_in_force {
                        TimeInForce::IOC | TimeInForce::FOK => {
                            // IOC: discard remainder. FOK: already handled in match phase.
                        }
                        TimeInForce::GTC | TimeInForce::GTD { .. } => {
                            // Rest the remainder on the book.
                            self.rest_order(order);
                        }
                    }
                }
            }
        }

        fills
    }

    /// Cancel an order by its ID.
    ///
    /// Returns the cancelled `Order` if found, or `None` if the ID is unknown.
    pub fn cancel(&mut self, id: OrderId) -> Option<Order> {
        let (side, price) = self.order_index.remove(&id)?;

        let cancelled = match side {
            Side::Bid => {
                let level = self.bids.get_mut(&Reverse(price))?;
                remove_from_level(level, id)
            }
            Side::Ask => {
                let level = self.asks.get_mut(&price)?;
                remove_from_level(level, id)
            }
        };

        // Prune the price level if it is now empty.
        if let Some(ref order) = cancelled {
            match side {
                Side::Bid => {
                    if self
                        .bids
                        .get(&Reverse(price))
                        .is_none_or(PriceLevel::is_empty)
                    {
                        self.bids.remove(&Reverse(price));
                    }
                    let _ = order; // already moved
                }
                Side::Ask => {
                    if self.asks.get(&price).is_none_or(PriceLevel::is_empty) {
                        self.asks.remove(&price);
                    }
                }
            }
        }

        cancelled
    }

    /// Remove all resting GTD orders whose `expiry_ns` is less than or equal
    /// to `now_ns` (nanoseconds since the Unix epoch).
    ///
    /// Returns the list of cancelled orders so callers can send cancellation
    /// acknowledgements or generate audit records.
    ///
    /// Complexity: O(n) in the number of resting orders, which is acceptable
    /// for the periodic housekeeping cadence at which this is typically called.
    pub fn purge_expired(&mut self, now_ns: u64) -> Vec<Order> {
        let mut expired: Vec<Order> = Vec::new();

        // Collect (side, price, order_id) tuples for every expired GTD order.
        let mut to_remove: Vec<(Side, i64, OrderId)> = Vec::new();

        for (Reverse(price), level) in &self.bids {
            for order in &level.orders {
                if let TimeInForce::GTD { expiry_ns } = order.time_in_force {
                    if expiry_ns <= now_ns {
                        to_remove.push((Side::Bid, *price, order.id));
                    }
                }
            }
        }

        for (price, level) in &self.asks {
            for order in &level.orders {
                if let TimeInForce::GTD { expiry_ns } = order.time_in_force {
                    if expiry_ns <= now_ns {
                        to_remove.push((Side::Ask, *price, order.id));
                    }
                }
            }
        }

        // Remove each expired order via the existing cancel path.
        for (_side, _price, id) in to_remove {
            if let Some(order) = self.cancel(id) {
                expired.push(order);
            }
        }

        expired
    }

    /// Collect all resting order IDs for a given side.
    #[must_use]
    pub fn all_order_ids_by_side(&self, side: Side) -> Vec<OrderId> {
        match side {
            Side::Bid => self
                .bids
                .values()
                .flat_map(|level| level.orders.iter().map(|o| o.id))
                .collect(),
            Side::Ask => self
                .asks
                .values()
                .flat_map(|level| level.orders.iter().map(|o| o.id))
                .collect(),
        }
    }

    // -----------------------------------------------------------------------
    // Private matching helpers
    // -----------------------------------------------------------------------

    /// Match an incoming bid against resting asks, from lowest ask price up.
    fn match_bid(&mut self, taker: &mut Order, fills: &mut Vec<Fill>) {
        // For FOK we must verify full fill is possible before executing.
        if taker.time_in_force == TimeInForce::FOK && !self.can_fill_bid(taker) {
            return; // Cancel without any execution.
        }

        let taker_price = match taker.order_type {
            OrderType::Market => i64::MAX,
            OrderType::Limit | OrderType::StopLimit { .. } => taker.price,
        };

        // Collect keys to avoid borrowing issues.
        let keys: Vec<i64> = self.asks.range(..=taker_price).map(|(k, _)| *k).collect();

        for ask_price in keys {
            if taker.is_filled() {
                break;
            }
            let taker_id = taker.id;
            let ts = taker.timestamp_ns;
            Self::fill_level_bid(
                self.asks.get_mut(&ask_price).unwrap(),
                taker,
                taker_id,
                ts,
                fills,
                &mut self.order_index,
            );
            // Remove the level if exhausted.
            if self.asks.get(&ask_price).is_some_and(PriceLevel::is_empty) {
                self.asks.remove(&ask_price);
            }
        }
    }

    /// Match an incoming ask against resting bids, from highest bid price down.
    fn match_ask(&mut self, taker: &mut Order, fills: &mut Vec<Fill>) {
        if taker.time_in_force == TimeInForce::FOK && !self.can_fill_ask(taker) {
            return;
        }

        let taker_price = match taker.order_type {
            OrderType::Market => i64::MIN,
            OrderType::Limit | OrderType::StopLimit { .. } => taker.price,
        };

        // Bids are keyed by Reverse<i64>; the lowest Reverse key = highest price.
        let keys: Vec<i64> = self
            .bids
            .range(..=Reverse(taker_price))
            .map(|(Reverse(k), _)| *k)
            .collect();

        for bid_price in keys {
            if taker.is_filled() {
                break;
            }
            let taker_id = taker.id;
            let ts = taker.timestamp_ns;
            Self::fill_level_ask(
                self.bids.get_mut(&Reverse(bid_price)).unwrap(),
                taker,
                taker_id,
                ts,
                fills,
                &mut self.order_index,
            );
            if self
                .bids
                .get(&Reverse(bid_price))
                .is_some_and(PriceLevel::is_empty)
            {
                self.bids.remove(&Reverse(bid_price));
            }
        }
    }

    /// Execute fills against a single ask price level on behalf of a bid taker.
    fn fill_level_bid(
        level: &mut PriceLevel,
        taker: &mut Order,
        taker_id: OrderId,
        timestamp_ns: u64,
        fills: &mut Vec<Fill>,
        index: &mut HashMap<OrderId, (Side, i64)>,
    ) {
        while let Some(maker) = level.orders.front_mut() {
            if taker.is_filled() {
                break;
            }
            let trade_qty = taker.remaining().min(maker.remaining());
            if trade_qty == 0 {
                break;
            }

            let fill_price = maker.price;
            taker.filled_quantity += trade_qty;
            maker.filled_quantity += trade_qty;
            level.total_quantity = level.total_quantity.saturating_sub(trade_qty);

            fills.push(Fill {
                maker_id: maker.id,
                taker_id,
                price: fill_price,
                quantity: trade_qty,
                timestamp_ns,
            });

            if maker.is_filled() {
                let filled_id = maker.id;
                level.orders.pop_front();
                index.remove(&filled_id);
            }
        }
    }

    /// Execute fills against a single bid price level on behalf of an ask taker.
    fn fill_level_ask(
        level: &mut PriceLevel,
        taker: &mut Order,
        taker_id: OrderId,
        timestamp_ns: u64,
        fills: &mut Vec<Fill>,
        index: &mut HashMap<OrderId, (Side, i64)>,
    ) {
        while let Some(maker) = level.orders.front_mut() {
            if taker.is_filled() {
                break;
            }
            let trade_qty = taker.remaining().min(maker.remaining());
            if trade_qty == 0 {
                break;
            }

            let fill_price = maker.price;
            taker.filled_quantity += trade_qty;
            maker.filled_quantity += trade_qty;
            level.total_quantity = level.total_quantity.saturating_sub(trade_qty);

            fills.push(Fill {
                maker_id: maker.id,
                taker_id,
                price: fill_price,
                quantity: trade_qty,
                timestamp_ns,
            });

            if maker.is_filled() {
                let filled_id = maker.id;
                level.orders.pop_front();
                index.remove(&filled_id);
            }
        }
    }

    /// Check whether the available ask-side liquidity can fully satisfy a FOK bid.
    fn can_fill_bid(&self, order: &Order) -> bool {
        let taker_price = match order.order_type {
            OrderType::Market => i64::MAX,
            OrderType::Limit | OrderType::StopLimit { .. } => order.price,
        };
        let available: u64 = self
            .asks
            .range(..=taker_price)
            .map(|(_, lvl)| lvl.total_quantity)
            .sum();
        available >= order.remaining()
    }

    /// Check whether the available bid-side liquidity can fully satisfy a FOK ask.
    fn can_fill_ask(&self, order: &Order) -> bool {
        let taker_price = match order.order_type {
            OrderType::Market => i64::MIN,
            OrderType::Limit | OrderType::StopLimit { .. } => order.price,
        };
        let available: u64 = self
            .bids
            .range(..=Reverse(taker_price))
            .map(|(_, lvl)| lvl.total_quantity)
            .sum();
        available >= order.remaining()
    }

    /// Place a fully or partially unexecuted order onto the resting book.
    #[inline(always)]
    fn rest_order(&mut self, order: Order) {
        self.order_index.insert(order.id, (order.side, order.price));
        match order.side {
            Side::Bid => {
                self.bids
                    .entry(Reverse(order.price))
                    .or_insert_with(|| PriceLevel::new(order.price))
                    .push(order);
            }
            Side::Ask => {
                self.asks
                    .entry(order.price)
                    .or_insert_with(|| PriceLevel::new(order.price))
                    .push(order);
            }
        }
    }
}

impl Default for OrderBook {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helper: remove a specific order from a price level by ID
// ---------------------------------------------------------------------------

/// Remove an order by `id` from `level`. Returns the removed order or `None`.
#[inline(always)]
fn remove_from_level(level: &mut PriceLevel, id: OrderId) -> Option<Order> {
    let pos = level.orders.iter().position(|o| o.id == id)?;
    let order = level.orders.remove(pos)?;
    level.total_quantity = level.total_quantity.saturating_sub(order.remaining());
    Some(order)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{Order, OrderId, OrderType, Side, TimeInForce};

    fn limit(id: u64, side: Side, price: i64, qty: u64, ts: u64) -> Order {
        Order {
            id: OrderId(id),
            side,
            order_type: OrderType::Limit,
            price,
            quantity: qty,
            filled_quantity: 0,
            timestamp_ns: ts,
            time_in_force: TimeInForce::GTC,
        }
    }

    fn market(id: u64, side: Side, qty: u64) -> Order {
        Order {
            id: OrderId(id),
            side,
            order_type: OrderType::Market,
            price: 0, // ignored for market orders
            quantity: qty,
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::IOC,
        }
    }

    // --- basic book state ---

    #[test]
    fn empty_book_has_no_best() {
        let book = OrderBook::new();
        assert!(book.best_bid().is_none());
        assert!(book.best_ask().is_none());
        assert!(book.spread().is_none());
    }

    #[test]
    fn insert_resting_bids_and_asks() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 990, 10, 0));
        book.insert(limit(2, Side::Bid, 995, 5, 1));
        book.insert(limit(3, Side::Ask, 1000, 10, 2));
        book.insert(limit(4, Side::Ask, 1005, 5, 3));

        assert_eq!(book.best_bid(), Some(995));
        assert_eq!(book.best_ask(), Some(1000));
        assert_eq!(book.spread(), Some(5));
    }

    // --- market order matching ---

    #[test]
    fn market_buy_matches_lowest_ask() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 10, 0));
        book.insert(limit(2, Side::Ask, 1005, 10, 1));

        let fills = book.insert(market(3, Side::Bid, 10));
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].price, 1000);
        assert_eq!(fills[0].quantity, 10);
        assert_eq!(fills[0].maker_id, OrderId(1));
    }

    #[test]
    fn market_sell_matches_highest_bid() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 1000, 10, 0));
        book.insert(limit(2, Side::Bid, 990, 10, 1));

        let fills = book.insert(market(3, Side::Ask, 10));
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].price, 1000);
        assert_eq!(fills[0].quantity, 10);
    }

    // --- partial fills ---

    #[test]
    fn partial_fill_leaves_remainder_on_book() {
        let mut book = OrderBook::new();
        // Resting ask of 20 lots; taker buys 8 lots.
        book.insert(limit(1, Side::Ask, 1000, 20, 0));
        let fills = book.insert(market(2, Side::Bid, 8));

        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].quantity, 8);
        // Remaining 12 lots should still be on the ask side.
        assert_eq!(book.best_ask(), Some(1000));
        let depth = book.depth(Side::Ask, 1);
        assert_eq!(depth[0].1, 12);
    }

    #[test]
    fn large_market_order_sweeps_multiple_levels() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 5, 0));
        book.insert(limit(2, Side::Ask, 1001, 5, 1));
        book.insert(limit(3, Side::Ask, 1002, 5, 2));

        let fills = book.insert(market(4, Side::Bid, 15));
        assert_eq!(fills.len(), 3);
        assert_eq!(fills[0].price, 1000);
        assert_eq!(fills[1].price, 1001);
        assert_eq!(fills[2].price, 1002);
        assert!(book.best_ask().is_none());
    }

    // --- FIFO at same price level ---

    #[test]
    fn fifo_priority_at_same_price() {
        let mut book = OrderBook::new();
        // Three bids at the same price; different timestamps to distinguish them.
        book.insert(limit(1, Side::Ask, 1000, 5, 100)); // earliest
        book.insert(limit(2, Side::Ask, 1000, 5, 200));
        book.insert(limit(3, Side::Ask, 1000, 5, 300)); // latest

        // Buy 5 lots — should fill against order #1 (oldest).
        let fills = book.insert(market(4, Side::Bid, 5));
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].maker_id, OrderId(1));

        // Buy another 5 — should fill against order #2.
        let fills = book.insert(market(5, Side::Bid, 5));
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].maker_id, OrderId(2));
    }

    // --- cancellation ---

    #[test]
    fn cancel_resting_order() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 1000, 10, 0));
        let cancelled = book.cancel(OrderId(1));
        assert!(cancelled.is_some());
        assert!(book.best_bid().is_none());
    }

    #[test]
    fn cancel_unknown_order_returns_none() {
        let mut book = OrderBook::new();
        assert!(book.cancel(OrderId(999)).is_none());
    }

    #[test]
    fn cancel_one_of_multiple_at_same_level() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 10, 0));
        book.insert(limit(2, Side::Ask, 1000, 5, 1));
        book.cancel(OrderId(1));

        let depth = book.depth(Side::Ask, 1);
        assert_eq!(depth.len(), 1);
        assert_eq!(depth[0].1, 5); // only order #2 remains
    }

    // --- depth ---

    #[test]
    fn depth_returns_correct_levels() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 995, 10, 0));
        book.insert(limit(2, Side::Bid, 990, 20, 1));

        let depth = book.depth(Side::Bid, 2);
        assert_eq!(depth.len(), 2);
        assert_eq!(depth[0], (995, 10)); // best bid first
        assert_eq!(depth[1], (990, 20));
    }

    // --- FOK ---

    #[test]
    fn fok_cancels_when_insufficient_liquidity() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 5, 0)); // only 5 available

        let fok = Order {
            id: OrderId(2),
            side: Side::Bid,
            order_type: OrderType::Limit,
            price: 1000,
            quantity: 10, // wants 10
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::FOK,
        };
        let fills = book.insert(fok);
        assert!(fills.is_empty()); // no partial fill allowed
    }

    #[test]
    fn fok_fills_completely_when_sufficient_liquidity() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 15, 0));

        let fok = Order {
            id: OrderId(2),
            side: Side::Bid,
            order_type: OrderType::Limit,
            price: 1000,
            quantity: 10,
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::FOK,
        };
        let fills = book.insert(fok);
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].quantity, 10);
    }

    // --- IOC ---

    #[test]
    fn ioc_fills_partial_and_discards_remainder() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 5, 0)); // only 5 lots

        let ioc = Order {
            id: OrderId(2),
            side: Side::Bid,
            order_type: OrderType::Limit,
            price: 1000,
            quantity: 10, // wants 10
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::IOC,
        };
        let fills = book.insert(ioc);
        // 5 lots filled, remainder discarded (not resting on book).
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].quantity, 5);
        // The bid side should remain empty.
        assert!(book.best_bid().is_none());
    }

    // --- GTD ---

    fn gtd(id: u64, side: Side, price: i64, qty: u64, ts: u64, expiry_ns: u64) -> Order {
        Order {
            id: OrderId(id),
            side,
            order_type: OrderType::Limit,
            price,
            quantity: qty,
            filled_quantity: 0,
            timestamp_ns: ts,
            time_in_force: TimeInForce::GTD { expiry_ns },
        }
    }

    #[test]
    fn gtd_rests_on_book_when_not_yet_expired() {
        let mut book = OrderBook::new();
        // expiry = 1000, now = 500 → still valid
        let fills = book.insert_with_time(gtd(1, Side::Bid, 990, 10, 0, 1_000), 500);
        assert!(fills.is_empty());
        assert_eq!(book.best_bid(), Some(990));
    }

    #[test]
    fn gtd_rejected_immediately_when_already_expired() {
        let mut book = OrderBook::new();
        // expiry = 500, now = 1000 → already expired
        let fills = book.insert_with_time(gtd(1, Side::Bid, 990, 10, 0, 500), 1_000);
        assert!(fills.is_empty());
        assert!(book.best_bid().is_none()); // must not have rested
    }

    #[test]
    fn gtd_rejected_at_exact_expiry_boundary() {
        let mut book = OrderBook::new();
        // expiry == now → expired
        let fills = book.insert_with_time(gtd(1, Side::Ask, 1000, 5, 0, 1_000), 1_000);
        assert!(fills.is_empty());
        assert!(book.best_ask().is_none());
    }

    #[test]
    fn purge_expired_removes_gtd_orders_past_expiry() {
        let mut book = OrderBook::new();
        // Two GTD orders on the bid side with different expiries.
        book.insert_with_time(gtd(1, Side::Bid, 990, 10, 0, 500), 0); // expires at 500 ns
        book.insert_with_time(gtd(2, Side::Bid, 995, 5, 1, 2_000), 0); // expires at 2000 ns
                                                                       // One GTC order that must survive.
        book.insert(limit(3, Side::Bid, 992, 3, 2));

        // At now = 1000 ns only order #1 (expiry=500) has expired.
        let expired = book.purge_expired(1_000);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, OrderId(1));

        // Best bid is now 995 (order #2), and the GTC at 992 is still present.
        assert_eq!(book.best_bid(), Some(995));
        let depth = book.depth(Side::Bid, 3);
        assert_eq!(depth.len(), 2); // 995 and 992
    }

    #[test]
    fn purge_expired_removes_gtd_on_ask_side() {
        let mut book = OrderBook::new();
        book.insert_with_time(gtd(1, Side::Ask, 1000, 10, 0, 100), 0); // expires at 100 ns
        book.insert_with_time(gtd(2, Side::Ask, 1001, 5, 1, 5_000), 0); // expires at 5000 ns

        let expired = book.purge_expired(200);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, OrderId(1));
        assert_eq!(book.best_ask(), Some(1001));
    }

    #[test]
    fn purge_expired_leaves_gtc_orders_untouched() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 10, 0)); // GTC — must survive any purge
        let expired = book.purge_expired(u64::MAX);
        assert!(expired.is_empty());
        assert_eq!(book.best_ask(), Some(1000));
    }

    #[test]
    fn purge_expired_returns_empty_when_nothing_expired() {
        let mut book = OrderBook::new();
        // expiry = 2000, now = 1000 → still valid
        book.insert_with_time(gtd(1, Side::Bid, 990, 10, 0, 2_000), 0);
        let expired = book.purge_expired(1_000);
        assert!(expired.is_empty());
        assert_eq!(book.best_bid(), Some(990));
    }

    #[test]
    fn gtd_order_fills_normally_before_expiry() {
        let mut book = OrderBook::new();
        // Resting GTD ask at 1000, expires far in the future.
        book.insert_with_time(gtd(1, Side::Ask, 1000, 10, 0, 9_999_999), 0);
        // Market bid hits it before expiry.
        let fills = book.insert_with_time(market(2, Side::Bid, 10), 500);
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].price, 1000);
        assert_eq!(fills[0].quantity, 10);
    }

    // --- spread エッジケース ---

    #[test]
    fn spread_only_bids_returns_none() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 1000, 5, 0));
        assert!(book.spread().is_none());
    }

    #[test]
    fn spread_only_asks_returns_none() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 5, 0));
        assert!(book.spread().is_none());
    }

    #[test]
    fn spread_zero_when_bid_equals_ask() {
        // bid = ask → crossed book (spread = 0)。マーケットオーダーで消化されるため
        // 残留する場合はスプレッドが 0 になる
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 1000, 5, 0));
        // 同価格の ask を置いても bid と即時マッチするので book は片側になる
        book.insert(limit(2, Side::Ask, 1005, 5, 1));
        assert_eq!(book.spread(), Some(5));
    }

    // --- depth エッジケース ---

    #[test]
    fn depth_zero_levels_requested_returns_empty() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 1000, 5, 0));
        assert!(book.depth(Side::Bid, 0).is_empty());
    }

    #[test]
    fn depth_empty_book_returns_empty() {
        let book = OrderBook::new();
        assert!(book.depth(Side::Bid, 5).is_empty());
        assert!(book.depth(Side::Ask, 5).is_empty());
    }

    #[test]
    fn depth_limited_to_requested_levels() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 5, 0));
        book.insert(limit(2, Side::Ask, 1001, 5, 1));
        book.insert(limit(3, Side::Ask, 1002, 5, 2));
        // 3レベルあるが 2 だけ要求する
        let d = book.depth(Side::Ask, 2);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].0, 1000);
        assert_eq!(d[1].0, 1001);
    }

    #[test]
    fn depth_ask_ascending_order() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1002, 5, 0));
        book.insert(limit(2, Side::Ask, 1000, 5, 1));
        book.insert(limit(3, Side::Ask, 1001, 5, 2));
        let d = book.depth(Side::Ask, 3);
        assert_eq!(d[0].0, 1000);
        assert_eq!(d[1].0, 1001);
        assert_eq!(d[2].0, 1002);
    }

    #[test]
    fn depth_bid_descending_order() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 998, 5, 0));
        book.insert(limit(2, Side::Bid, 1000, 5, 1));
        book.insert(limit(3, Side::Bid, 999, 5, 2));
        let d = book.depth(Side::Bid, 3);
        assert_eq!(d[0].0, 1000);
        assert_eq!(d[1].0, 999);
        assert_eq!(d[2].0, 998);
    }

    // --- cancel エッジケース ---

    #[test]
    fn cancel_ask_order() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 10, 0));
        let cancelled = book.cancel(OrderId(1));
        assert!(cancelled.is_some());
        assert!(book.best_ask().is_none());
    }

    #[test]
    fn cancel_removes_level_when_empty() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 1000, 10, 0));
        book.cancel(OrderId(1));
        // レベルが空になったので best_bid は None
        assert!(book.best_bid().is_none());
        assert!(book.spread().is_none());
    }

    #[test]
    fn cancel_after_partial_fill() {
        let mut book = OrderBook::new();
        // ask 20 lots → market buy 8 lots → cancel 残り 12
        book.insert(limit(1, Side::Ask, 1000, 20, 0));
        book.insert(market(2, Side::Bid, 8));
        let cancelled = book.cancel(OrderId(1));
        assert!(cancelled.is_some());
        // remaining = 20 - 8 = 12
        assert_eq!(cancelled.unwrap().remaining(), 12);
        assert!(book.best_ask().is_none());
    }

    #[test]
    fn cancel_then_reinsert_same_id() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 1000, 10, 0));
        book.cancel(OrderId(1));
        // 同じ ID で再挿入
        book.insert(limit(1, Side::Bid, 1000, 5, 1));
        assert_eq!(book.best_bid(), Some(1000));
    }

    // --- market order に流動性がない場合 ---

    #[test]
    fn market_buy_on_empty_book_returns_no_fills() {
        let mut book = OrderBook::new();
        let fills = book.insert(market(1, Side::Bid, 10));
        assert!(fills.is_empty());
    }

    #[test]
    fn market_sell_on_empty_book_returns_no_fills() {
        let mut book = OrderBook::new();
        let fills = book.insert(market(1, Side::Ask, 10));
        assert!(fills.is_empty());
    }

    // --- limit 注文が相手方を超えない価格の場合は約定しない ---

    #[test]
    fn limit_bid_below_ask_rests_on_book() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1005, 10, 0));
        // bid 価格が ask より低い → 約定せず rest
        let fills = book.insert(limit(2, Side::Bid, 1000, 10, 1));
        assert!(fills.is_empty());
        assert_eq!(book.best_bid(), Some(1000));
        assert_eq!(book.best_ask(), Some(1005));
    }

    #[test]
    fn limit_ask_above_bid_rests_on_book() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 995, 10, 0));
        // ask 価格が bid より高い → 約定せず rest
        let fills = book.insert(limit(2, Side::Ask, 1000, 10, 1));
        assert!(fills.is_empty());
        assert_eq!(book.best_ask(), Some(1000));
    }

    // --- IOC が完全に約定する場合 ---

    #[test]
    fn ioc_fills_completely_when_sufficient_liquidity() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Ask, 1000, 20, 0)); // 20 lots available

        let ioc = Order {
            id: OrderId(2),
            side: Side::Bid,
            order_type: OrderType::Limit,
            price: 1000,
            quantity: 10, // 10 lots required
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::IOC,
        };
        let fills = book.insert(ioc);
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].quantity, 10);
        // 残り 10 lots は book に残っていること
        let d = book.depth(Side::Ask, 1);
        assert_eq!(d[0].1, 10);
    }

    // --- FOK ask 側 ---

    #[test]
    fn fok_ask_cancels_when_insufficient_bid_liquidity() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 1000, 3, 0)); // 3 lots

        let fok = Order {
            id: OrderId(2),
            side: Side::Ask,
            order_type: OrderType::Limit,
            price: 1000,
            quantity: 10, // 10 必要
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::FOK,
        };
        let fills = book.insert(fok);
        assert!(fills.is_empty());
        // 元の bid は無傷で残っていること
        assert_eq!(book.best_bid(), Some(1000));
    }

    #[test]
    fn fok_ask_fills_when_sufficient_bid_liquidity() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 1000, 20, 0)); // 20 lots

        let fok = Order {
            id: OrderId(2),
            side: Side::Ask,
            order_type: OrderType::Limit,
            price: 1000,
            quantity: 10,
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::FOK,
        };
        let fills = book.insert(fok);
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].quantity, 10);
    }

    // --- market order が複数レベルを跨ぐ (ask 側) ---

    #[test]
    fn market_sell_sweeps_multiple_bid_levels() {
        let mut book = OrderBook::new();
        book.insert(limit(1, Side::Bid, 1000, 5, 0));
        book.insert(limit(2, Side::Bid, 999, 5, 1));
        book.insert(limit(3, Side::Bid, 998, 5, 2));

        let fills = book.insert(market(4, Side::Ask, 15));
        assert_eq!(fills.len(), 3);
        assert_eq!(fills[0].price, 1000);
        assert_eq!(fills[1].price, 999);
        assert_eq!(fills[2].price, 998);
        assert!(book.best_bid().is_none());
    }

    // --- purge_expired: 空の book では何も返さない ---

    #[test]
    fn purge_expired_on_empty_book_returns_empty() {
        let mut book = OrderBook::new();
        let expired = book.purge_expired(u64::MAX);
        assert!(expired.is_empty());
    }

    // --- purge_expired: 全 GTD が期限切れ ---

    #[test]
    fn purge_expired_removes_all_gtd_when_all_expired() {
        let mut book = OrderBook::new();
        book.insert_with_time(gtd(1, Side::Bid, 990, 5, 0, 100), 0);
        book.insert_with_time(gtd(2, Side::Ask, 1010, 5, 1, 200), 0);

        let expired = book.purge_expired(1000);
        assert_eq!(expired.len(), 2);
        assert!(book.best_bid().is_none());
        assert!(book.best_ask().is_none());
    }

    // --- OrderBook::default ---

    #[test]
    fn order_book_default_is_empty() {
        let book = OrderBook::default();
        assert!(book.best_bid().is_none());
        assert!(book.best_ask().is_none());
    }

    // --- PriceLevel ---

    #[test]
    fn price_level_new_is_empty() {
        let lvl = PriceLevel::new(1000);
        assert_eq!(lvl.price, 1000);
        assert_eq!(lvl.total_quantity, 0);
        assert!(lvl.is_empty());
    }

    #[test]
    fn price_level_push_updates_total_quantity() {
        let mut lvl = PriceLevel::new(1000);
        let order = Order {
            id: OrderId(1),
            side: Side::Ask,
            order_type: OrderType::Limit,
            price: 1000,
            quantity: 15,
            filled_quantity: 5,
            timestamp_ns: 0,
            time_in_force: TimeInForce::GTC,
        };
        lvl.push(order);
        // remaining = 15 - 5 = 10
        assert_eq!(lvl.total_quantity, 10);
        assert!(!lvl.is_empty());
    }

    // --- StopLimit 注文が rest される ---

    #[test]
    fn stop_limit_order_rests_on_book() {
        let mut book = OrderBook::new();
        let sl = Order {
            id: OrderId(1),
            side: Side::Bid,
            order_type: OrderType::StopLimit { stop_price: 980 },
            price: 990,
            quantity: 10,
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::GTC,
        };
        let fills = book.insert(sl);
        assert!(fills.is_empty());
        assert_eq!(book.best_bid(), Some(990));
    }

    // --- GTD insert_with_time: now_ns = 0 → 期限チェックをスキップ ---

    #[test]
    fn insert_with_time_zero_skips_expiry_check() {
        let mut book = OrderBook::new();
        // expiry_ns = 1 だが now_ns = 0 → チェックなしで rest する
        let fills = book.insert_with_time(gtd(1, Side::Bid, 990, 10, 0, 1), 0);
        assert!(fills.is_empty());
        assert_eq!(book.best_bid(), Some(990));
    }

    // --- property-based tests ---

    use proptest::prelude::*;

    proptest! {
        /// FIFO priority: two ask orders at the same price; the taker buy must
        /// fill the earliest-inserted (lowest id) maker first.
        #[test]
        fn prop_fifo_same_price_earliest_filled_first(
            price in 1_i64..=10_000_i64,
            qty in 1_u64..=100_u64,
        ) {
            let mut book = OrderBook::new();
            // Order 1 inserted earlier (ts=0), order 2 inserted later (ts=1).
            book.insert(limit(1, Side::Ask, price, qty, 0));
            book.insert(limit(2, Side::Ask, price, qty, 1));

            // Taker buys exactly `qty` lots — should fill against order 1 only.
            let fills = book.insert(market(3, Side::Bid, qty));

            prop_assert!(!fills.is_empty(), "expected at least one fill");
            prop_assert_eq!(
                fills[0].maker_id,
                OrderId(1),
                "{}",
                "first fill must come from the earliest-resting order"
            );
        }

        /// Fill at maker price: when a market bid fills against a resting ask,
        /// every fill's price must equal the ask's limit price, not the taker's.
        #[test]
        fn prop_fill_price_equals_maker_price(
            ask_price in 1_i64..=10_000_i64,
            qty in 1_u64..=100_u64,
        ) {
            let mut book = OrderBook::new();
            book.insert(limit(1, Side::Ask, ask_price, qty, 0));

            // Market bid — price field is irrelevant for matching but let's
            // set it to something larger than the ask to guarantee a match.
            let fills = book.insert(market(2, Side::Bid, qty));

            prop_assert!(!fills.is_empty());
            for f in &fills {
                prop_assert_eq!(
                    f.price, ask_price,
                    "{}",
                    "fill price must equal the maker's ask price"
                );
            }
        }

        /// FOK atomicity: a FOK order that cannot be completely filled must
        /// produce zero fills (no partial execution).
        #[test]
        fn prop_fok_no_partial_fill(
            ask_qty in 1_u64..=99_u64,
            // FOK wants strictly more than available, so demand = ask_qty + 1..=200
            extra in 1_u64..=100_u64,
            price in 1_i64..=10_000_i64,
        ) {
            let fok_qty = ask_qty + extra; // guaranteed > ask_qty

            let mut book = OrderBook::new();
            book.insert(limit(1, Side::Ask, price, ask_qty, 0));

            let fok = Order {
                id: OrderId(2),
                side: Side::Bid,
                order_type: OrderType::Limit,
                price,
                quantity: fok_qty,
                filled_quantity: 0,
                timestamp_ns: 0,
                time_in_force: TimeInForce::FOK,
            };
            let fills = book.insert(fok);

            prop_assert!(
                fills.is_empty(),
                "FOK that cannot fill completely must produce zero fills; ask_qty={}, fok_qty={}",
                ask_qty, fok_qty
            );
        }
    }
}
