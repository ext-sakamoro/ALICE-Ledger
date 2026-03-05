/*
    ALICE-Ledger — UE5 C++ Bindings
    Copyright (C) 2026 Moroya Sakamoto

    20 extern C + 3 RAII types
    Prefix: al_ldg_*
*/

#pragma once

#include <cstdint>
#include <memory>

// ============================================================================
// Repr(C) structs
// ============================================================================

struct AlLdgOrder {
    uint64_t id;
    uint8_t  side;        // 0 = Bid, 1 = Ask
    uint8_t  order_type;  // 0 = Limit, 1 = Market, 2 = StopLimit
    int64_t  stop_price;  // only when order_type == 2
    int64_t  price;
    uint64_t quantity;
    uint64_t filled_quantity;
    uint64_t timestamp_ns;
    uint8_t  tif;         // 0 = GTC, 1 = IOC, 2 = FOK, 3 = GTD
    uint64_t expiry_ns;   // only when tif == 3
};

struct AlLdgFill {
    uint64_t maker_id;
    uint64_t taker_id;
    int64_t  price;
    uint64_t quantity;
    uint64_t timestamp_ns;
};

struct AlLdgPosition {
    uint64_t symbol_hash;
    int64_t  net_quantity;
    int64_t  avg_entry_price;
    int64_t  realized_pnl;
    int64_t  unrealized_pnl;
    uint64_t trade_count;
};

struct AlLdgDepthLevel {
    int64_t  price;
    uint64_t quantity;
};

// ============================================================================
// Opaque forward declarations
// ============================================================================

struct AlLdgOrderBook;
struct AlLdgPositionTracker;

// ============================================================================
// extern "C" (20 functions)
// ============================================================================

extern "C" {

// --- Memory Management (3) ---
void al_ldg_fills_free(AlLdgFill* ptr, int32_t count);
void al_ldg_depth_free(AlLdgDepthLevel* ptr, int32_t count);
void al_ldg_orders_free(AlLdgOrder* ptr, int32_t count);

// --- OrderBook (11) ---
AlLdgOrderBook* al_ldg_book_new();
void             al_ldg_book_free(AlLdgOrderBook* book);

AlLdgFill* al_ldg_book_insert(
    AlLdgOrderBook* book, const AlLdgOrder* order, int32_t* out_count);

AlLdgFill* al_ldg_book_insert_with_time(
    AlLdgOrderBook* book, const AlLdgOrder* order, uint64_t now_ns, int32_t* out_count);

int32_t al_ldg_book_cancel(
    AlLdgOrderBook* book, uint64_t id, AlLdgOrder* out);

int64_t al_ldg_book_best_bid(const AlLdgOrderBook* book);
int64_t al_ldg_book_best_ask(const AlLdgOrderBook* book);
int64_t al_ldg_book_spread(const AlLdgOrderBook* book);

AlLdgDepthLevel* al_ldg_book_depth(
    const AlLdgOrderBook* book, uint8_t side, int32_t levels, int32_t* out_count);

AlLdgOrder* al_ldg_book_purge_expired(
    AlLdgOrderBook* book, uint64_t now_ns, int32_t* out_count);

int32_t al_ldg_book_order_count(const AlLdgOrderBook* book);

// --- PositionTracker (5) ---
AlLdgPositionTracker* al_ldg_tracker_new();
void                   al_ldg_tracker_free(AlLdgPositionTracker* tracker);

void al_ldg_tracker_apply_fill(
    AlLdgPositionTracker* tracker, uint64_t symbol_hash,
    const AlLdgFill* fill, uint8_t side);

void al_ldg_tracker_mark_to_market(
    AlLdgPositionTracker* tracker, uint64_t symbol_hash, int64_t current_price);

int32_t al_ldg_tracker_get(
    const AlLdgPositionTracker* tracker, uint64_t symbol_hash, AlLdgPosition* out);

// --- Utility (1) ---
const char* al_ldg_version();

} // extern "C"

// ============================================================================
// RAII wrappers
// ============================================================================

struct AlLdgBookDeleter {
    void operator()(AlLdgOrderBook* p) const { if (p) al_ldg_book_free(p); }
};
using AlLdgBookPtr = std::unique_ptr<AlLdgOrderBook, AlLdgBookDeleter>;

struct AlLdgTrackerDeleter {
    void operator()(AlLdgPositionTracker* p) const { if (p) al_ldg_tracker_free(p); }
};
using AlLdgTrackerPtr = std::unique_ptr<AlLdgPositionTracker, AlLdgTrackerDeleter>;

struct AlLdgFillArrayDeleter {
    int32_t count = 0;
    void operator()(AlLdgFill* p) const { if (p) al_ldg_fills_free(p, count); }
};
using AlLdgFillArrayPtr = std::unique_ptr<AlLdgFill, AlLdgFillArrayDeleter>;
