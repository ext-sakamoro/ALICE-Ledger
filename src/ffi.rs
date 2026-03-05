//! C-ABI FFI for Unity / UE5 / C consumers
//!
//! All functions use the `al_ldg_*` prefix (ALICE-Ledger).
//!
//! ## Repr(C) Structs
//!
//! | Struct | Fields |
//! |--------|--------|
//! | `FfiOrder` | id, side, order_type, stop_price, price, quantity, filled_quantity, timestamp_ns, tif, expiry_ns |
//! | `FfiFill` | maker_id, taker_id, price, quantity, timestamp_ns |
//! | `FfiPosition` | symbol_hash, net_quantity, avg_entry_price, realized_pnl, unrealized_pnl, trade_count |
//! | `FfiDepthLevel` | price, quantity |
//!
//! ## Error Convention
//!
//! Functions returning `i32`: 0 = success / not-found, 1 = found.
//! Functions returning pointers: null = error or empty result.

#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int};
use std::ffi::CString;
use std::sync::OnceLock;

use crate::book::{Fill, OrderBook};
use crate::order::{Order, OrderId, OrderType, Side, TimeInForce};
use crate::position::PositionTracker;

// ============================================================================
// Repr(C) Structs
// ============================================================================

/// C-compatible order.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FfiOrder {
    pub id: u64,
    /// 0 = Bid, 1 = Ask
    pub side: u8,
    /// 0 = Limit, 1 = Market, 2 = StopLimit
    pub order_type: u8,
    /// Only used when order_type == 2 (StopLimit)
    pub stop_price: i64,
    pub price: i64,
    pub quantity: u64,
    pub filled_quantity: u64,
    pub timestamp_ns: u64,
    /// 0 = GTC, 1 = IOC, 2 = FOK, 3 = GTD
    pub tif: u8,
    /// Only used when tif == 3 (GTD)
    pub expiry_ns: u64,
}

/// C-compatible fill record.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FfiFill {
    pub maker_id: u64,
    pub taker_id: u64,
    pub price: i64,
    pub quantity: u64,
    pub timestamp_ns: u64,
}

/// C-compatible position.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FfiPosition {
    pub symbol_hash: u64,
    pub net_quantity: i64,
    pub avg_entry_price: i64,
    pub realized_pnl: i64,
    pub unrealized_pnl: i64,
    pub trade_count: u64,
}

/// C-compatible depth level.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FfiDepthLevel {
    pub price: i64,
    pub quantity: u64,
}

// ============================================================================
// Conversions
// ============================================================================

fn ffi_to_order(o: &FfiOrder) -> Order {
    let side = if o.side == 0 { Side::Bid } else { Side::Ask };
    let order_type = match o.order_type {
        1 => OrderType::Market,
        2 => OrderType::StopLimit {
            stop_price: o.stop_price,
        },
        _ => OrderType::Limit,
    };
    let time_in_force = match o.tif {
        1 => TimeInForce::IOC,
        2 => TimeInForce::FOK,
        3 => TimeInForce::GTD {
            expiry_ns: o.expiry_ns,
        },
        _ => TimeInForce::GTC,
    };
    Order {
        id: OrderId(o.id),
        side,
        order_type,
        price: o.price,
        quantity: o.quantity,
        filled_quantity: o.filled_quantity,
        timestamp_ns: o.timestamp_ns,
        time_in_force,
    }
}

fn order_to_ffi(o: &Order) -> FfiOrder {
    let (order_type, stop_price) = match o.order_type {
        OrderType::Limit => (0, 0),
        OrderType::Market => (1, 0),
        OrderType::StopLimit { stop_price } => (2, stop_price),
    };
    let (tif, expiry_ns) = match o.time_in_force {
        TimeInForce::GTC => (0, 0),
        TimeInForce::IOC => (1, 0),
        TimeInForce::FOK => (2, 0),
        TimeInForce::GTD { expiry_ns } => (3, expiry_ns),
    };
    FfiOrder {
        id: o.id.0,
        side: if o.side == Side::Bid { 0 } else { 1 },
        order_type,
        stop_price,
        price: o.price,
        quantity: o.quantity,
        filled_quantity: o.filled_quantity,
        timestamp_ns: o.timestamp_ns,
        tif,
        expiry_ns,
    }
}

fn fill_to_ffi(f: &Fill) -> FfiFill {
    FfiFill {
        maker_id: f.maker_id.0,
        taker_id: f.taker_id.0,
        price: f.price,
        quantity: f.quantity,
        timestamp_ns: f.timestamp_ns,
    }
}

