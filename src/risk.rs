/*
    ALICE-Ledger
    Copyright (C) 2026 Moroya Sakamoto
*/

//! Risk limits — notional exposure cap, max order size, position limits.
//!
//! 注文サブミッション前のリスクチェック。取引所レベルのプリトレードリスク制御。

use crate::order::{Order, Side};

/// リスクチェック結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskCheckResult {
    /// 通過。
    Passed,
    /// 拒否 (理由付き)。
    Rejected(RiskRejectReason),
}

/// リスク拒否理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskRejectReason {
    /// 最大注文サイズ超過。
    MaxOrderSizeExceeded {
        /// 注文数量。
        requested: u64,
        /// 上限。
        limit: u64,
    },
    /// 最大想定元本超過。
    MaxNotionalExceeded {
        /// 想定元本 (price × quantity)。
        notional: i128,
        /// 上限。
        limit: i128,
    },
    /// ポジション上限超過。
    PositionLimitExceeded {
        /// 現在ポジション + 注文後のネット数量。
        projected: i64,
        /// 上限 (絶対値)。
        limit: u64,
    },
    /// 注文レート上限超過。
    OrderRateLimitExceeded {
        /// 現在の期間内注文数。
        count: u64,
        /// 上限。
        limit: u64,
    },
    /// 最小注文サイズ未満。
    MinOrderSizeNotMet {
        /// 注文数量。
        requested: u64,
        /// 下限。
        minimum: u64,
    },
}

/// リスク制限設定。
#[derive(Debug, Clone)]
pub struct RiskLimits {
    /// 1注文あたりの最大数量。
    pub max_order_size: u64,
    /// 1注文あたりの最小数量。
    pub min_order_size: u64,
    /// 1注文あたりの最大想定元本 (price × quantity)。
    pub max_notional: i128,
    /// ネットポジション上限 (絶対値)。
    pub max_position: u64,
    /// 期間あたり最大注文数。
    pub max_orders_per_period: u64,
    /// 有効フラグ。
    pub enabled: bool,
}

impl Default for RiskLimits {
    fn default() -> Self {
        Self {
            max_order_size: 1_000_000,
            min_order_size: 1,
            max_notional: 1_000_000_000_000, // 1T ticks
            max_position: 10_000_000,
            max_orders_per_period: 10_000,
            enabled: true,
        }
    }
}

/// リスクマネージャ。
#[derive(Debug)]
pub struct RiskManager {
    /// リスク制限設定。
    pub limits: RiskLimits,
    /// 現在の期間内注文数。
    orders_in_period: u64,
}

impl RiskManager {
    /// 新しいリスクマネージャを作成。
    #[must_use]
    pub const fn new(limits: RiskLimits) -> Self {
        Self {
            limits,
            orders_in_period: 0,
        }
    }

