/*
    ALICE-Ledger — Unity C# Bindings
    Copyright (C) 2026 Moroya Sakamoto

    20 DllImport + 3 RAII handles
    Prefix: al_ldg_*
*/

using System;
using System.Runtime.InteropServices;

namespace Alice.Ledger
{
    // ========================================================================
    // Repr(C) structs
    // ========================================================================

    [StructLayout(LayoutKind.Sequential)]
    public struct FfiOrder
    {
        public ulong id;
        /// <summary>0 = Bid, 1 = Ask</summary>
        public byte side;
        /// <summary>0 = Limit, 1 = Market, 2 = StopLimit</summary>
        public byte orderType;
        /// <summary>Only used when orderType == 2</summary>
        public long stopPrice;
        public long price;
        public ulong quantity;
        public ulong filledQuantity;
        public ulong timestampNs;
        /// <summary>0 = GTC, 1 = IOC, 2 = FOK, 3 = GTD</summary>
        public byte tif;
        /// <summary>Only used when tif == 3</summary>
        public ulong expiryNs;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct FfiFill
    {
        public ulong makerId;
        public ulong takerId;
        public long price;
        public ulong quantity;
        public ulong timestampNs;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct FfiPosition
    {
        public ulong symbolHash;
        public long netQuantity;
        public long avgEntryPrice;
        public long realizedPnl;
        public long unrealizedPnl;
        public ulong tradeCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct FfiDepthLevel
    {
        public long price;
        public ulong quantity;
    }

    // ========================================================================
    // RAII handles
    // ========================================================================

    public class BookHandle : IDisposable
    {
        public IntPtr Ptr { get; private set; }
        public bool IsValid => Ptr != IntPtr.Zero;

        public BookHandle() { Ptr = Native.al_ldg_book_new(); }
        internal BookHandle(IntPtr ptr) { Ptr = ptr; }

        public void Dispose()
        {
            if (Ptr != IntPtr.Zero)
            {
                Native.al_ldg_book_free(Ptr);
                Ptr = IntPtr.Zero;
            }
        }
    }

    public class TrackerHandle : IDisposable
    {
        public IntPtr Ptr { get; private set; }
        public bool IsValid => Ptr != IntPtr.Zero;

        public TrackerHandle() { Ptr = Native.al_ldg_tracker_new(); }
        internal TrackerHandle(IntPtr ptr) { Ptr = ptr; }

        public void Dispose()
        {
            if (Ptr != IntPtr.Zero)
            {
                Native.al_ldg_tracker_free(Ptr);
                Ptr = IntPtr.Zero;
            }
        }
    }

    public class FillArrayHandle : IDisposable
    {
        public IntPtr Ptr { get; private set; }
        public int Count { get; private set; }

        internal FillArrayHandle(IntPtr ptr, int count)
        {
            Ptr = ptr;
            Count = count;
        }

        public FfiFill Get(int index)
        {
            if (index < 0 || index >= Count)
                throw new IndexOutOfRangeException();
            IntPtr elem = IntPtr.Add(Ptr, index * Marshal.SizeOf<FfiFill>());
            return Marshal.PtrToStructure<FfiFill>(elem);
        }

        public void Dispose()
        {
            if (Ptr != IntPtr.Zero)
            {
                Native.al_ldg_fills_free(Ptr, Count);
                Ptr = IntPtr.Zero;
                Count = 0;
            }
        }
    }

    // ========================================================================
    // Native (20 DllImport)
    // ========================================================================

    public static class Native
    {
        const string Lib = "alice_ledger";

        // --- Memory Management (3) ---

        [DllImport(Lib)] public static extern void al_ldg_fills_free(IntPtr ptr, int count);
        [DllImport(Lib)] public static extern void al_ldg_depth_free(IntPtr ptr, int count);
        [DllImport(Lib)] public static extern void al_ldg_orders_free(IntPtr ptr, int count);

        // --- OrderBook (11) ---

        [DllImport(Lib)] public static extern IntPtr al_ldg_book_new();
        [DllImport(Lib)] public static extern void al_ldg_book_free(IntPtr book);

        [DllImport(Lib)]
        public static extern IntPtr al_ldg_book_insert(
            IntPtr book, ref FfiOrder order, out int outCount);

        [DllImport(Lib)]
        public static extern IntPtr al_ldg_book_insert_with_time(
            IntPtr book, ref FfiOrder order, ulong nowNs, out int outCount);

        [DllImport(Lib)]
        public static extern int al_ldg_book_cancel(
            IntPtr book, ulong id, out FfiOrder outOrder);

        [DllImport(Lib)] public static extern long al_ldg_book_best_bid(IntPtr book);
        [DllImport(Lib)] public static extern long al_ldg_book_best_ask(IntPtr book);
        [DllImport(Lib)] public static extern long al_ldg_book_spread(IntPtr book);

        [DllImport(Lib)]
        public static extern IntPtr al_ldg_book_depth(
            IntPtr book, byte side, int levels, out int outCount);

        [DllImport(Lib)]
        public static extern IntPtr al_ldg_book_purge_expired(
            IntPtr book, ulong nowNs, out int outCount);

        [DllImport(Lib)] public static extern int al_ldg_book_order_count(IntPtr book);

        // --- PositionTracker (5) ---

        [DllImport(Lib)] public static extern IntPtr al_ldg_tracker_new();
        [DllImport(Lib)] public static extern void al_ldg_tracker_free(IntPtr tracker);

        [DllImport(Lib)]
        public static extern void al_ldg_tracker_apply_fill(
            IntPtr tracker, ulong symbolHash, ref FfiFill fill, byte side);

        [DllImport(Lib)]
        public static extern void al_ldg_tracker_mark_to_market(
            IntPtr tracker, ulong symbolHash, long currentPrice);

        [DllImport(Lib)]
        public static extern int al_ldg_tracker_get(
            IntPtr tracker, ulong symbolHash, out FfiPosition outPos);

        // --- Utility (1) ---

        [DllImport(Lib)] public static extern IntPtr al_ldg_version();

        // --- Helpers ---

        public static string Version()
        {
            IntPtr ptr = al_ldg_version();
            return ptr != IntPtr.Zero ? Marshal.PtrToStringAnsi(ptr) : "unknown";
        }
    }
}
