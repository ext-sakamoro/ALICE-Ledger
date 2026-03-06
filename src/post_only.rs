/*
    ALICE-Ledger
    Copyright (C) 2026 Moroya Sakamoto
*/

//! Post-only orders and mass cancellation.
//!
//! Post-only: aggressive matching を防止し、必ず板に載る注文。
//! Cancel-all: 指定サイドの全注文を一括キャンセル。

use crate::book::OrderBook;
use crate::order::{Order, OrderId, OrderType, Side};

/// Post-only チェック結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostOnlyResult {
    /// 板に載る (passive)。
    WouldRest,
    /// 即時約定してしまう (aggressive) → 拒否。
    WouldMatch,
    /// Market注文は Post-only 不可。
    InvalidOrderType,
}

/// 注文が Post-only 条件を満たすか判定。
///
/// Post-only 注文は板に載る (resting) 場合のみ受理される。
/// 即時約定してしまう価格の場合は拒否。
#[must_use]
pub fn check_post_only(order: &Order, book: &OrderBook) -> PostOnlyResult {
    // Market注文は Post-only 不可
    if order.order_type == OrderType::Market {
        return PostOnlyResult::InvalidOrderType;
    }

    match order.side {
        Side::Bid => {
            // Bid注文: best ask 以上の価格なら即時約定
            if let Some(best_ask) = book.best_ask() {
                if order.price >= best_ask {
                    return PostOnlyResult::WouldMatch;
                }
            }
        }
        Side::Ask => {
            // Ask注文: best bid 以下の価格なら即時約定
            if let Some(best_bid) = book.best_bid() {
                if order.price <= best_bid {
                    return PostOnlyResult::WouldMatch;
                }
            }
        }
    }

    PostOnlyResult::WouldRest
}

/// Cancel-all 結果。
#[derive(Debug, Clone)]
pub struct CancelAllResult {
    /// キャンセルされた注文 ID。
    pub cancelled_ids: Vec<OrderId>,
    /// キャンセル数。
    pub count: usize,
}

/// 指定サイドの全注文を一括キャンセル。
pub fn cancel_all_side(book: &mut OrderBook, side: Side) -> CancelAllResult {
    let ids = book.all_order_ids_by_side(side);
    let count = ids.len();
    for &id in &ids {
        book.cancel(id);
    }
    CancelAllResult {
        cancelled_ids: ids,
        count,
    }
}

/// 全注文を一括キャンセル (両サイド)。
pub fn cancel_all(book: &mut OrderBook) -> CancelAllResult {
    let bid_result = cancel_all_side(book, Side::Bid);
    let ask_result = cancel_all_side(book, Side::Ask);

    let mut cancelled_ids = bid_result.cancelled_ids;
    cancelled_ids.extend(ask_result.cancelled_ids);
    let count = cancelled_ids.len();

    CancelAllResult {
        cancelled_ids,
        count,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::TimeInForce;

    fn make_limit(id: u64, side: Side, price: i64, qty: u64) -> Order {
        Order {
            id: OrderId(id),
            side,
            order_type: OrderType::Limit,
            price,
            quantity: qty,
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::GTC,
        }
    }

    fn make_market(id: u64, side: Side, qty: u64) -> Order {
        Order {
            id: OrderId(id),
            side,
            order_type: OrderType::Market,
            price: 0,
            quantity: qty,
            filled_quantity: 0,
            timestamp_ns: 0,
            time_in_force: TimeInForce::IOC,
        }
    }

    #[test]
    fn post_only_bid_below_ask_passes() {
        let mut book = OrderBook::new();
        book.insert(make_limit(1, Side::Ask, 100, 10));
        let bid = make_limit(2, Side::Bid, 99, 5);
        assert_eq!(check_post_only(&bid, &book), PostOnlyResult::WouldRest);
    }

    #[test]
    fn post_only_bid_at_ask_rejected() {
        let mut book = OrderBook::new();
        book.insert(make_limit(1, Side::Ask, 100, 10));
        let bid = make_limit(2, Side::Bid, 100, 5);
        assert_eq!(check_post_only(&bid, &book), PostOnlyResult::WouldMatch);
    }

    #[test]
    fn post_only_bid_above_ask_rejected() {
        let mut book = OrderBook::new();
        book.insert(make_limit(1, Side::Ask, 100, 10));
        let bid = make_limit(2, Side::Bid, 105, 5);
        assert_eq!(check_post_only(&bid, &book), PostOnlyResult::WouldMatch);
    }

    #[test]
    fn post_only_ask_above_bid_passes() {
        let mut book = OrderBook::new();
        book.insert(make_limit(1, Side::Bid, 100, 10));
        let ask = make_limit(2, Side::Ask, 101, 5);
        assert_eq!(check_post_only(&ask, &book), PostOnlyResult::WouldRest);
    }

    #[test]
    fn post_only_ask_at_bid_rejected() {
        let mut book = OrderBook::new();
        book.insert(make_limit(1, Side::Bid, 100, 10));
        let ask = make_limit(2, Side::Ask, 100, 5);
        assert_eq!(check_post_only(&ask, &book), PostOnlyResult::WouldMatch);
    }

    #[test]
    fn post_only_empty_book_passes() {
        let book = OrderBook::new();
        let bid = make_limit(1, Side::Bid, 100, 10);
        assert_eq!(check_post_only(&bid, &book), PostOnlyResult::WouldRest);
    }

    #[test]
    fn post_only_market_invalid() {
        let book = OrderBook::new();
        let m = make_market(1, Side::Bid, 10);
        assert_eq!(check_post_only(&m, &book), PostOnlyResult::InvalidOrderType);
    }

    #[test]
    fn cancel_all_side_empty_book() {
        let mut book = OrderBook::new();
        let r = cancel_all_side(&mut book, Side::Bid);
        assert_eq!(r.count, 0);
        assert!(r.cancelled_ids.is_empty());
    }

    #[test]
    fn cancel_all_side_removes_bids() {
        let mut book = OrderBook::new();
        book.insert(make_limit(1, Side::Bid, 100, 10));
        book.insert(make_limit(2, Side::Bid, 99, 5));
        book.insert(make_limit(3, Side::Ask, 110, 8));
        let r = cancel_all_side(&mut book, Side::Bid);
        assert_eq!(r.count, 2);
        // ask は残っている
        assert!(book.best_ask().is_some());
    }

    #[test]
    fn cancel_all_both_sides() {
        let mut book = OrderBook::new();
        book.insert(make_limit(1, Side::Bid, 100, 10));
        book.insert(make_limit(2, Side::Ask, 110, 5));
        let r = cancel_all(&mut book);
        assert_eq!(r.count, 2);
        assert!(book.best_bid().is_none());
        assert!(book.best_ask().is_none());
    }

    #[test]
    fn post_only_result_eq() {
        assert_eq!(PostOnlyResult::WouldRest, PostOnlyResult::WouldRest);
        assert_ne!(PostOnlyResult::WouldRest, PostOnlyResult::WouldMatch);
    }
}
