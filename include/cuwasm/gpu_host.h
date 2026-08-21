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
 * Ledger K/V: two parallel arrays (`keys[]`, `values[]`). Lookup is a linear
 * scan with byte-by-byte compare — guest handles are never used as key identity.
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

// Construct a raw Val from a handle index and tag.
// soroban-env-common v22: Object = from_major_minor_and_tag(handle, 0, tag)
//   = (handle << 32) | tag
HD static uint64_t soroban_make_obj(uint32_t handle, uint8_t tag) {
    return ((uint64_t)handle << 32) | (uint64_t)tag;
}

// Extract the tag byte from a raw Val
HD static uint8_t soroban_tag(uint64_t raw) {
    return (uint8_t)(raw & 0xFF);
}

// Extract the object handle (major) from an object Val
HD static uint32_t soroban_handle(uint64_t raw) {
    return (uint32_t)(raw >> 32);
}

HD static uint64_t soroban_i128_small(uint64_t n) {
    return (n << 8) | (uint64_t)SOROBAN_TAG_I128SMALL;
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
static constexpr uint32_t GPU_OBJ_CAP     = 32;
static constexpr uint32_t GPU_OBJ_BYTES   = 32;  // AddressObject pubkey is 32 B
static constexpr uint32_t GPU_VEC_ELEMS   = 4;

struct GpuObjEntry {
    uint8_t  tag;
    uint8_t  _pad[3];
    uint32_t len;                    // string/address byte length, or vec arity
    uint8_t  data[GPU_OBJ_BYTES];    // pubkey / i128 / string prefix
    uint64_t elems[GPU_VEC_ELEMS];   // raw Vals for VecObject
};

struct GpuObjHeap {
    GpuObjEntry entries[GPU_OBJ_CAP];
    uint32_t    count;
};

HD static uint64_t obj_alloc(GpuObjHeap& heap, uint8_t tag,
                              uint32_t len = 0,
                              const uint8_t* data = nullptr) {
    if (heap.count >= GPU_OBJ_CAP) return SOROBAN_VOID;
    uint32_t h = heap.count++;
    auto& e = heap.entries[h];
    e.tag = tag; e.len = len;
    for (uint32_t i = 0; i < GPU_OBJ_BYTES; ++i) e.data[i] = 0;
    for (uint32_t i = 0; i < GPU_VEC_ELEMS; ++i) e.elems[i] = 0;
    if (data) {
        uint32_t n = len < GPU_OBJ_BYTES ? len : GPU_OBJ_BYTES;
        for (uint32_t i = 0; i < n; ++i) e.data[i] = data[i];
    }
    return soroban_make_obj(h, tag);
}

HD static GpuObjEntry* obj_get(GpuObjHeap& heap, uint64_t raw) {
    uint32_t h = soroban_handle(raw);
    if (h >= heap.count) return nullptr;
    return &heap.entries[h];
}

// ── Ledger K/V: two parallel arrays, linear scan, byte-wise compare ───────────
//
// Keys are canonical byte blobs (not guest handles). Guest VecObject handles
// change every call; the ScVal XDR / resolved contents do not.
//   byte[0]     = StorageType (0=temp, 1=persistent, 2=instance)
//   remaining   = resolved key payload:
//     SymbolSmall / other small Val : 8-byte little-endian payload
//     VecObject                     : concat of each element:
//         AddressObject → 32-byte pubkey from the object heap
//         other         → 8-byte Val
// Values are raw Val payloads (8 bytes) for the token/increment workloads.
static constexpr uint32_t GPU_STORAGE_CAP = 16;
static constexpr uint32_t GPU_KV_KEY_MAX  = 128;
static constexpr uint32_t GPU_KV_VAL_MAX  = 32;

struct GpuStorage {
    uint8_t  keys[GPU_STORAGE_CAP][GPU_KV_KEY_MAX];
    uint32_t key_lens[GPU_STORAGE_CAP];
    uint8_t  values[GPU_STORAGE_CAP][GPU_KV_VAL_MAX];
    uint32_t val_lens[GPU_STORAGE_CAP];
    uint32_t count;
};

HD static bool kv_bytes_eq(const uint8_t* a, const uint8_t* b, uint32_t n) {
    for (uint32_t i = 0; i < n; ++i)
        if (a[i] != b[i]) return false;
    return true;
}

HD static int kv_find(const GpuStorage& store, const uint8_t* key, uint32_t klen) {
    for (uint32_t i = 0; i < store.count; ++i) {
        if (store.key_lens[i] != klen) continue;
        if (kv_bytes_eq(store.keys[i], key, klen)) return (int)i;
    }
    return -1;
}

HD static void kv_write_u64(uint8_t* dst, uint64_t v) {
    for (int i = 0; i < 8; ++i) dst[i] = (uint8_t)(v >> (8 * i));
}

HD static uint64_t kv_read_u64(const uint8_t* src) {
    uint64_t v = 0;
    for (int i = 0; i < 8; ++i) v |= ((uint64_t)src[i]) << (8 * i);
    return v;
}

// Build a canonical key blob from a guest Val + storage type.
HD static bool kv_canon_key(GpuObjHeap& heap, uint64_t key_val, uint8_t stype,
                             uint8_t* out, uint32_t* out_len) {
    if (GPU_KV_KEY_MAX < 9) return false;
    uint32_t n = 0;
    out[n++] = stype;
    uint8_t tag = soroban_tag(key_val);
    if (tag == SOROBAN_TAG_VEC_OBJECT) {
        GpuObjEntry* e = obj_get(heap, key_val);
        if (!e) return false;
        uint32_t ne = e->len < GPU_VEC_ELEMS ? e->len : GPU_VEC_ELEMS;
        for (uint32_t i = 0; i < ne; ++i) {
            uint64_t ev = e->elems[i];
            if (soroban_tag(ev) == SOROBAN_TAG_ADDRESS_OBJECT) {
                GpuObjEntry* a = obj_get(heap, ev);
                if (n + 32 > GPU_KV_KEY_MAX) return false;
                if (a) {
                    for (uint32_t b = 0; b < 32; ++b) out[n++] = a->data[b];
                } else {
                    for (uint32_t b = 0; b < 32; ++b) out[n++] = 0;
                }
            } else {
                if (n + 8 > GPU_KV_KEY_MAX) return false;
                kv_write_u64(out + n, ev);
                n += 8;
            }
        }
    } else if (tag == SOROBAN_TAG_ADDRESS_OBJECT) {
        GpuObjEntry* a = obj_get(heap, key_val);
        if (n + 32 > GPU_KV_KEY_MAX) return false;
        if (a) {
            for (uint32_t b = 0; b < 32; ++b) out[n++] = a->data[b];
        } else {
            for (uint32_t b = 0; b < 32; ++b) out[n++] = 0;
        }
    } else {
        if (n + 8 > GPU_KV_KEY_MAX) return false;
        kv_write_u64(out + n, key_val);
        n += 8;
    }
    *out_len = n;
    return true;
}

// Token WASM symbol-small constants (from the compiled contract, stable).
static constexpr uint64_t SYM_ADMIN   = 0xca72bb30eULL;
static constexpr uint64_t SYM_BALANCE = 0xd9b19b3a2a0eULL;
static constexpr uint8_t  STOR_TEMPORARY  = 0;
static constexpr uint8_t  STOR_PERSISTENT = 1;
static constexpr uint8_t  STOR_INSTANCE   = 2;

HD static void gpu_kv_put_raw(GpuStorage& store,
                               const uint8_t* key, uint32_t klen,
                               const uint8_t* val, uint32_t vlen) {
    int idx = kv_find(store, key, klen);
    if (idx < 0) {
        if (store.count >= GPU_STORAGE_CAP) return;
        idx = (int)store.count++;
        for (uint32_t b = 0; b < klen && b < GPU_KV_KEY_MAX; ++b)
            store.keys[idx][b] = key[b];
        store.key_lens[idx] = klen;
    }
    uint32_t n = vlen < GPU_KV_VAL_MAX ? vlen : GPU_KV_VAL_MAX;
    for (uint32_t b = 0; b < n; ++b)
        store.values[idx][b] = val[b];
    store.val_lens[idx] = n;
}

HD static void gpu_kv_put_u64(GpuStorage& store,
                               const uint8_t* key, uint32_t klen, uint64_t val) {
    uint8_t buf[8];
    kv_write_u64(buf, val);
    gpu_kv_put_raw(store, key, klen, buf, 8);
}

// Seed Vec([Admin]) instance → 32-byte admin pubkey.
HD static void gpu_seed_admin(GpuStorage& store, const uint8_t admin_pk[32]) {
    uint8_t key[9];
    key[0] = STOR_INSTANCE;
    kv_write_u64(key + 1, SYM_ADMIN);
    gpu_kv_put_raw(store, key, 9, admin_pk, 32);
}

// Seed Vec([Balance, addr]) persistent → I128Small(amount).
HD static void gpu_seed_balance(GpuStorage& store, const uint8_t addr_pk[32], uint64_t amount) {
    uint8_t key[1 + 8 + 32];
    key[0] = STOR_PERSISTENT;
    kv_write_u64(key + 1, SYM_BALANCE);
    for (uint32_t b = 0; b < 32; ++b) key[9 + b] = addr_pk[b];
    gpu_kv_put_u64(store, key, 41, soroban_i128_small(amount));
}

HD static void gpu_heap_reset(GpuObjHeap& heap) {
    heap.count = 0;
    for (uint32_t i = 0; i < GPU_OBJ_CAP; ++i) {
        auto& e = heap.entries[i];
        e.tag = 0; e.len = 0;
        for (uint32_t b = 0; b < GPU_OBJ_BYTES; ++b) e.data[b] = 0;
        for (uint32_t k = 0; k < GPU_VEC_ELEMS; ++k) e.elems[k] = 0;
    }
}

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
        uint64_t raw = obj_alloc(heap, SOROBAN_TAG_VEC_OBJECT, n, nullptr);
        GpuObjEntry* e = obj_get(heap, raw);
        if (e && mem) {
            uint32_t ne = n < GPU_VEC_ELEMS ? n : GPU_VEC_ELEMS;
            e->len = n;
            for (uint32_t i = 0; i < ne; ++i) {
                uint32_t off = ptr + i * 8;
                if ((uint64_t)off + 8 <= mem_size) {
                    uint64_t v = 0;
                    for (int b = 0; b < 8; ++b)
                        v |= ((uint64_t)mem[off + b]) << (8 * b);
                    e->elems[i] = v;
                }
            }
        }
        mb.n_results = 1;
        mb.results[0] = raw;
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

    // ── Ledger storage (byte-wise key compare over two arrays) ────────────
    case FN_HAS_CONTRACT_DATA: {
        uint8_t keybuf[GPU_KV_KEY_MAX];
        uint32_t klen = 0;
        uint8_t stype = (uint8_t)mb.args[1];
        mb.n_results = 1;
        if (!kv_canon_key(heap, mb.args[0], stype, keybuf, &klen)) {
            mb.results[0] = SOROBAN_FALSE;
            return true;
        }
        mb.results[0] = (kv_find(storage, keybuf, klen) >= 0)
                            ? SOROBAN_TRUE : SOROBAN_FALSE;
        return true;
    }
    case FN_GET_CONTRACT_DATA: {
        uint8_t keybuf[GPU_KV_KEY_MAX];
        uint32_t klen = 0;
        uint8_t stype = (uint8_t)mb.args[1];
        mb.n_results = 1;
        int idx = -1;
        if (kv_canon_key(heap, mb.args[0], stype, keybuf, &klen))
            idx = kv_find(storage, keybuf, klen);
        if (idx < 0) {
            mb.results[0] = SOROBAN_VOID;
            return true;
        }
        if (storage.val_lens[idx] == 32) {
            mb.results[0] = obj_alloc(heap, SOROBAN_TAG_ADDRESS_OBJECT, 32,
                                      storage.values[idx]);
            return true;
        }
        if (storage.val_lens[idx] >= 8) {
            mb.results[0] = kv_read_u64(storage.values[idx]);
            return true;
        }
        mb.results[0] = SOROBAN_VOID;
        return true;
    }
    case FN_PUT_CONTRACT_DATA: {
        uint8_t keybuf[GPU_KV_KEY_MAX];
        uint32_t klen = 0;
        uint8_t stype = (uint8_t)mb.args[2];
        mb.n_results = 1;
        mb.results[0] = SOROBAN_VOID;
        if (!kv_canon_key(heap, mb.args[0], stype, keybuf, &klen))
            return true;
        uint64_t v = mb.args[1];
        if (soroban_tag(v) == SOROBAN_TAG_ADDRESS_OBJECT) {
            GpuObjEntry* a = obj_get(heap, v);
            uint8_t pk[32] = {};
            if (a) {
                for (uint32_t b = 0; b < 32; ++b) pk[b] = a->data[b];
            }
            gpu_kv_put_raw(storage, keybuf, klen, pk, 32);
        } else {
            gpu_kv_put_u64(storage, keybuf, klen, v);
        }
        return true;
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
