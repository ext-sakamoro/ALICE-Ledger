#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::inline_always
)]
/*
    ALICE-Ledger
    Copyright (C) 2026 Moroya Sakamoto
*/

//! # ALICE-Ledger
//!
//! Order book, matching engine, and position management for financial systems.
//!
//! Prices are represented as `i64` ticks — the smallest discrete price unit
//! for the instrument — matching the deterministic fixed-point arithmetic
//! exported by ALICE-Sync (`Fixed`, Q16.16). This guarantees bit-exact,
//! cross-platform results with no floating-point drift.
//!
//! ## Modules
//!
//! - [`order`] — `Order`, `OrderId`, `Side`, `OrderType`, `TimeInForce`
//! - [`book`]  — `OrderBook`, `PriceLevel`, `Fill` (price-time priority LOB)
//! - [`position`] — `Position`, `PositionTracker` (P&L accounting)
//!
//! ## Example
//!
//! ```rust
//! use alice_ledger::{
//!     book::OrderBook,
//!     order::{Order, OrderId, OrderType, Side, TimeInForce},
//!     position::PositionTracker,
//! };
//!
//! let mut book = OrderBook::new();
//! let mut tracker = PositionTracker::new();
//! const SYM: u64 = 0x4254435553445400; // "BTCUSDT\0" as u64
//!
//! // Post a resting ask at 50_000 ticks for 1 lot.
//! book.insert(Order {
//!     id: OrderId(1),
//!     side: Side::Ask,
//!     order_type: OrderType::Limit,
//!     price: 50_000,
//!     quantity: 1,
//!     filled_quantity: 0,
//!     timestamp_ns: 0,
//!     time_in_force: TimeInForce::GTC,
//! });
//!
//! // Aggress with a market buy.
//! let fills = book.insert(Order {
//!     id: OrderId(2),
//!     side: Side::Bid,
//!     order_type: OrderType::Market,
//!     price: 0,
//!     quantity: 1,
//!     filled_quantity: 0,
//!     timestamp_ns: 1,
//!     time_in_force: TimeInForce::IOC,
//! });
//!
//! assert_eq!(fills.len(), 1);
//! assert_eq!(fills[0].price, 50_000);
//!
//! // Update position from the fill.
//! tracker.apply_fill(SYM, &fills[0], Side::Bid);
//! tracker.mark_to_market(SYM, 51_000);
//!
//! let pos = tracker.get(SYM).unwrap();
//! assert_eq!(pos.net_quantity, 1);
//! assert_eq!(pos.unrealized_pnl, 1_000);
//! ```

pub mod book;
pub mod bulk;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod order;
pub mod position;
pub mod post_only;
pub mod risk;

// Re-export the most commonly used types at the crate root for ergonomics.
pub use book::{Fill, OrderBook, PriceLevel};
pub use order::{Order, OrderId, OrderType, Side, TimeInForce};
pub use position::{Position, PositionTracker};

/// ALICE-Ledger crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