fn ffi_to_fill(f: &FfiFill) -> Fill {
    Fill {
        maker_id: OrderId(f.maker_id),
        taker_id: OrderId(f.taker_id),
        price: f.price,
        quantity: f.quantity,
        timestamp_ns: f.timestamp_ns,
    }
}

/// Convert a Vec<FfiFill> into a heap-allocated array. Returns (ptr, count).
fn fills_to_raw(fills: Vec<FfiFill>) -> (*mut FfiFill, c_int) {
    if fills.is_empty() {
        return (std::ptr::null_mut(), 0);
    }
    let len = fills.len() as c_int;
    let boxed = fills.into_boxed_slice();
    let ptr = Box::into_raw(boxed) as *mut FfiFill;
    (ptr, len)
}

// ============================================================================
// Memory Management (3)
// ============================================================================

/// Free a fills array returned by `al_ldg_book_insert*`.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_fills_free(ptr: *mut FfiFill, count: c_int) {
    if !ptr.is_null() && count > 0 {
        let slice = std::slice::from_raw_parts_mut(ptr, count as usize);
        drop(Box::from_raw(slice as *mut [FfiFill]));
    }
}

/// Free a depth array returned by `al_ldg_book_depth`.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_depth_free(ptr: *mut FfiDepthLevel, count: c_int) {
    if !ptr.is_null() && count > 0 {
        let slice = std::slice::from_raw_parts_mut(ptr, count as usize);
        drop(Box::from_raw(slice as *mut [FfiDepthLevel]));
    }
}

/// Free an orders array returned by `al_ldg_book_purge_expired`.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_orders_free(ptr: *mut FfiOrder, count: c_int) {
    if !ptr.is_null() && count > 0 {
        let slice = std::slice::from_raw_parts_mut(ptr, count as usize);
        drop(Box::from_raw(slice as *mut [FfiOrder]));
    }
}

// ============================================================================
// OrderBook (11)
// ============================================================================

/// Create a new, empty order book.
#[no_mangle]
pub extern "C" fn al_ldg_book_new() -> *mut OrderBook {
    Box::into_raw(Box::new(OrderBook::new()))
}

/// Free an order book.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_book_free(ptr: *mut OrderBook) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
}

/// Insert an order (no GTD time check). Returns fills array; writes count to `out_count`.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_book_insert(
    book: *mut OrderBook,
    order: *const FfiOrder,
    out_count: *mut c_int,
) -> *mut FfiFill {
    if book.is_null() || order.is_null() || out_count.is_null() {
        return std::ptr::null_mut();
    }
    let rust_order = ffi_to_order(&*order);
    let fills: Vec<FfiFill> = (*book).insert(rust_order).iter().map(fill_to_ffi).collect();
    let (ptr, len) = fills_to_raw(fills);
    *out_count = len;
    ptr
}

/// Insert an order with GTD expiry check. Returns fills array.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_book_insert_with_time(
    book: *mut OrderBook,
    order: *const FfiOrder,
    now_ns: u64,
    out_count: *mut c_int,
) -> *mut FfiFill {
    if book.is_null() || order.is_null() || out_count.is_null() {
        return std::ptr::null_mut();
    }
    let rust_order = ffi_to_order(&*order);
    let fills: Vec<FfiFill> = (*book)
        .insert_with_time(rust_order, now_ns)
        .iter()
        .map(fill_to_ffi)
        .collect();
    let (ptr, len) = fills_to_raw(fills);
    *out_count = len;
    ptr
}

/// Cancel an order by ID. Writes cancelled order to `out` if found.
/// Returns 1 if found, 0 if not found.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_book_cancel(
    book: *mut OrderBook,
    id: u64,
    out: *mut FfiOrder,
) -> c_int {
    if book.is_null() {
        return 0;
    }
    match (*book).cancel(OrderId(id)) {
        Some(order) => {
            if !out.is_null() {
                *out = order_to_ffi(&order);
            }
            1
        }
        None => 0,
    }
}

/// Best bid price in ticks, or `i64::MIN` if no bids.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_book_best_bid(book: *const OrderBook) -> i64 {
    if book.is_null() {
        return i64::MIN;
    }
    (*book).best_bid().unwrap_or(i64::MIN)
}

/// Best ask price in ticks, or `i64::MAX` if no asks.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_book_best_ask(book: *const OrderBook) -> i64 {
    if book.is_null() {
        return i64::MAX;
    }
    (*book).best_ask().unwrap_or(i64::MAX)
}

