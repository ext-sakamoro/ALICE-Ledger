# Contributing to ALICE-Ledger

## Build

```bash
cargo build
```

## Test

```bash
cargo test
```

## Lint

```bash
cargo clippy -- -W clippy::all
cargo fmt -- --check
cargo doc --no-deps 2>&1 | grep warning
```

## Design Constraints

- **Integer prices**: all prices are `i64` ticks — no floating-point. Matches ALICE-Sync `Fixed` Q16.16.
- **Price-time priority**: bids sorted descending, asks ascending; FIFO within each level.
- **Deterministic matching**: identical inputs produce identical fills on all platforms.
- **No external dependencies**: only ALICE-Sync (for telemetry) and `std` collections.
- **Fixed fill records**: `Fill` is a flat struct with maker/taker IDs, price, quantity, timestamp.
