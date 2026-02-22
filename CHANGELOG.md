# Changelog

All notable changes to ALICE-Ledger will be documented in this file.

## [0.1.0] - 2026-02-23

### Added
- `order` — `Order`, `OrderId`, `Side`, `OrderType` (Market / Limit / StopLimit), `TimeInForce` (GTC / IOC / FOK / GTD)
- `book` — `OrderBook` with price-time priority matching (BTreeMap + VecDeque FIFO)
- `book` — `Fill` records, bid/ask level management, cancel, amend, `purge_expired`
- `book` — `insert_with_time` for GTD expiry check
- `position` — `Position` and `PositionTracker` for P&L accounting with mark-to-market
- Prices as `i64` ticks — deterministic fixed-point arithmetic (no floating-point)
- Integration with ALICE-Sync `Fixed` (Q16.16) via telemetry feature
- 39 tests (38 unit + 1 doc-test)