/// Bid-ask spread in ticks, or `i64::MIN` if either side is empty.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_book_spread(book: *const OrderBook) -> i64 {
    if book.is_null() {
        return i64::MIN;
    }
    (*book).spread().unwrap_or(i64::MIN)
}

/// Market depth. Returns array of (price, quantity) pairs; writes count to `out_count`.
///
/// `side`: 0 = Bid, 1 = Ask.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_book_depth(
    book: *const OrderBook,
    side: u8,
    levels: c_int,
    out_count: *mut c_int,
) -> *mut FfiDepthLevel {
    if book.is_null() || out_count.is_null() || levels <= 0 {
        if !out_count.is_null() {
            *out_count = 0;
        }
        return std::ptr::null_mut();
    }
    let s = if side == 0 { Side::Bid } else { Side::Ask };
    let depth: Vec<FfiDepthLevel> = (*book)
        .depth(s, levels as usize)
        .iter()
        .map(|&(price, quantity)| FfiDepthLevel { price, quantity })
        .collect();
    if depth.is_empty() {
        *out_count = 0;
        return std::ptr::null_mut();
    }
    let len = depth.len() as c_int;
    let boxed = depth.into_boxed_slice();
    let ptr = Box::into_raw(boxed) as *mut FfiDepthLevel;
    *out_count = len;
    ptr
}

/// Purge expired GTD orders. Returns array of cancelled orders.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_book_purge_expired(
    book: *mut OrderBook,
    now_ns: u64,
    out_count: *mut c_int,
) -> *mut FfiOrder {
    if book.is_null() || out_count.is_null() {
        if !out_count.is_null() {
            *out_count = 0;
        }
        return std::ptr::null_mut();
    }
    let expired: Vec<FfiOrder> = (*book)
        .purge_expired(now_ns)
        .iter()
        .map(order_to_ffi)
        .collect();
    if expired.is_empty() {
        *out_count = 0;
        return std::ptr::null_mut();
    }
    let len = expired.len() as c_int;
    let boxed = expired.into_boxed_slice();
    let ptr = Box::into_raw(boxed) as *mut FfiOrder;
    *out_count = len;
    ptr
}

/// Total number of resting orders in the book.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_book_order_count(book: *const OrderBook) -> c_int {
    if book.is_null() {
        return 0;
    }
    // Count all orders across bid and ask levels
    let mut count = 0i32;
    let depth_bids = (*book).depth(Side::Bid, usize::MAX);
    let depth_asks = (*book).depth(Side::Ask, usize::MAX);
    for (_, qty) in &depth_bids {
        count += *qty as i32;
    }
    for (_, qty) in &depth_asks {
        count += *qty as i32;
    }
    count
}

// ============================================================================
// PositionTracker (5)
// ============================================================================

/// Create an empty position tracker.
#[no_mangle]
pub extern "C" fn al_ldg_tracker_new() -> *mut PositionTracker {
    Box::into_raw(Box::new(PositionTracker::new()))
}

/// Free a position tracker.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_tracker_free(ptr: *mut PositionTracker) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
}

/// Apply a fill to a position. `side`: 0 = Bid, 1 = Ask.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_tracker_apply_fill(
    tracker: *mut PositionTracker,
    symbol_hash: u64,
    fill: *const FfiFill,
    side: u8,
) {
    if tracker.is_null() || fill.is_null() {
        return;
    }
    let s = if side == 0 { Side::Bid } else { Side::Ask };
    let rust_fill = ffi_to_fill(&*fill);
    (*tracker).apply_fill(symbol_hash, &rust_fill, s);
}

/// Revalue unrealized P&L at `current_price`.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_tracker_mark_to_market(
    tracker: *mut PositionTracker,
    symbol_hash: u64,
    current_price: i64,
) {
    if tracker.is_null() {
        return;
    }
    (*tracker).mark_to_market(symbol_hash, current_price);
}

/// Get position for a symbol. Writes to `out` if found.
/// Returns 1 if found, 0 if not found.
#[no_mangle]
pub unsafe extern "C" fn al_ldg_tracker_get(
    tracker: *const PositionTracker,
    symbol_hash: u64,
    out: *mut FfiPosition,
) -> c_int {
    if tracker.is_null() || out.is_null() {
        return 0;
    }
    match (*tracker).get(symbol_hash) {
        Some(pos) => {
            *out = FfiPosition {
                symbol_hash: pos.symbol_hash,
                net_quantity: pos.net_quantity,
                avg_entry_price: pos.avg_entry_price,
                realized_pnl: pos.realized_pnl,
                unrealized_pnl: pos.unrealized_pnl,
                trade_count: pos.trade_count,
            };
            1
        }
        None => 0,
    }
}

