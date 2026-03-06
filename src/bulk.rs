/*
    ALICE-Ledger
    Copyright (C) 2026 Moroya Sakamoto
*/

//! Bulk order validation — バッチ注文サブミッション最適化。
//!
//! 複数注文の一括バリデーションとリスクチェック。

use crate::order::{Order, OrderId, Side};
use crate::risk::{RiskCheckResult, RiskManager};

/// バッチ注文のバリデーション結果。
#[derive(Debug, Clone)]
pub struct BulkValidationResult {
    /// 通過した注文。
    pub accepted: Vec<OrderId>,
    /// 拒否された注文とその理由。
    pub rejected: Vec<(OrderId, BulkRejectReason)>,
}

/// バルク拒否理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkRejectReason {
    /// リスクチェック失敗。
    RiskCheck(String),
    /// 重複 ID。
    DuplicateId,
    /// 数量ゼロ。
    ZeroQuantity,
    /// 自己約定 (同一バッチ内のBid/Ask が交差)。
    SelfTrade,
}

/// バルクバリデータ。
#[derive(Debug)]
pub struct BulkValidator {
    /// 自己約定チェック有効化。
    pub self_trade_prevention: bool,
    /// バッチサイズ上限。
    pub max_batch_size: usize,
}

impl Default for BulkValidator {
    fn default() -> Self {
        Self {
            self_trade_prevention: true,
            max_batch_size: 1000,
        }
    }
}

impl BulkValidator {
    /// 新しいバリデータを作成。
    #[must_use]
    pub const fn new(self_trade_prevention: bool, max_batch_size: usize) -> Self {
        Self {
            self_trade_prevention,
            max_batch_size,
        }
    }

    /// バッチ注文のバリデーション。
    #[must_use]
    pub fn validate(
        &self,
        orders: &[Order],
        risk_manager: &RiskManager,
        current_position: i64,
    ) -> BulkValidationResult {
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();

        // 重複 ID チェック用
        let mut seen_ids = std::collections::HashSet::new();

        // 自己約定チェック用: 同一バッチ内の best bid/ask を追跡
        let mut best_bid: Option<i64> = None;
        let mut best_ask: Option<i64> = None;

        let mut cumulative_position = current_position;

        for order in orders {
            // 重複 ID チェック
            if !seen_ids.insert(order.id) {
                rejected.push((order.id, BulkRejectReason::DuplicateId));
                continue;
            }

            // 数量ゼロ
            if order.quantity == 0 {
                rejected.push((order.id, BulkRejectReason::ZeroQuantity));
                continue;
            }

            // 自己約定チェック
            if self.self_trade_prevention {
                match order.side {
                    Side::Bid => {
                        if let Some(ask) = best_ask {
                            if order.price >= ask {
                                rejected.push((order.id, BulkRejectReason::SelfTrade));
                                continue;
                            }
                        }
                        best_bid = Some(best_bid.map_or(order.price, |b: i64| b.max(order.price)));
                    }
                    Side::Ask => {
                        if let Some(bid) = best_bid {
                            if order.price <= bid {
                                rejected.push((order.id, BulkRejectReason::SelfTrade));
                                continue;
                            }
                        }
                        best_ask = Some(best_ask.map_or(order.price, |a: i64| a.min(order.price)));
                    }
                }
            }

            // リスクチェック
            let risk_result = risk_manager.check_order(order, cumulative_position);
            match risk_result {
                RiskCheckResult::Passed => {
                    // 累積ポジション更新
                    let delta = order.quantity as i64;
                    match order.side {
                        Side::Bid => cumulative_position += delta,
                        Side::Ask => cumulative_position -= delta,
                    }
                    accepted.push(order.id);
                }
                RiskCheckResult::Rejected(reason) => {
                    rejected.push((order.id, BulkRejectReason::RiskCheck(format!("{reason:?}"))));
                }
            }
        }

        BulkValidationResult { accepted, rejected }
    }

