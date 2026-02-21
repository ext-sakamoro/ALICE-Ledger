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

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::cmp::Reverse;

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
    pub fn new(price: i64) -> Self {
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
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            order_index: HashMap::new(),
        }
    }

    /// Best bid price (highest) in ticks, or `None` if the bid side is empty.
    #[inline(always)]
    pub fn best_bid(&self) -> Option<i64> {
        self.bids.keys().next().map(|r| r.0)
    }

    /// Best ask price (lowest) in ticks, or `None` if the ask side is empty.
    #[inline(always)]
    pub fn best_ask(&self) -> Option<i64> {
        self.asks.keys().next().copied()
    }

    /// Bid-ask spread in ticks, or `None` when either side is empty.
    #[inline(always)]
    pub fn spread(&self) -> Option<i64> {
        match (self.best_ask(), self.best_bid()) {
            (Some(ask), Some(bid)) => Some(ask - bid),
            _ => None,
        }
    }

    /// Return up to `levels` price levels on `side` as `(price, quantity)` pairs.
    ///
    /// Bids are returned in descending price order; asks in ascending price order.
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
    pub fn insert(&mut self, mut order: Order) -> Vec<Fill> {
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
                    if self.bids.get(&Reverse(price)).map_or(true, |l| l.is_empty()) {
                        self.bids.remove(&Reverse(price));
                    }
                    let _ = order; // already moved
                }
                Side::Ask => {
                    if self.asks.get(&price).map_or(true, |l| l.is_empty()) {
                        self.asks.remove(&price);
                    }
                }
            }
        }

        cancelled
    }

    // -----------------------------------------------------------------------
    // Private matching helpers
    // -----------------------------------------------------------------------

    /// Match an incoming bid against resting asks, from lowest ask price up.
    fn match_bid(&mut self, taker: &mut Order, fills: &mut Vec<Fill>) {
        // For FOK we must verify full fill is possible before executing.
        if taker.time_in_force == TimeInForce::FOK {
            if !self.can_fill_bid(taker) {
                return; // Cancel without any execution.
            }
        }

        let taker_price = match taker.order_type {
            OrderType::Market => i64::MAX,
            OrderType::Limit | OrderType::StopLimit { .. } => taker.price,
        };

        // Collect keys to avoid borrowing issues.
        let keys: Vec<i64> = self
            .asks
            .range(..=taker_price)
            .map(|(k, _)| *k)
            .collect();

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
            if self.asks.get(&ask_price).map_or(false, |l| l.is_empty()) {
                self.asks.remove(&ask_price);
            }
        }
    }

    /// Match an incoming ask against resting bids, from highest bid price down.
    fn match_ask(&mut self, taker: &mut Order, fills: &mut Vec<Fill>) {
        if taker.time_in_force == TimeInForce::FOK {
            if !self.can_fill_ask(taker) {
                return;
            }
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
            if self.bids.get(&Reverse(bid_price)).map_or(false, |l| l.is_empty()) {
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
}