// ============================================================================
// Utility (1)
// ============================================================================

/// Library version string. Do NOT free the returned pointer.
#[no_mangle]
pub extern "C" fn al_ldg_version() -> *const c_char {
    static VERSION: OnceLock<CString> = OnceLock::new();
    VERSION
        .get_or_init(|| {
            CString::new(env!("CARGO_PKG_VERSION"))
                .unwrap_or_else(|_| CString::new("0.0.0").unwrap())
        })
        .as_ptr()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_limit_bid(id: u64, price: i64, qty: u64) -> FfiOrder {
        FfiOrder {
            id,
            side: 0,
            order_type: 0,
            stop_price: 0,
            price,
            quantity: qty,
            filled_quantity: 0,
            timestamp_ns: id,
            tif: 0,
            expiry_ns: 0,
        }
    }

    fn make_limit_ask(id: u64, price: i64, qty: u64) -> FfiOrder {
        FfiOrder {
            id,
            side: 1,
            order_type: 0,
            stop_price: 0,
            price,
            quantity: qty,
            filled_quantity: 0,
            timestamp_ns: id,
            tif: 0,
            expiry_ns: 0,
        }
    }

    fn make_market_bid(id: u64, qty: u64) -> FfiOrder {
        FfiOrder {
            id,
            side: 0,
            order_type: 1,
            stop_price: 0,
            price: 0,
            quantity: qty,
            filled_quantity: 0,
            timestamp_ns: id,
            tif: 1,
            expiry_ns: 0,
        }
    }

    #[test]
    fn test_book_lifecycle() {
        unsafe {
            let book = al_ldg_book_new();
            assert!(!book.is_null());

            // Insert resting ask
            let ask = make_limit_ask(1, 1000, 10);
            let mut count: c_int = 0;
            let fills = al_ldg_book_insert(book, &ask, &mut count);
            assert_eq!(count, 0);
            assert!(fills.is_null());

            assert_eq!(al_ldg_book_best_ask(book), 1000);
            assert_eq!(al_ldg_book_best_bid(book), i64::MIN);

            // Insert market bid
            let bid = make_market_bid(2, 5);
            let fills = al_ldg_book_insert(book, &bid, &mut count);
            assert_eq!(count, 1);
            assert!(!fills.is_null());
            assert_eq!((*fills).price, 1000);
            assert_eq!((*fills).quantity, 5);
            al_ldg_fills_free(fills, count);

            al_ldg_book_free(book);
        }
    }

    #[test]
    fn test_cancel() {
        unsafe {
            let book = al_ldg_book_new();
            let ask = make_limit_ask(1, 1000, 10);
            let mut count: c_int = 0;
            al_ldg_book_insert(book, &ask, &mut count);

            let mut out = std::mem::zeroed::<FfiOrder>();
            assert_eq!(al_ldg_book_cancel(book, 1, &mut out), 1);
            assert_eq!(out.id, 1);
            assert_eq!(out.price, 1000);

            // Cancel again — not found
            assert_eq!(al_ldg_book_cancel(book, 1, &mut out), 0);

            al_ldg_book_free(book);
        }
    }

    #[test]
    fn test_depth() {
        unsafe {
            let book = al_ldg_book_new();
            let mut count: c_int = 0;
            al_ldg_book_insert(book, &make_limit_bid(1, 995, 10), &mut count);
            al_ldg_book_insert(book, &make_limit_bid(2, 990, 20), &mut count);

            let mut depth_count: c_int = 0;
            let depth = al_ldg_book_depth(book, 0, 5, &mut depth_count);
            assert_eq!(depth_count, 2);
            assert_eq!((*depth).price, 995);
            assert_eq!((*depth.add(1)).price, 990);
            al_ldg_depth_free(depth, depth_count);

            al_ldg_book_free(book);
        }
    }

    #[test]
    fn test_spread() {
        unsafe {
            let book = al_ldg_book_new();
            let mut count: c_int = 0;
            al_ldg_book_insert(book, &make_limit_bid(1, 995, 10), &mut count);
            al_ldg_book_insert(book, &make_limit_ask(2, 1000, 10), &mut count);
            assert_eq!(al_ldg_book_spread(book), 5);
            al_ldg_book_free(book);
        }
    }

    #[test]
    fn test_tracker_lifecycle() {
        unsafe {
            let tracker = al_ldg_tracker_new();
            assert!(!tracker.is_null());

            let fill = FfiFill {
                maker_id: 1,
                taker_id: 2,
                price: 1000,
                quantity: 10,
                timestamp_ns: 0,
            };
            let sym: u64 = 0xDEAD_BEEF;

            al_ldg_tracker_apply_fill(tracker, sym, &fill, 0); // Bid
            al_ldg_tracker_mark_to_market(tracker, sym, 1010);

            let mut pos = std::mem::zeroed::<FfiPosition>();
            assert_eq!(al_ldg_tracker_get(tracker, sym, &mut pos), 1);
            assert_eq!(pos.net_quantity, 10);
            assert_eq!(pos.avg_entry_price, 1000);
            assert_eq!(pos.unrealized_pnl, 100); // 10 * (1010 - 1000)

            // Unknown symbol
            assert_eq!(al_ldg_tracker_get(tracker, 0xCAFE, &mut pos), 0);

            al_ldg_tracker_free(tracker);
        }
    }

    #[test]
    fn test_null_safety() {
        unsafe {
            al_ldg_book_free(std::ptr::null_mut());
            al_ldg_tracker_free(std::ptr::null_mut());
            al_ldg_fills_free(std::ptr::null_mut(), 0);
            al_ldg_depth_free(std::ptr::null_mut(), 0);
            al_ldg_orders_free(std::ptr::null_mut(), 0);

            assert_eq!(al_ldg_book_best_bid(std::ptr::null()), i64::MIN);
            assert_eq!(al_ldg_book_best_ask(std::ptr::null()), i64::MAX);
            assert_eq!(al_ldg_book_spread(std::ptr::null()), i64::MIN);
            assert_eq!(
                al_ldg_book_cancel(std::ptr::null_mut(), 0, std::ptr::null_mut()),
                0
            );
            assert_eq!(al_ldg_book_order_count(std::ptr::null()), 0);
            assert_eq!(
                al_ldg_tracker_get(std::ptr::null(), 0, std::ptr::null_mut()),
                0
            );

            let mut count: c_int = 0;
            assert!(
                al_ldg_book_insert(std::ptr::null_mut(), std::ptr::null(), &mut count).is_null()
            );
            assert!(al_ldg_book_depth(std::ptr::null(), 0, 5, &mut count).is_null());

            al_ldg_tracker_apply_fill(std::ptr::null_mut(), 0, std::ptr::null(), 0);
            al_ldg_tracker_mark_to_market(std::ptr::null_mut(), 0, 0);
        }
    }

    #[test]
    fn test_purge_expired() {
        unsafe {
            let book = al_ldg_book_new();
            let mut count: c_int = 0;

            // GTD bid expires at 500
            let gtd_order = FfiOrder {
                id: 1,
                side: 0,
                order_type: 0,
                stop_price: 0,
                price: 990,
                quantity: 10,
                filled_quantity: 0,
                timestamp_ns: 0,
                tif: 3,
                expiry_ns: 500,
            };
            al_ldg_book_insert_with_time(book, &gtd_order, 0, &mut count);

            // GTC bid (should survive)
            al_ldg_book_insert(book, &make_limit_bid(2, 995, 5), &mut count);

            let mut expired_count: c_int = 0;
            let expired = al_ldg_book_purge_expired(book, 1000, &mut expired_count);
            assert_eq!(expired_count, 1);
            assert_eq!((*expired).id, 1);
            al_ldg_orders_free(expired, expired_count);

            assert_eq!(al_ldg_book_best_bid(book), 995);
            al_ldg_book_free(book);
        }
    }

    #[test]
    fn test_version() {
        let ptr = al_ldg_version();
        assert!(!ptr.is_null());
        let v = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_str().unwrap();
        assert!(v.starts_with("0."));
    }

    #[test]
    fn test_order_conversion_roundtrip() {
        let ffi = FfiOrder {
            id: 42,
            side: 1,
            order_type: 2,
            stop_price: 5000,
            price: 4900,
            quantity: 100,
            filled_quantity: 25,
            timestamp_ns: 99,
            tif: 3,
            expiry_ns: 9999,
        };
        let rust = ffi_to_order(&ffi);
        let back = order_to_ffi(&rust);
        assert_eq!(ffi.id, back.id);
        assert_eq!(ffi.side, back.side);
        assert_eq!(ffi.order_type, back.order_type);
        assert_eq!(ffi.stop_price, back.stop_price);
        assert_eq!(ffi.price, back.price);
        assert_eq!(ffi.quantity, back.quantity);
        assert_eq!(ffi.filled_quantity, back.filled_quantity);
        assert_eq!(ffi.tif, back.tif);
        assert_eq!(ffi.expiry_ns, back.expiry_ns);
    }
}
