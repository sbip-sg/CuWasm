/**
 * bench.cu — N-thread batch GPU benchmark for CuWASM.
 *
 * Each CUDA thread runs an independent contract instance with:
 *   - Private stack, frame, globals, linear memory, data-live flags
 *   - Private GPU-side object heap (GpuObjHeap) for Soroban Val objects
 *   - Private GPU-side K/V ledger storage (GpuStorage)
 *   - GPU-side host function dispatch (no host boundary crossing)
 *
 * CUDA events measure kernel time only (excluding H2D/D2H transfers).
 * TPS = N_completed / kernel_seconds.
 */

#include "cuwasm/host.h"
#include "cuwasm/interp.h"
#include "cuwasm/gpu_host.h"

#include <cuda_runtime.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

namespace cuwasm {

static constexpr uint32_t BENCH_STACK_CAP  = 512;
static constexpr uint32_t BENCH_FRAME_CAP  = 64;
static constexpr uint64_t BENCH_MAX_STEPS  = 100000000ULL;

// Kernel: one thread = one independent contract instance.
// Host calls handled on-GPU via gpu_host_dispatch (object heap + ledger storage).
__global__ void k_batch(
    DevModule        mod,
    VmState*         states,
    uint64_t*        stacks,      // [n_threads * BENCH_STACK_CAP]
    Frame*           frames_arr,  // [n_threads * BENCH_FRAME_CAP]
    uint64_t*        globals,     // [n_threads * n_globals]
    uint32_t         n_globals,
    uint8_t*         memories,    // [n_threads * mem_max]
    uint32_t         mem_max,
    const uint8_t*   data_blob,
    uint32_t         data_blob_len,
    const uint32_t*  data_off,
    const uint32_t*  data_len,
    uint8_t*         data_lives,  // [n_threads * n_data]
    uint32_t         n_data,
    GpuHostState*    host_states, // [n_threads] — object heap + ledger storage
    uint64_t         max_steps)
{
    uint32_t tid = blockIdx.x * blockDim.x + threadIdx.x;

    VmState&  st       = states[tid];
    uint64_t* th_stack = stacks     + (uint64_t)tid * BENCH_STACK_CAP;
    Frame*    th_frame = frames_arr + (uint64_t)tid * BENCH_FRAME_CAP;
    uint64_t* th_glob  = globals    + (uint64_t)tid * (n_globals ? n_globals : 1u);
    uint8_t*  th_mem   = memories   + (uint64_t)tid * (mem_max ? mem_max : 1u);
    uint8_t*  th_live  = data_lives + (uint64_t)tid * (n_data ? n_data : 1u);

    AoSView      sv{ th_stack, BENCH_STACK_CAP, 0 };
    AoSFrameView fv{ th_frame, BENCH_FRAME_CAP, 0 };
    MemView      mv{ th_mem,   st.mem_size,      mem_max };
    DataView     dv{ const_cast<uint8_t*>(data_blob), data_blob_len,
                     data_off, data_len, th_live, n_data };

    GpuHostState& hstate = host_states[tid];

    HostMailbox mb{};
    for (;;) {
        run_instance(mod, st, sv, fv, th_glob, n_globals, mv, dv, &mb, max_steps);
        if (st.status != ST_HOSTCALL_PENDING)
            break;
        // Dispatch host call on-GPU, passing thread's WASM linear memory
        // so string/vec constructors can read data from it.
        if (!gpu_host_dispatch(hstate, mb, th_mem, st.mem_size)) {
            st.status = ST_UNSUPPORTED_OP;
            break;
        }
        // Push return values onto the thread's stack
        uint32_t sp = st.sp;
        for (uint16_t i = 0; i < mb.n_results && sp < BENCH_STACK_CAP; ++i)
            th_stack[sp++] = mb.results[i];
        st.sp = sp;
        st.status = ST_RUNNING;
    }
}

} // namespace cuwasm

// ─── Helpers ─────────────────────────────────────────────────────────────────

static bool cuda_check(cudaError_t e, const char* what) {
    if (e == cudaSuccess) return true;
    std::fprintf(stderr, "CUDA error in %s: %s\n", what, cudaGetErrorString(e));
    return false;
}
#define CU(expr) do { if (!cuda_check(expr, #expr)) return 1; } while(0)

