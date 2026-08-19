#pragma once
/**
 * gpu_host.h — GPU-side Soroban host function simulation.
 *
 * Implements a per-thread object heap and K/V ledger storage so that
 * WASM contracts can execute on the GPU without crossing the host boundary.
 *
 * Soroban Val encoding (soroban-env-common v22, 8-bit tag, 56-bit body):
 *   raw = (body << 8) | tag
 *   Object handle encoding: body = handle_index, tag = object Tag enum value
 *   Tags:  I128Object=69(0x45), StringObject=73(0x49), VecObject=75(0x4B),
 *          AddressObject=77(0x4D)
 *   Inline: Void=2, False=0, True=1
 *
 * Object heap: each thread owns GpuObjHeap (up to GPU_OBJ_CAP objects).
 * Ledger K/V: each thread owns GpuStorage (up to GPU_STORAGE_CAP entries).
 */

#include "hd.h"
#include "vmstate.h"

namespace cuwasm {

// ── Soroban Val tag constants (soroban-env-common v22) ────────────────────────
static constexpr uint8_t  SOROBAN_TAG_FALSE           =  0;
static constexpr uint8_t  SOROBAN_TAG_TRUE             =  1;
static constexpr uint8_t  SOROBAN_TAG_VOID             =  2;
static constexpr uint8_t  SOROBAN_TAG_ERROR            =  3;
static constexpr uint8_t  SOROBAN_TAG_U32VAL           =  4;
static constexpr uint8_t  SOROBAN_TAG_I32VAL           =  5;
static constexpr uint8_t  SOROBAN_TAG_U64SMALL         =  6;
static constexpr uint8_t  SOROBAN_TAG_I64SMALL         =  7;
static constexpr uint8_t  SOROBAN_TAG_I128SMALL        = 11;
static constexpr uint8_t  SOROBAN_TAG_SYMBOL_SMALL     = 14;
static constexpr uint8_t  SOROBAN_TAG_I128OBJECT       = 69;  // 0x45
static constexpr uint8_t  SOROBAN_TAG_STRING_OBJECT    = 73;  // 0x49
static constexpr uint8_t  SOROBAN_TAG_SYMBOL_OBJECT    = 74;  // 0x4A
static constexpr uint8_t  SOROBAN_TAG_VEC_OBJECT       = 75;  // 0x4B
static constexpr uint8_t  SOROBAN_TAG_MAP_OBJECT       = 76;  // 0x4C
static constexpr uint8_t  SOROBAN_TAG_ADDRESS_OBJECT   = 77;  // 0x4D

static constexpr uint64_t SOROBAN_TRUE   = 1ULL;
static constexpr uint64_t SOROBAN_FALSE  = 0ULL;
static constexpr uint64_t SOROBAN_VOID   = 2ULL;

// Construct a raw Val from a handle index and tag
HD static uint64_t soroban_make_obj(uint32_t handle, uint8_t tag) {
    return ((uint64_t)handle << 8) | (uint64_t)tag;
}

// Extract the tag byte from a raw Val
HD static uint8_t soroban_tag(uint64_t raw) {
    return (uint8_t)(raw & 0xFF);
}

// Extract the handle index from an object Val
HD static uint32_t soroban_handle(uint64_t raw) {
    return (uint32_t)(raw >> 8);
}

// ── fn_id constants (enumeration order in docs/soroban-env.json) ──────────────
static constexpr uint32_t FN_CONTRACT_EVENT     =   2;
static constexpr uint32_t FN_GET_LEDGER_SEQ     =   4;
static constexpr uint32_t FN_GET_CUR_ADDR       =   8;
static constexpr uint32_t FN_OBJ_FROM_I128      =  17;
static constexpr uint32_t FN_OBJ_TO_I128_LO     =  18;
static constexpr uint32_t FN_OBJ_TO_I128_HI     =  19;
static constexpr uint32_t FN_MAP_NEW_LM         =  72;
static constexpr uint32_t FN_MAP_UNPACK_LM      =  73;
static constexpr uint32_t FN_VEC_NEW_LM         =  93;
static constexpr uint32_t FN_PUT_CONTRACT_DATA  =  95;
static constexpr uint32_t FN_HAS_CONTRACT_DATA  =  96;
static constexpr uint32_t FN_GET_CONTRACT_DATA  =  97;
static constexpr uint32_t FN_EXTEND_DATA_TTL    = 103;
static constexpr uint32_t FN_EXTEND_INST_TTL    = 104;
static constexpr uint32_t FN_STR_NEW_LM         = 137;
static constexpr uint32_t FN_SYM_NEW_LM         = 138;
static constexpr uint32_t FN_REQUIRE_AUTH       = 182;

// ── Object heap ───────────────────────────────────────────────────────────────
static constexpr uint32_t GPU_OBJ_CAP    = 32;  // max live objects per thread
static constexpr uint32_t GPU_OBJ_BYTES  = 16;  // bytes of inline object data

struct GpuObjEntry {
    uint8_t  tag;        // SOROBAN_TAG_* value
    uint8_t  _pad[3];
    uint32_t len;        // for strings/vecs: element count or byte length
    uint8_t  data[GPU_OBJ_BYTES]; // inline data (e.g., i128 hi/lo)
};

struct GpuObjHeap {
    GpuObjEntry entries[GPU_OBJ_CAP];
    uint32_t    count;  // number of allocated objects
};

// Allocate a new object in the heap; returns the raw Val or 0 on OOM.
HD static uint64_t obj_alloc(GpuObjHeap& heap, uint8_t tag,
                              uint32_t len = 0,
                              const uint8_t* data = nullptr) {
    if (heap.count >= GPU_OBJ_CAP) return SOROBAN_VOID;
    uint32_t h = heap.count++;
    auto& e = heap.entries[h];
    e.tag = tag; e.len = len;
    if (data) {
        uint32_t n = len < GPU_OBJ_BYTES ? len : GPU_OBJ_BYTES;
        for (uint32_t i = 0; i < n; ++i) e.data[i] = data[i];
    }
    return soroban_make_obj(h, tag);
}

// Retrieve an entry by raw Val (returns nullptr if invalid)
HD static GpuObjEntry* obj_get(GpuObjHeap& heap, uint64_t raw) {
    uint32_t h = soroban_handle(raw);
    if (h >= heap.count) return nullptr;
    return &heap.entries[h];
}

// ── Ledger K/V storage ────────────────────────────────────────────────────────
static constexpr uint32_t GPU_STORAGE_CAP = 16;

struct GpuStorageEntry {
    uint64_t key;       // raw Val (opaque 64-bit key)
    uint64_t val;       // raw Val stored
    uint8_t  occupied;
    uint8_t  _pad[7];
};

struct GpuStorage {
    GpuStorageEntry entries[GPU_STORAGE_CAP];
    uint32_t count;
};

// ── Combined per-thread GPU host state ────────────────────────────────────────
struct GpuHostState {
    GpuObjHeap  obj_heap;
    GpuStorage  storage;
};

// ── Main dispatch function ────────────────────────────────────────────────────
/**
 * Handle one Soroban host call on-GPU.
 * Returns true if handled; false if unsupported (caller should set ST_UNSUPPORTED_OP).
 * On return, mb.n_results and mb.results[] contain the return value(s).
 *
 * Memory pointer 'mem' and 'mem_size' allow reading WASM linear memory
 * for string/vec constructors that serialize data there.
 */
HD bool gpu_host_dispatch(GpuHostState& state,
                           HostMailbox& mb,
                           const uint8_t* mem, uint32_t mem_size) {
    GpuObjHeap& heap    = state.obj_heap;
    GpuStorage& storage = state.storage;

    switch (mb.fn_id) {

    // ── String/Symbol from linear memory ─────────────────────────────────
    // U32Val encoding: raw = (u32_value << 32) | tag,  so value = raw >> 32
    case FN_STR_NEW_LM: {
        uint32_t ptr = (uint32_t)(mb.args[0] >> 32);
        uint32_t len = (uint32_t)(mb.args[1] >> 32);
        const uint8_t* src = (mem && (uint64_t)ptr + len <= mem_size) ? mem + ptr : nullptr;
        mb.n_results = 1;
        mb.results[0] = obj_alloc(heap, SOROBAN_TAG_STRING_OBJECT, len, src);
        return true;
    }
    case FN_SYM_NEW_LM: {
        uint32_t ptr = (uint32_t)(mb.args[0] >> 32);
        uint32_t len = (uint32_t)(mb.args[1] >> 32);
        const uint8_t* src = (mem && (uint64_t)ptr + len <= mem_size) ? mem + ptr : nullptr;
        mb.n_results = 1;
        mb.results[0] = obj_alloc(heap, SOROBAN_TAG_SYMBOL_OBJECT, len, src);
        return true;
    }

    // ── Vec from linear memory ────────────────────────────────────────────
    case FN_VEC_NEW_LM: {
        uint32_t ptr = (uint32_t)(mb.args[0] >> 32);
        uint32_t n   = (uint32_t)(mb.args[1] >> 32);
        // Store first 2 element payloads inline (enough for most uses)
        uint8_t inline_data[GPU_OBJ_BYTES] = {};
        if (mem && ptr + n * 8 <= mem_size) {
            uint32_t bytes = (n * 8 < GPU_OBJ_BYTES) ? n * 8 : GPU_OBJ_BYTES;
            for (uint32_t i = 0; i < bytes; ++i)
                inline_data[i] = mem[ptr + i];
        }
        mb.n_results = 1;
        mb.results[0] = obj_alloc(heap, SOROBAN_TAG_VEC_OBJECT, n, inline_data);
        return true;
    }

    // ── Map from linear memory ────────────────────────────────────────────
    case FN_MAP_NEW_LM:
        mb.n_results = 1;
        mb.results[0] = obj_alloc(heap, SOROBAN_TAG_MAP_OBJECT, 0, nullptr);
        return true;

    case FN_MAP_UNPACK_LM:
        mb.n_results = 1;
        mb.results[0] = SOROBAN_VOID;
        return true;

    // ── i128 helpers ──────────────────────────────────────────────────────
    case FN_OBJ_FROM_I128: {
        // args: (i64 hi, u64 lo)
        uint8_t data[16];
        uint64_t lo = mb.args[1], hi = (uint64_t)(int64_t)mb.args[0];
        for (int i = 0; i < 8; ++i) data[i]     = (uint8_t)(lo >> (8*i));
        for (int i = 0; i < 8; ++i) data[8+i]   = (uint8_t)(hi >> (8*i));
        mb.n_results = 1;
        mb.results[0] = obj_alloc(heap, SOROBAN_TAG_I128OBJECT, 16, data);
        return true;
    }
    case FN_OBJ_TO_I128_LO: {
        auto* e = obj_get(heap, mb.args[0]);
        uint64_t lo = 0;
        if (e && e->tag == SOROBAN_TAG_I128OBJECT) {
            for (int i = 0; i < 8; ++i) lo |= ((uint64_t)e->data[i]) << (8*i);
        }
        mb.n_results = 1;
        mb.results[0] = lo;
        return true;
    }
    case FN_OBJ_TO_I128_HI: {
        auto* e = obj_get(heap, mb.args[0]);
        int64_t hi = 0;
        if (e && e->tag == SOROBAN_TAG_I128OBJECT) {
            uint64_t hi_u = 0;
            for (int i = 0; i < 8; ++i) hi_u |= ((uint64_t)e->data[8+i]) << (8*i);
            hi = (int64_t)hi_u;
        }
        mb.n_results = 1;
        mb.results[0] = (uint64_t)hi;
        return true;
    }

    // ── Ledger storage ────────────────────────────────────────────────────
    case FN_HAS_CONTRACT_DATA: {
        uint64_t key = mb.args[0];
        for (uint32_t i = 0; i < storage.count; ++i)
            if (storage.entries[i].occupied && storage.entries[i].key == key) {
                mb.n_results = 1; mb.results[0] = SOROBAN_TRUE; return true;
            }
        mb.n_results = 1; mb.results[0] = SOROBAN_FALSE; return true;
    }
    case FN_GET_CONTRACT_DATA: {
        uint64_t key = mb.args[0];
        for (uint32_t i = 0; i < storage.count; ++i)
            if (storage.entries[i].occupied && storage.entries[i].key == key) {
                mb.n_results = 1; mb.results[0] = storage.entries[i].val; return true;
            }
        mb.n_results = 1; mb.results[0] = SOROBAN_VOID; return true;
    }
    case FN_PUT_CONTRACT_DATA: {
        uint64_t key = mb.args[0], val = mb.args[1];
        for (uint32_t i = 0; i < storage.count; ++i)
            if (storage.entries[i].occupied && storage.entries[i].key == key) {
                storage.entries[i].val = val;
                mb.n_results = 1; mb.results[0] = SOROBAN_VOID; return true;
            }
        if (storage.count < GPU_STORAGE_CAP) {
            auto& e = storage.entries[storage.count++];
            e.key = key; e.val = val; e.occupied = 1;
        }
        mb.n_results = 1; mb.results[0] = SOROBAN_VOID; return true;
    }

    // ── TTL extensions: no-op ─────────────────────────────────────────────
    case FN_EXTEND_DATA_TTL:
    case FN_EXTEND_INST_TTL:
        mb.n_results = 1; mb.results[0] = SOROBAN_VOID; return true;

    // ── Auth / events ─────────────────────────────────────────────────────
    case FN_REQUIRE_AUTH:
        mb.n_results = 1; mb.results[0] = SOROBAN_VOID; return true;
    case FN_CONTRACT_EVENT:
        mb.n_results = 1; mb.results[0] = SOROBAN_VOID; return true;

    // ── Ledger/context queries ────────────────────────────────────────────
    case FN_GET_LEDGER_SEQ:
        // Return a U32Val for ledger sequence 100
        mb.n_results = 1;
        mb.results[0] = ((uint64_t)100 << 8) | SOROBAN_TAG_U32VAL;
        return true;
    case FN_GET_CUR_ADDR:
        mb.n_results = 1;
        mb.results[0] = obj_alloc(heap, SOROBAN_TAG_ADDRESS_OBJECT, 0, nullptr);
        return true;

    default:
        return false;
    }
}

} // namespace cuwasm