    /// バッチサイズが上限以内かチェック。
    #[must_use]
    pub const fn check_batch_size(&self, count: usize) -> bool {
        count <= self.max_batch_size
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{OrderType, TimeInForce};
    use crate::risk::RiskLimits;

    fn make_order(id: u64, side: Side, price: i64, qty: u64) -> Order {
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

    #[test]
    fn empty_batch() {
        let v = BulkValidator::default();
        let rm = RiskManager::with_defaults();
        let r = v.validate(&[], &rm, 0);
        assert!(r.accepted.is_empty());
        assert!(r.rejected.is_empty());
    }

    #[test]
    fn single_valid_order() {
        let v = BulkValidator::default();
        let rm = RiskManager::with_defaults();
        let orders = [make_order(1, Side::Bid, 100, 10)];
        let r = v.validate(&orders, &rm, 0);
        assert_eq!(r.accepted.len(), 1);
        assert!(r.rejected.is_empty());
    }

    #[test]
    fn multiple_valid_orders() {
        let v = BulkValidator::default();
        let rm = RiskManager::with_defaults();
        let orders = [
            make_order(1, Side::Bid, 100, 10),
            make_order(2, Side::Bid, 99, 20),
            make_order(3, Side::Ask, 110, 5),
        ];
        let r = v.validate(&orders, &rm, 0);
        assert_eq!(r.accepted.len(), 3);
    }

    #[test]
    fn duplicate_id_rejected() {
        let v = BulkValidator::default();
        let rm = RiskManager::with_defaults();
        let orders = [
            make_order(1, Side::Bid, 100, 10),
            make_order(1, Side::Ask, 110, 10), // 重複 ID
        ];
        let r = v.validate(&orders, &rm, 0);
        assert_eq!(r.accepted.len(), 1);
        assert_eq!(r.rejected.len(), 1);
        assert_eq!(r.rejected[0].1, BulkRejectReason::DuplicateId);
    }

    #[test]
    fn zero_quantity_rejected() {
        let v = BulkValidator::default();
        let rm = RiskManager::with_defaults();
        let orders = [make_order(1, Side::Bid, 100, 0)];
        let r = v.validate(&orders, &rm, 0);
        assert_eq!(r.rejected.len(), 1);
        assert_eq!(r.rejected[0].1, BulkRejectReason::ZeroQuantity);
    }

    #[test]
    fn self_trade_detected() {
        let v = BulkValidator::default();
        let rm = RiskManager::with_defaults();
        let orders = [
            make_order(1, Side::Bid, 105, 10),
            make_order(2, Side::Ask, 100, 10), // ask <= bid → self trade
        ];
        let r = v.validate(&orders, &rm, 0);
        assert_eq!(r.accepted.len(), 1);
        assert_eq!(r.rejected.len(), 1);
        assert_eq!(r.rejected[0].1, BulkRejectReason::SelfTrade);
    }

    #[test]
    fn self_trade_prevention_disabled() {
        let v = BulkValidator::new(false, 1000);
        let rm = RiskManager::with_defaults();
        let orders = [
            make_order(1, Side::Bid, 105, 10),
            make_order(2, Side::Ask, 100, 10),
        ];
        let r = v.validate(&orders, &rm, 0);
        assert_eq!(r.accepted.len(), 2); // 自己約定チェックなし
    }

    #[test]
    fn risk_check_rejection_in_batch() {
        let v = BulkValidator::default();
        let rm = RiskManager::new(RiskLimits {
            max_order_size: 50,
            ..RiskLimits::default()
        });
        let orders = [
            make_order(1, Side::Bid, 100, 30),
            make_order(2, Side::Bid, 100, 100), // 超過
        ];
        let r = v.validate(&orders, &rm, 0);
        assert_eq!(r.accepted.len(), 1);
        assert_eq!(r.rejected.len(), 1);
    }

    #[test]
    fn cumulative_position_tracking() {
        let v = BulkValidator::default();
        let rm = RiskManager::new(RiskLimits {
            max_position: 50,
            ..RiskLimits::default()
        });
        let orders = [
            make_order(1, Side::Bid, 100, 30),
            make_order(2, Side::Bid, 99, 30), // 30+30=60 > 50
        ];
        let r = v.validate(&orders, &rm, 0);
        assert_eq!(r.accepted.len(), 1);
        assert_eq!(r.rejected.len(), 1);
    }

    #[test]
    fn check_batch_size() {
        let v = BulkValidator::new(true, 5);
        assert!(v.check_batch_size(5));
        assert!(!v.check_batch_size(6));
    }

    #[test]
    fn default_validator() {
        let v = BulkValidator::default();
        assert!(v.self_trade_prevention);
        assert_eq!(v.max_batch_size, 1000);
    }

    #[test]
    fn reject_reason_eq() {
        assert_eq!(BulkRejectReason::DuplicateId, BulkRejectReason::DuplicateId);
        assert_ne!(
            BulkRejectReason::DuplicateId,
            BulkRejectReason::ZeroQuantity
        );
    }
}
