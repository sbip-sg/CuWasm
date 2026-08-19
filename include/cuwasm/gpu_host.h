#pragma once

#include "hd.h"
#include "vmstate.h"

/**
 * GPU-side host function simulation.
 *
 * Each CUDA thread owns a small fixed-capacity key-value store
 * (GpuStorage) that implements the Soroban ledger-storage functions:
 *   has_contract_data, get_contract_data, put_contract_data,
 *   extend_contract_data_ttl, extend_current_contract_instance_and_code_ttl.
 *
 * Other host functions (require_auth, contract_event, etc.) return
 * stub values appropriate for benchmark execution.
 *
 * Keys and values are raw Soroban Val payloads (uint64_t).
 */

namespace cuwasm {

static constexpr uint32_t GPU_STORAGE_CAP = 16;  // max KV entries per thread

struct GpuStorageEntry {
    uint64_t key;
    uint64_t val;
    uint8_t  occupied;
    uint8_t  _pad[7];
};

struct GpuStorage {
    GpuStorageEntry entries[GPU_STORAGE_CAP];
    uint32_t count;
};

// Soroban Val tag constants used for encoding results
// Bool: tag=0, true payload = (1<<1)|0 = 2; false = 0
static constexpr uint64_t SOROBAN_TRUE  = 0x0000000000000001ULL;  // Bool true
static constexpr uint64_t SOROBAN_FALSE = 0x0000000000000000ULL;  // Bool false
static constexpr uint64_t SOROBAN_VOID  = 0x0000000000000002ULL;  // Void

// fn_id constants (from docs/soroban-env.json enumeration order)
static constexpr uint32_t FN_CONTRACT_EVENT     =   2;
static constexpr uint32_t FN_GET_LEDGER_SEQ     =   4;
static constexpr uint32_t FN_GET_CUR_ADDR       =   8;
static constexpr uint32_t FN_OBJ_FROM_I128      =  17;
static constexpr uint32_t FN_OBJ_TO_I128_LO     =  18;
static constexpr uint32_t FN_OBJ_TO_I128_HI     =  19;
static constexpr uint32_t FN_MAP_NEW_LM         =  72;
static constexpr uint32_t FN_MAP_UNPACK_LM      =  73;
static constexpr uint32_t FN_VEC_NEW_LM         =  93;
static constexpr uint32_t FN_PUT_CONTRACT_DATA   =  95;
static constexpr uint32_t FN_HAS_CONTRACT_DATA   =  96;
static constexpr uint32_t FN_GET_CONTRACT_DATA   =  97;
static constexpr uint32_t FN_EXTEND_DATA_TTL     = 103;
static constexpr uint32_t FN_EXTEND_INST_TTL     = 104;
static constexpr uint32_t FN_STR_NEW_LM          = 137;
static constexpr uint32_t FN_SYM_NEW_LM          = 138;
static constexpr uint32_t FN_REQUIRE_AUTH         = 182;

/**
 * Handle a host call on-GPU.  Returns true if handled successfully;
 * on return, mb->n_results and mb->results[] are filled in.
 * st.sp is NOT adjusted here — the caller (k_batch) must push
 * mb->results onto the stack after calling this.
 */
HD bool gpu_host_dispatch(GpuStorage& store, HostMailbox& mb) {
    switch (mb.fn_id) {

    // ─── Storage: has_contract_data(key, type) ─────────────────────────
    case FN_HAS_CONTRACT_DATA: {
        uint64_t key = mb.args[0];
        for (uint32_t i = 0; i < store.count; ++i) {
            if (store.entries[i].occupied && store.entries[i].key == key) {
                mb.n_results = 1;
                mb.results[0] = SOROBAN_TRUE;
                return true;
            }
        }
        mb.n_results = 1;
        mb.results[0] = SOROBAN_FALSE;
        return true;
    }

    // ─── Storage: get_contract_data(key, type) ─────────────────────────
    case FN_GET_CONTRACT_DATA: {
        uint64_t key = mb.args[0];
        for (uint32_t i = 0; i < store.count; ++i) {
            if (store.entries[i].occupied && store.entries[i].key == key) {
                mb.n_results = 1;
                mb.results[0] = store.entries[i].val;
                return true;
            }
        }
        // Key not found — return 0 (stub; real host would trap)
        mb.n_results = 1;
        mb.results[0] = 0;
        return true;
    }

    // ─── Storage: put_contract_data(key, val, type) ────────────────────
    case FN_PUT_CONTRACT_DATA: {
        uint64_t key = mb.args[0];
        uint64_t val = mb.args[1];
        // Update existing
        for (uint32_t i = 0; i < store.count; ++i) {
            if (store.entries[i].occupied && store.entries[i].key == key) {
                store.entries[i].val = val;
                mb.n_results = 1;
                mb.results[0] = SOROBAN_VOID;
                return true;
            }
        }
        // Insert new
        if (store.count < GPU_STORAGE_CAP) {
            store.entries[store.count].key = key;
            store.entries[store.count].val = val;
            store.entries[store.count].occupied = 1;
            store.count++;
        }
        mb.n_results = 1;
        mb.results[0] = SOROBAN_VOID;
        return true;
    }

    // ─── TTL extensions: no-op ─────────────────────────────────────────
    case FN_EXTEND_DATA_TTL:
    case FN_EXTEND_INST_TTL:
        mb.n_results = 1;
        mb.results[0] = SOROBAN_VOID;
        return true;

    // ─── Auth: stub (always succeed) ───────────────────────────────────
    case FN_REQUIRE_AUTH:
        mb.n_results = 1;
        mb.results[0] = SOROBAN_VOID;
        return true;

    // ─── Events: no-op ─────────────────────────────────────────────────
    case FN_CONTRACT_EVENT:
        mb.n_results = 1;
        mb.results[0] = SOROBAN_VOID;
        return true;

    // ─── Ledger sequence: return a constant ────────────────────────────
    case FN_GET_LEDGER_SEQ:
        mb.n_results = 1;
        // Encode as U32Val: payload = (value << 32) | tag_bits
        // For simplicity, return raw value; the contract may just compare.
        mb.results[0] = 100;  // stub sequence number
        return true;

    // ─── get_current_contract_address: return a stub address ───────────
    case FN_GET_CUR_ADDR:
        mb.n_results = 1;
        mb.results[0] = 0x100;  // stub address object handle
        return true;

    // ─── i128 helpers ──────────────────────────────────────────────────
    case FN_OBJ_FROM_I128:
        mb.n_results = 1;
        mb.results[0] = mb.args[1]; // return lo part as stub handle
        return true;
    case FN_OBJ_TO_I128_LO:
        mb.n_results = 1;
        mb.results[0] = mb.args[0]; // identity stub
        return true;
    case FN_OBJ_TO_I128_HI:
        mb.n_results = 1;
        mb.results[0] = 0;
        return true;

    // ─── Linear-memory object constructors: return stub handles ────────
    case FN_STR_NEW_LM:
    case FN_SYM_NEW_LM:
    case FN_VEC_NEW_LM:
    case FN_MAP_NEW_LM:
        mb.n_results = 1;
        mb.results[0] = 0x200; // stub object handle
        return true;

    case FN_MAP_UNPACK_LM:
        mb.n_results = 1;
        mb.results[0] = SOROBAN_VOID;
        return true;

    default:
        return false; // unsupported — will trap
    }
}

} // namespace cuwasm
