//! Convenience re-export (= `use alice_ledger::prelude::*;` で主要 API 一括取得)
//!
//! Order book + matching engine + position management の 4 core module
//! (order / book / position / fix + signed_order) から主要型 + 関数を提供
//! `bulk` / `post_only` / `risk` / `ffi` (feature-gated) は補助 module のため
//! prelude 非対象

pub use crate::book::{Fill, OrderBook, PriceLevel};
pub use crate::fix::{parse as parse_fix, serialize as serialize_fix, FixError, FixMessage, SOH};
pub use crate::order::{Order, OrderId, OrderType, Side, TimeInForce};
pub use crate::position::{Position, PositionTracker};
pub use crate::signed_order::{
    AuditEvent, AuditEventKind, OrderAuditLog, OrderPayload, OrderSide as SignedOrderSide,
    SignedOrder,
};
