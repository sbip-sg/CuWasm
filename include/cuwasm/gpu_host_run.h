#pragma once
/**
 * Host-side runner: execute a CuOp export against GpuHostState (CPU).
 * Used for token mint/transfer correctness without CUDA.
 */
#include "cuwasm/gpu_host.h"
#include "cuwasm/host.h"
#include "cuwasm/interp.h"

#include <cstring>
#include <string>
#include <vector>

namespace cuwasm {

struct GpuHostRun {
    uint16_t status = ST_UNSUPPORTED_OP;
    uint64_t result = 0;
    std::string error;
};

inline void snapshot_module(const HostModule& m,
                            std::vector<uint8_t>& mem,
                            std::vector<uint64_t>& globals,
                            std::vector<uint8_t>& live,
                            uint32_t& mem_size) {
    mem = m.memory;
    globals = m.globals;
    live = m.data_live;
    mem_size = m.mem_size;
}

inline void restore_module(HostModule& m,
                           const std::vector<uint8_t>& mem,
                           const std::vector<uint64_t>& globals,
                           const std::vector<uint8_t>& live,
                           uint32_t mem_size) {
    m.memory = mem;
    m.globals = globals;
    m.data_live = live;
    m.mem_size = mem_size;
}

inline GpuHostRun run_gpu_host(HostModule& m, const char* export_name,
                               const uint64_t* args, uint32_t n_args,
                               GpuHostState& hs, uint64_t max_steps = DEFAULT_MAX_STEPS) {
    GpuHostRun r;
    int fi = m.find_export(export_name);
    if (fi < 0) {
        r.error = "export not found";
        return r;
    }
    const FuncMeta& f = m.funcs[(uint32_t)fi];
    if (n_args != f.n_params) {
        r.error = "argc mismatch";
        return r;
    }

    std::vector<uint64_t> stack(STACK_CAP, 0);
    std::vector<Frame> frames(FRAME_CAP);
    VmState st{};
    st.pc = f.code_off;
    st.sp = 0;
    st.fp = 0;
    st.csp = 1;
    st.fuel = (int64_t)max_steps;
    st.status = ST_RUNNING;
    st.peak_csp = 1;
    st.mem_size = m.mem_size;
    frames[0] = Frame{0, 0, 0, f.n_results};
    for (uint32_t i = 0; i < n_args; ++i)
        stack[st.sp++] = args[i];
    for (uint16_t i = 0; i < f.n_locals; ++i)
        stack[st.sp++] = 0;

    AoSView sv{stack.data(), STACK_CAP, 0};
    AoSFrameView fv{frames.data(), FRAME_CAP, 0};
    uint64_t dummy_g = 0;
    uint64_t* gptr = m.globals.empty() ? &dummy_g : m.globals.data();
    uint8_t dummy_m = 0;
    MemView memv{m.memory.empty() ? &dummy_m : m.memory.data(), m.mem_size,
                 m.memory.empty() ? 0u : (uint32_t)m.memory.size()};
    DataView data{};
    data.blob = m.data_blob.empty() ? nullptr : m.data_blob.data();
    data.blob_len = (uint32_t)m.data_blob.size();
    data.off = m.data_off.empty() ? nullptr : m.data_off.data();
    data.len = m.data_len.empty() ? nullptr : m.data_len.data();
    data.live = m.data_live.empty() ? nullptr : m.data_live.data();
    data.n = (uint32_t)m.data_live.size();
    HostMailbox mb{};
    DevModule dm = m.dev();

    for (;;) {
        run_instance(dm, st, sv, fv, gptr, (uint32_t)m.globals.size(), memv, data, &mb, max_steps);
        if (st.status != ST_HOSTCALL_PENDING)
            break;
        if (!gpu_host_dispatch(hs, mb, memv.data, st.mem_size)) {
            r.status = ST_UNSUPPORTED_OP;
            r.error = "unsupported host fn";
            return r;
        }
        for (uint16_t i = 0; i < mb.n_results; ++i) {
            if (st.sp >= STACK_CAP) {
                r.status = ST_TRAP_STACK_OVERFLOW;
                return r;
            }
            stack[st.sp++] = mb.results[i];
        }
        st.status = ST_RUNNING;
    }
    m.mem_size = st.mem_size;
    r.status = st.status;
    if (f.n_results > 0 && st.sp > 0)
        r.result = stack[0];
    return r;
}

inline void fill_pk(uint8_t pk[32], uint8_t b) {
    for (int i = 0; i < 32; ++i) pk[i] = b;
}

// Seed heap for mint(alice, 1000): [0]=alice, [1]=i128(1000), [2]=admin (optional).
inline void seed_mint_args(GpuHostState& hs, const uint8_t alice[32], uint64_t amount) {
    gpu_heap_reset(hs.obj_heap);
    obj_alloc(hs.obj_heap, SOROBAN_TAG_ADDRESS_OBJECT, 32, alice);
    uint8_t i128b[16] = {};
    kv_write_u64(i128b, amount);
    obj_alloc(hs.obj_heap, SOROBAN_TAG_I128OBJECT, 16, i128b);
}

inline void seed_balance_args(GpuHostState& hs, const uint8_t addr[32]) {
    gpu_heap_reset(hs.obj_heap);
    obj_alloc(hs.obj_heap, SOROBAN_TAG_ADDRESS_OBJECT, 32, addr);
}

inline void seed_transfer_args(GpuHostState& hs, const uint8_t from[32],
                               const uint8_t to[32], uint64_t amount) {
    gpu_heap_reset(hs.obj_heap);
    obj_alloc(hs.obj_heap, SOROBAN_TAG_ADDRESS_OBJECT, 32, from);
    obj_alloc(hs.obj_heap, SOROBAN_TAG_ADDRESS_OBJECT, 32, to);
    uint8_t i128b[16] = {};
    kv_write_u64(i128b, amount);
    obj_alloc(hs.obj_heap, SOROBAN_TAG_I128OBJECT, 16, i128b);
}

} // namespace cuwasm
