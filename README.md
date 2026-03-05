# ALICE-Ledger

Order book, matching engine, and position management for financial systems.

## Features

| Feature | Description |
|---------|-------------|
| **Order Book** | Price-time priority LOB (BTreeMap + VecDeque FIFO) |
| **Matching** | Market, Limit, StopLimit with GTC/IOC/FOK/GTD |
| **Position** | P&L accounting with mark-to-market |
| **Deterministic** | `i64` tick prices — no floating-point drift |
| **FFI** | C-ABI for Unity / UE5 / C consumers (`al_ldg_*`) |

## Quick Start

```rust
use alice_ledger::{
    book::OrderBook,
    order::{Order, OrderId, OrderType, Side, TimeInForce},
    position::PositionTracker,
};

let mut book = OrderBook::new();
let mut tracker = PositionTracker::new();
const SYM: u64 = 0x4254435553445400; // "BTCUSDT\0"

// Post resting ask at 50,000 ticks
book.insert(Order {
    id: OrderId(1),
    side: Side::Ask,
    order_type: OrderType::Limit,
    price: 50_000,
    quantity: 1,
    filled_quantity: 0,
    timestamp_ns: 0,
    time_in_force: TimeInForce::GTC,
});

// Aggress with market buy
let fills = book.insert(Order {
    id: OrderId(2),
    side: Side::Bid,
    order_type: OrderType::Market,
    price: 0,
    quantity: 1,
    filled_quantity: 0,
    timestamp_ns: 1,
    time_in_force: TimeInForce::IOC,
});

assert_eq!(fills.len(), 1);
tracker.apply_fill(SYM, &fills[0], Side::Bid);
tracker.mark_to_market(SYM, 51_000);
assert_eq!(tracker.get(SYM).unwrap().unrealized_pnl, 1_000);
```

## Build

```bash
cargo build --release
cargo test
```

### FFI (staticlib + cdylib)

```bash
cargo build --release --features ffi
```

## Test Suite

| Category | Tests |
|----------|-------|
| order | 4 |
| book | 18 |
| position | 13 |
| ffi | 10 |
| proptest | 8 |
| **Total** | **53** |

## FFI Functions (20)

Prefix: `al_ldg_*`

| Group | Count | Functions |
|-------|-------|-----------|
| Memory | 3 | fills_free, depth_free, orders_free |
| OrderBook | 11 | book_new, book_free, book_insert, book_insert_with_time, book_cancel, book_best_bid, book_best_ask, book_spread, book_depth, book_purge_expired, book_order_count |
| PositionTracker | 5 | tracker_new, tracker_free, tracker_apply_fill, tracker_mark_to_market, tracker_get |
| Utility | 1 | version |

## Bindings

- `bindings/unity/AliceLedger.cs` — 20 DllImport + 3 RAII handles
- `bindings/ue5/AliceLedger.h` — 20 extern C + 3 RAII types

## License

AGPL-3.0-only — Copyright (C) 2026 Moroya Sakamoto
