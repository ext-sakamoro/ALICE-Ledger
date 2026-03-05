# Changelog

All notable changes to ALICE-Ledger will be documented in this file.

## [0.1.1] - 2026-03-04

### Added
- `ffi` — 20 `al_ldg_*` extern "C" functions (OrderBook 11, PositionTracker 5, Memory 3, Utility 1)
- `ffi` — 4 `#[repr(C)]` structs: `FfiOrder`, `FfiFill`, `FfiPosition`, `FfiDepthLevel`
- `ffi` — 10 FFI tests (lifecycle, null safety, roundtrip)
- `bindings/unity/AliceLedger.cs` — 20 DllImport + 3 RAII handles
- `bindings/ue5/AliceLedger.h` — 20 extern C + 3 RAII types
- `Cargo.toml` — `crate-type = ["rlib", "staticlib", "cdylib"]`
- `README.md`

### Changed
- Total tests: 39 → 53

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
