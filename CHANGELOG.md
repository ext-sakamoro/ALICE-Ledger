# Changelog

All notable changes to ALICE-Ledger will be documented in this file.

## [Unreleased]

### Added
- CI: `Test (public siblings)` job `cargo test --all-features` を走らせ、cargo の失敗、失敗した試験、通った試験が 168 件未満のどれでも失敗 `ci/**` の push でも走る `scripts/preflight.sh` は生成し直して同じ step を持つ

### Changed

- `alice-sign` の git rev を 8a1b00d に上げた (HMAC-SHA256 の追加のみ、使っている API と byte 列は同じ)
- Licence: `AGPL-3.0-only OR LicenseRef-Commercial` → `LicenseRef-Proprietary`
- Ed25519 の署名・検証と SHA-256 (`hash_data` / `Hash`) を `alice-sign` (MIT OR Apache-2.0、公開) から取るようにした 鍵・署名・hash の byte 列は同じ
- CI: `.github/preflight-exact` を置いた `scripts/preflight.sh` が生成器の出力 (見出しの `generator rev` の版) と byte で一致しなければ送る前の確認が通らない
- 設定 file の comment を中身の説明だけにし、.gitignore を project の build 出力だけにした
- TRADEMARK_NOTICE が指す licence の file を LICENSE-* の file にした

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