int main(int argc, char** argv) {
    if (argc < 3) {
        std::fprintf(stderr,
            "usage: bench <module.wasm> <export> [n_threads=%d] [block_size=%d] [i64args...]\n",
            8192, 256);
        return 2;
    }
    const char* wasm_path = argv[1];
    const char* exp_name  = argv[2];
    int n_threads   = argc > 3 ? std::atoi(argv[3]) : 8192;
    int block_size  = argc > 4 ? std::atoi(argv[4]) : 256;
    int n_blocks    = (n_threads + block_size - 1) / block_size;

    std::vector<uint64_t> call_args;
    for (int i = 5; i < argc; ++i)
        call_args.push_back((uint64_t)std::strtoll(argv[i], nullptr, 16));

    std::string err;
    std::vector<uint8_t> wasm_bytes;
    if (!cuwasm::load_file(wasm_path, wasm_bytes, err)) {
        std::fprintf(stderr, "load: %s\n", err.c_str());
        return 1;
    }
    cuwasm::HostModule hm;
    if (!cuwasm::translate_wasm(wasm_bytes.data(), wasm_bytes.size(), hm, err) ||
        !cuwasm::verify_cuop(hm, err)) {
        std::fprintf(stderr, "translate/verify: %s\n", err.c_str());
        return 1;
    }
    int fi = hm.find_export(exp_name);
    if (fi < 0) {
        std::fprintf(stderr, "export not found: %s\n", exp_name);
        return 1;
    }
    const cuwasm::FuncMeta& func = hm.funcs[(uint32_t)fi];

    // ── GPU setup ────────────────────────────────────────────────────────
    uint32_t n_globals = (uint32_t)hm.globals.size();
    uint32_t mem_max   = hm.mem_size;  // actually-used bytes
    uint32_t n_data    = (uint32_t)hm.data_live.size();

    // Pad call_args to match function signature
    std::vector<uint64_t> padded_args(func.n_params, 0);
    for (size_t i = 0; i < call_args.size() && i < padded_args.size(); ++i)
        padded_args[i] = call_args[i];

    cuwasm::VmState st0{};
    st0.pc       = func.code_off;
    st0.sp       = func.n_params + func.n_locals;
    st0.fp       = 0;
    st0.csp      = 1;
    st0.fuel     = (int64_t)cuwasm::BENCH_MAX_STEPS;
    st0.status   = cuwasm::ST_RUNNING;
    st0.peak_csp = 1;
    st0.mem_size = mem_max;

    std::vector<uint64_t> h_stack_tmpl(cuwasm::BENCH_STACK_CAP, 0);
    for (size_t i = 0; i < padded_args.size() && i < cuwasm::BENCH_STACK_CAP; ++i)
        h_stack_tmpl[i] = padded_args[i];

    std::vector<cuwasm::Frame> h_frame_tmpl(cuwasm::BENCH_FRAME_CAP);
    h_frame_tmpl[0] = cuwasm::Frame{0, 0, 0, func.n_results};

    // ── Device allocations ───────────────────────────────────────────────
    cuwasm::CuOp*     d_code   = nullptr;
    uint64_t*  d_consts        = nullptr;
    cuwasm::FuncMeta* d_funcs  = nullptr;
    const uint8_t*  d_blob     = nullptr;
    const uint32_t* d_doff     = nullptr;
    const uint32_t* d_dlen     = nullptr;
    uint32_t* d_table  = nullptr;
    uint32_t* d_tidx   = nullptr;
    uint64_t* d_tfp    = nullptr;

    cuwasm::VmState*       d_states     = nullptr;
    uint64_t*              d_stacks     = nullptr;
    cuwasm::Frame*         d_frames     = nullptr;
    uint64_t*              d_globals    = nullptr;
    uint8_t*               d_mems       = nullptr;
    uint8_t*               d_lives      = nullptr;
    cuwasm::GpuHostState*  d_host_states = nullptr;

    size_t code_bytes   = hm.code.size() * sizeof(cuwasm::CuOp);
    size_t consts_bytes = (hm.consts.empty() ? 1 : hm.consts.size()) * 8;
    size_t funcs_bytes  = hm.funcs.size() * sizeof(cuwasm::FuncMeta);
    size_t blob_bytes   = hm.data_blob.empty() ? 1 : hm.data_blob.size();
    size_t doff_bytes   = (hm.data_off.empty() ? 1 : hm.data_off.size()) * 4;
    size_t dlen_bytes   = (hm.data_len.empty() ? 1 : hm.data_len.size()) * 4;
    size_t table_bytes  = (hm.table.empty() ? 1 : hm.table.size()) * 4;
    size_t tidx_bytes   = (hm.func_typeidx.empty() ? 1 : hm.func_typeidx.size()) * 4;
    size_t tfp_bytes    = (hm.type_fp.empty() ? 1 : hm.type_fp.size()) * 8;

    size_t per_states    = (size_t)n_threads * sizeof(cuwasm::VmState);
    size_t per_stacks    = (size_t)n_threads * cuwasm::BENCH_STACK_CAP * 8;
    size_t per_frames    = (size_t)n_threads * cuwasm::BENCH_FRAME_CAP * sizeof(cuwasm::Frame);
    size_t per_globs     = (size_t)n_threads * (n_globals ? n_globals : 1u) * 8;
    size_t per_mems      = (size_t)n_threads * (mem_max ? mem_max : 1u);
    size_t per_lives     = (size_t)n_threads * (n_data ? n_data : 1u);
    size_t per_hstates   = (size_t)n_threads * sizeof(cuwasm::GpuHostState);

    size_t total_bytes   = code_bytes + consts_bytes + funcs_bytes +
                           per_states + per_stacks + per_frames + per_globs +
                           per_mems + per_lives + per_hstates;
    std::fprintf(stderr, "bench: %d threads, %zu MB device memory\n",
                 n_threads, total_bytes / (1024*1024));
    std::fprintf(stderr, "  per-thread: stack=%zuB frames=%zuB globals=%zuB "
                 "mem=%uB host_state=%zuB\n",
                 cuwasm::BENCH_STACK_CAP * 8,
                 cuwasm::BENCH_FRAME_CAP * sizeof(cuwasm::Frame),
                 (size_t)(n_globals ? n_globals : 1u) * 8,
                 mem_max ? mem_max : 1u,
                 sizeof(cuwasm::GpuHostState));

    CU(cudaMalloc(&d_code,   code_bytes));
    CU(cudaMalloc(&d_consts, consts_bytes));
    CU(cudaMalloc(&d_funcs,  funcs_bytes));
    CU(cudaMalloc((void**)&d_blob, blob_bytes));
    CU(cudaMalloc((void**)&d_doff, doff_bytes));
    CU(cudaMalloc((void**)&d_dlen, dlen_bytes));
    CU(cudaMalloc((void**)&d_table, table_bytes));
    CU(cudaMalloc((void**)&d_tidx, tidx_bytes));
    CU(cudaMalloc((void**)&d_tfp, tfp_bytes));
    CU(cudaMalloc(&d_states,      per_states));
    CU(cudaMalloc(&d_stacks,      per_stacks));
    CU(cudaMalloc(&d_frames,      per_frames));
    CU(cudaMalloc(&d_globals,     per_globs));
    CU(cudaMalloc(&d_mems,        per_mems ? per_mems : 1));
    CU(cudaMalloc(&d_lives,       per_lives ? per_lives : 1));
    CU(cudaMalloc(&d_host_states, per_hstates));

    // Upload read-only module data (shared across all threads)
    CU(cudaMemcpy(d_code, hm.code.data(), code_bytes, cudaMemcpyHostToDevice));
    if (!hm.consts.empty())
        CU(cudaMemcpy(d_consts, hm.consts.data(), hm.consts.size()*8, cudaMemcpyHostToDevice));
    CU(cudaMemcpy(d_funcs, hm.funcs.data(), funcs_bytes, cudaMemcpyHostToDevice));
    if (!hm.data_blob.empty())
        CU(cudaMemcpy((void*)d_blob, hm.data_blob.data(), hm.data_blob.size(), cudaMemcpyHostToDevice));
    if (!hm.data_off.empty())
        CU(cudaMemcpy((void*)d_doff, hm.data_off.data(), hm.data_off.size()*4, cudaMemcpyHostToDevice));
    if (!hm.data_len.empty())
        CU(cudaMemcpy((void*)d_dlen, hm.data_len.data(), hm.data_len.size()*4, cudaMemcpyHostToDevice));
    if (!hm.table.empty())
        CU(cudaMemcpy(d_table, hm.table.data(), hm.table.size()*4, cudaMemcpyHostToDevice));
    if (!hm.func_typeidx.empty())
        CU(cudaMemcpy(d_tidx, hm.func_typeidx.data(), hm.func_typeidx.size()*4, cudaMemcpyHostToDevice));
    if (!hm.type_fp.empty())
        CU(cudaMemcpy(d_tfp, hm.type_fp.data(), hm.type_fp.size()*8, cudaMemcpyHostToDevice));

    cuwasm::DevModule dm{};
    dm.code     = d_code;
    dm.consts   = d_consts;
    dm.funcs    = d_funcs;
    dm.n_funcs  = (uint32_t)hm.funcs.size();
    dm.code_len = (uint32_t)hm.code.size();
    dm.table    = hm.table.empty()    ? nullptr : d_table;
    dm.table_len= (uint32_t)hm.table.size();
    dm.func_typeidx = hm.func_typeidx.empty() ? nullptr : d_tidx;
    dm.type_fp  = hm.type_fp.empty() ? nullptr : d_tfp;
    dm.n_types  = (uint32_t)hm.type_fp.size();

    // ── reinit: reset all per-thread mutable state ───────────────────────
    auto reinit = [&]() {
        {
            std::vector<cuwasm::VmState> h_st(n_threads, st0);
            cudaMemcpy(d_states, h_st.data(), per_states, cudaMemcpyHostToDevice);
        }
        {
            std::vector<uint64_t> h_s((size_t)n_threads * cuwasm::BENCH_STACK_CAP, 0);
            for (int t = 0; t < n_threads; ++t)
                std::memcpy(&h_s[(size_t)t * cuwasm::BENCH_STACK_CAP],
                            h_stack_tmpl.data(), cuwasm::BENCH_STACK_CAP * 8);
            cudaMemcpy(d_stacks, h_s.data(), per_stacks, cudaMemcpyHostToDevice);
        }
        {
            std::vector<cuwasm::Frame> h_f((size_t)n_threads * cuwasm::BENCH_FRAME_CAP);
            for (int t = 0; t < n_threads; ++t)
                std::memcpy(&h_f[(size_t)t * cuwasm::BENCH_FRAME_CAP],
                            h_frame_tmpl.data(), cuwasm::BENCH_FRAME_CAP * sizeof(cuwasm::Frame));
            cudaMemcpy(d_frames, h_f.data(), per_frames, cudaMemcpyHostToDevice);
        }
        if (n_globals) {
            std::vector<uint64_t> h_g((size_t)n_threads * n_globals);
            for (int t = 0; t < n_threads; ++t)
                std::memcpy(&h_g[(size_t)t * n_globals], hm.globals.data(), n_globals*8);
            cudaMemcpy(d_globals, h_g.data(), per_globs, cudaMemcpyHostToDevice);
        }
        if (mem_max) {
            std::vector<uint8_t> h_m((size_t)n_threads * mem_max);
            for (int t = 0; t < n_threads; ++t)
                std::memcpy(&h_m[(size_t)t * mem_max], hm.memory.data(), mem_max);
            cudaMemcpy(d_mems, h_m.data(), per_mems, cudaMemcpyHostToDevice);
        }
        if (n_data) {
            std::vector<uint8_t> h_l((size_t)n_threads * n_data, 1);
            cudaMemcpy(d_lives, h_l.data(), per_lives, cudaMemcpyHostToDevice);
        }
        // Zero-init all per-thread host states (empty object heap + storage)
        cudaMemset(d_host_states, 0, per_hstates);
    };

    auto launch = [&]() {
        cuwasm::k_batch<<<n_blocks, block_size>>>(
            dm, d_states, d_stacks, d_frames, d_globals, n_globals,
            d_mems, mem_max,
            d_blob, (uint32_t)hm.data_blob.size(), d_doff, d_dlen,
            d_lives, n_data,
            d_host_states,
            cuwasm::BENCH_MAX_STEPS);
    };

    // ── Warmup run ───────────────────────────────────────────────────────
    reinit();
    launch();
    CU(cudaDeviceSynchronize());

    // Check results and print thread[0] diagnostics
    {
        std::vector<cuwasm::VmState> h_out(n_threads);
        cudaMemcpy(h_out.data(), d_states, per_states, cudaMemcpyDeviceToHost);
        int ok = 0, pending = 0, unsup = 0, other = 0;
        uint16_t first_fail = 0;
        for (int i = 0; i < n_threads; ++i) {
            auto s = h_out[i].status;
            if (s == cuwasm::ST_OK) ok++;
            else if (s == cuwasm::ST_HOSTCALL_PENDING) pending++;
            else if (s == cuwasm::ST_UNSUPPORTED_OP) unsup++;
            else { if (!other) first_fail = s; other++; }
        }
        std::fprintf(stderr, "warmup: ok=%d pending=%d unsupported=%d other=%d",
                     ok, pending, unsup, other);
        if (other) std::fprintf(stderr, " (first=%s)", cuwasm::status_name(first_fail));
        std::fprintf(stderr, "\n");

        // Diagnostics for thread[0]
        if (func.n_results > 0) {
            uint64_t result0 = 0;
            cudaMemcpy(&result0, d_stacks, sizeof(uint64_t), cudaMemcpyDeviceToHost);
            std::fprintf(stderr, "  thread[0] result = 0x%llx\n", (long long)result0);
        }
        {
            cuwasm::GpuHostState h_hstate;
            cudaMemcpy(&h_hstate, d_host_states, sizeof(cuwasm::GpuHostState), cudaMemcpyDeviceToHost);
            std::fprintf(stderr, "  thread[0] obj_heap: %u objects, storage: %u entries\n",
                         h_hstate.obj_heap.count, h_hstate.storage.count);
            for (uint32_t i = 0; i < h_hstate.storage.count && i < 4; ++i)
                std::fprintf(stderr, "    storage[%u] key=0x%llx val=0x%llx\n", i,
                             (long long)h_hstate.storage.entries[i].key,
                             (long long)h_hstate.storage.entries[i].val);
        }

        if (ok == 0) {
            std::fprintf(stderr, "ERROR: zero threads succeeded; aborting.\n");
            std::printf("{\"contract\":\"%s\",\"export\":\"%s\","
                        "\"n_threads\":%d,\"kernel_ms\":0,\"tps\":0,\"ok\":0}\n",
                        wasm_path, exp_name, n_threads);
            return 1;
        }
    }

    // ── Timed run ────────────────────────────────────────────────────────
    reinit();
    cudaEvent_t ev0, ev1;
    CU(cudaEventCreate(&ev0));
    CU(cudaEventCreate(&ev1));
    CU(cudaEventRecord(ev0));
    launch();
    CU(cudaEventRecord(ev1));
    CU(cudaGetLastError());
    CU(cudaEventSynchronize(ev1));

    float ms = 0.f;
    CU(cudaEventElapsedTime(&ms, ev0, ev1));

    // Count successes
    int n_ok = 0;
    {
        std::vector<cuwasm::VmState> h_out(n_threads);
        cudaMemcpy(h_out.data(), d_states, per_states, cudaMemcpyDeviceToHost);
        for (int i = 0; i < n_threads; ++i)
            if (h_out[i].status == cuwasm::ST_OK) n_ok++;
    }

    double tps = (double)n_ok / (ms / 1000.0);
    std::printf(
        "{\"contract\":\"%s\",\"export\":\"%s\","
        "\"n_threads\":%d,\"blocks\":%d,\"block_size\":%d,"
        "\"ok\":%d,\"kernel_ms\":%.3f,\"tps\":%.0f}\n",
        wasm_path, exp_name, n_threads, n_blocks, block_size,
        n_ok, (double)ms, tps);

    cudaFree(d_code); cudaFree(d_consts); cudaFree(d_funcs);
    cudaFree(d_states); cudaFree(d_stacks); cudaFree(d_frames);
    cudaFree(d_globals); cudaFree(d_mems); cudaFree(d_lives);
    cudaFree((void*)d_blob); cudaFree((void*)d_doff); cudaFree((void*)d_dlen);
    cudaFree(d_table); cudaFree(d_tidx); cudaFree(d_tfp);
    cudaFree(d_host_states);
    cudaEventDestroy(ev0);
    cudaEventDestroy(ev1);
    return 0;
}