    /// デフォルト設定のリスクマネージャを作成。
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(RiskLimits::default())
    }

    /// 注文のリスクチェック。
    #[must_use]
    pub fn check_order(&self, order: &Order, current_position: i64) -> RiskCheckResult {
        if !self.limits.enabled {
            return RiskCheckResult::Passed;
        }

        // 最小注文サイズ
        if order.quantity < self.limits.min_order_size {
            return RiskCheckResult::Rejected(RiskRejectReason::MinOrderSizeNotMet {
                requested: order.quantity,
                minimum: self.limits.min_order_size,
            });
        }

        // 最大注文サイズ
        if order.quantity > self.limits.max_order_size {
            return RiskCheckResult::Rejected(RiskRejectReason::MaxOrderSizeExceeded {
                requested: order.quantity,
                limit: self.limits.max_order_size,
            });
        }

        // 最大想定元本
        let notional = i128::from(order.price) * i128::from(order.quantity);
        if notional.abs() > self.limits.max_notional {
            return RiskCheckResult::Rejected(RiskRejectReason::MaxNotionalExceeded {
                notional,
                limit: self.limits.max_notional,
            });
        }

        // ポジション上限
        let delta = order.quantity as i64;
        let projected = match order.side {
            Side::Bid => current_position + delta,
            Side::Ask => current_position - delta,
        };
        if projected.unsigned_abs() > self.limits.max_position {
            return RiskCheckResult::Rejected(RiskRejectReason::PositionLimitExceeded {
                projected,
                limit: self.limits.max_position,
            });
        }

        // 注文レート制限
        if self.orders_in_period >= self.limits.max_orders_per_period {
            return RiskCheckResult::Rejected(RiskRejectReason::OrderRateLimitExceeded {
                count: self.orders_in_period,
                limit: self.limits.max_orders_per_period,
            });
        }

        RiskCheckResult::Passed
    }

    /// 注文カウントをインクリメント。
    pub const fn record_order(&mut self) {
        self.orders_in_period += 1;
    }

    /// 期間リセット。
    pub const fn reset_period(&mut self) {
        self.orders_in_period = 0;
    }

    /// 現在の期間内注文数。
    #[must_use]
    pub const fn orders_in_period(&self) -> u64 {
        self.orders_in_period
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{OrderId, OrderType, TimeInForce};

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
    fn default_limits() {
        let l = RiskLimits::default();
        assert_eq!(l.max_order_size, 1_000_000);
        assert!(l.enabled);
    }

    #[test]
    fn check_passes_normal_order() {
        let rm = RiskManager::with_defaults();
        let o = make_order(1, Side::Bid, 100, 10);
        assert_eq!(rm.check_order(&o, 0), RiskCheckResult::Passed);
    }

    #[test]
    fn check_rejects_oversized_order() {
        let rm = RiskManager::new(RiskLimits {
            max_order_size: 100,
            ..RiskLimits::default()
        });
        let o = make_order(1, Side::Bid, 100, 200);
        assert!(matches!(
            rm.check_order(&o, 0),
            RiskCheckResult::Rejected(RiskRejectReason::MaxOrderSizeExceeded { .. })
        ));
    }

    #[test]
    fn check_rejects_undersized_order() {
        let rm = RiskManager::new(RiskLimits {
            min_order_size: 10,
            ..RiskLimits::default()
        });
        let o = make_order(1, Side::Bid, 100, 5);
        assert!(matches!(
            rm.check_order(&o, 0),
            RiskCheckResult::Rejected(RiskRejectReason::MinOrderSizeNotMet { .. })
        ));
    }

    #[test]
    fn check_rejects_notional_exceeded() {
        let rm = RiskManager::new(RiskLimits {
            max_notional: 1000,
            ..RiskLimits::default()
        });
        let o = make_order(1, Side::Bid, 100, 20); // 2000 > 1000
        assert!(matches!(
            rm.check_order(&o, 0),
            RiskCheckResult::Rejected(RiskRejectReason::MaxNotionalExceeded { .. })
        ));
    }

    #[test]
    fn check_rejects_position_limit_bid() {
        let rm = RiskManager::new(RiskLimits {
            max_position: 50,
            ..RiskLimits::default()
        });
        let o = make_order(1, Side::Bid, 100, 30);
        // 既存ポジション 30 + 注文 30 = 60 > 50
        assert!(matches!(
            rm.check_order(&o, 30),
            RiskCheckResult::Rejected(RiskRejectReason::PositionLimitExceeded { .. })
        ));
    }

    #[test]
    fn check_rejects_position_limit_ask() {
        let rm = RiskManager::new(RiskLimits {
            max_position: 50,
            ..RiskLimits::default()
        });
        let o = make_order(1, Side::Ask, 100, 30);
        // 既存ポジション -30 - 30 = -60, |60| > 50
        assert!(matches!(
            rm.check_order(&o, -30),
            RiskCheckResult::Rejected(RiskRejectReason::PositionLimitExceeded { .. })
        ));
    }

    #[test]
    fn check_rejects_rate_limit() {
        let mut rm = RiskManager::new(RiskLimits {
            max_orders_per_period: 2,
            ..RiskLimits::default()
        });
        rm.record_order();
        rm.record_order();
        let o = make_order(1, Side::Bid, 100, 1);
        assert!(matches!(
            rm.check_order(&o, 0),
            RiskCheckResult::Rejected(RiskRejectReason::OrderRateLimitExceeded { .. })
        ));
    }

    #[test]
    fn reset_period_clears_count() {
        let mut rm = RiskManager::with_defaults();
        rm.record_order();
        rm.record_order();
        assert_eq!(rm.orders_in_period(), 2);
        rm.reset_period();
        assert_eq!(rm.orders_in_period(), 0);
    }

    #[test]
    fn disabled_always_passes() {
        let rm = RiskManager::new(RiskLimits {
            max_order_size: 1,
            enabled: false,
            ..RiskLimits::default()
        });
        let o = make_order(1, Side::Bid, 100, 9999);
        assert_eq!(rm.check_order(&o, 0), RiskCheckResult::Passed);
    }

    #[test]
    fn position_reducing_trade_passes() {
        let rm = RiskManager::new(RiskLimits {
            max_position: 50,
            ..RiskLimits::default()
        });
        // 既存ロング 40、売り注文 10 → projected = 30 < 50
        let o = make_order(1, Side::Ask, 100, 10);
        assert_eq!(rm.check_order(&o, 40), RiskCheckResult::Passed);
    }

    #[test]
    fn risk_check_result_eq() {
        assert_eq!(RiskCheckResult::Passed, RiskCheckResult::Passed);
        assert_ne!(
            RiskCheckResult::Passed,
            RiskCheckResult::Rejected(RiskRejectReason::MaxOrderSizeExceeded {
                requested: 1,
                limit: 0,
            })
        );
    }
}
