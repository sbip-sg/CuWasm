#include "cuwasm/gpu.h"
#include "cuwasm/host.h"

#include <cuda_runtime.h>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

namespace cuwasm {

__global__ void k_run(DevModule m, VmState* st, uint64_t* stack, Frame* frames,
                      uint64_t* globals, uint32_t n_globals, uint8_t* mem, uint32_t mem_max,
                      uint8_t* data_blob, uint32_t data_blob_len, const uint32_t* data_off,
                      const uint32_t* data_len, uint8_t* data_live, uint32_t n_data,
                      HostMailbox* mb, uint32_t stack_cap, uint32_t frame_cap, uint64_t max_steps) {
    if (blockIdx.x != 0 || threadIdx.x != 0)
        return;
    AoSView sv{stack, stack_cap, 0};
    AoSFrameView fv{frames, frame_cap, 0};
    MemView mv{mem, st->mem_size, mem_max};
    DataView dv{data_blob, data_blob_len, data_off, data_len, data_live, n_data};
    run_instance(m, *st, sv, fv, globals, n_globals, mv, dv, mb, max_steps);
}

static bool cuda_ok(cudaError_t e, std::string& err, const char* what) {
    if (e == cudaSuccess)
        return true;
    err = std::string(what) + ": " + cudaGetErrorString(e);
    return false;
}

RunResult run_gpu(HostModule& m, uint32_t func_idx, const uint64_t* args, uint32_t n_args,
                  uint64_t max_steps, HostFn host_fn) {
    RunResult r;
    if (func_idx >= m.funcs.size()) {
        r.status = ST_UNSUPPORTED_OP;
        return r;
    }
    const FuncMeta& f = m.funcs[func_idx];
    if (n_args != f.n_params) {
        r.status = ST_UNSUPPORTED_OP;
        return r;
    }
    if (!host_fn)
        host_fn = default_host_fn;

    std::vector<uint64_t> h_stack(STACK_CAP, 0);
    std::vector<Frame> h_frames(FRAME_CAP);
    VmState st{};
    st.pc = f.code_off;
    st.sp = 0;
    st.fp = 0;
    st.csp = 1;
    st.fuel = 1000000000000LL;
    st.status = ST_RUNNING;
    st.peak_csp = 1;
    st.mem_size = m.mem_size;
    h_frames[0] = Frame{0, 0, 0, f.n_results};
    for (uint32_t i = 0; i < n_args; ++i)
        h_stack[st.sp++] = args[i];
    for (uint16_t i = 0; i < f.n_locals; ++i)
        h_stack[st.sp++] = 0;

    std::string err;
    CuOp* d_code = nullptr;
    uint64_t* d_consts = nullptr;
    FuncMeta* d_funcs = nullptr;
    uint64_t* d_stack = nullptr;
    Frame* d_frames = nullptr;
    VmState* d_st = nullptr;
    uint64_t* d_globals = nullptr;
    uint8_t* d_mem = nullptr;
    uint8_t* d_blob = nullptr;
    uint32_t* d_doff = nullptr;
    uint32_t* d_dlen = nullptr;
    uint8_t* d_live = nullptr;
    uint32_t* d_table = nullptr;
    uint32_t* d_tidx = nullptr;
    uint64_t* d_tfp = nullptr;
    HostMailbox* d_mb = nullptr;

    auto fail = [&](uint16_t status) {
        r.status = status;
        cudaFree(d_code);
        cudaFree(d_consts);
        cudaFree(d_funcs);
        cudaFree(d_stack);
        cudaFree(d_frames);
        cudaFree(d_st);
        cudaFree(d_globals);
        cudaFree(d_mem);
        cudaFree(d_blob);
        cudaFree(d_doff);
        cudaFree(d_dlen);
        cudaFree(d_live);
        cudaFree(d_table);
        cudaFree(d_tidx);
        cudaFree(d_tfp);
        cudaFree(d_mb);
        return r;
    };

    auto mal = [&](void** p, size_t n, const char* w) {
        if (n == 0)
            n = 1;
        return cuda_ok(cudaMalloc(p, n), err, w);
    };

    if (m.code.empty() || !mal((void**)&d_code, m.code.size() * sizeof(CuOp), "code"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_consts, (m.consts.empty() ? 1 : m.consts.size()) * sizeof(uint64_t), "consts"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_funcs, m.funcs.size() * sizeof(FuncMeta), "funcs"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_stack, STACK_CAP * sizeof(uint64_t), "stack"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_frames, FRAME_CAP * sizeof(Frame), "frames"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_st, sizeof(VmState), "state"))
        return fail(ST_UNSUPPORTED_OP);
    uint32_t n_globals = (uint32_t)m.globals.size();
    if (!mal((void**)&d_globals, (n_globals ? n_globals : 1u) * sizeof(uint64_t), "globals"))
        return fail(ST_UNSUPPORTED_OP);
    uint32_t mem_max = (uint32_t)m.memory.size();
    if (!mal((void**)&d_mem, mem_max ? mem_max : 1u, "mem"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_blob, m.data_blob.empty() ? 1 : m.data_blob.size(), "blob"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_doff, (m.data_off.empty() ? 1 : m.data_off.size()) * sizeof(uint32_t), "doff"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_dlen, (m.data_len.empty() ? 1 : m.data_len.size()) * sizeof(uint32_t), "dlen"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_live, m.data_live.empty() ? 1 : m.data_live.size(), "live"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_table, (m.table.empty() ? 1 : m.table.size()) * sizeof(uint32_t), "table"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_tidx, (m.func_typeidx.empty() ? 1 : m.func_typeidx.size()) * sizeof(uint32_t),
             "tidx"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_tfp, (m.type_fp.empty() ? 1 : m.type_fp.size()) * sizeof(uint64_t), "tfp"))
        return fail(ST_UNSUPPORTED_OP);
    if (!mal((void**)&d_mb, sizeof(HostMailbox), "mb"))
        return fail(ST_UNSUPPORTED_OP);

    if (n_globals &&
        !cuda_ok(cudaMemcpy(d_globals, m.globals.data(), n_globals * sizeof(uint64_t), cudaMemcpyHostToDevice),
                 err, "h2d globals"))
        return fail(ST_UNSUPPORTED_OP);
    if (!cuda_ok(cudaMemcpy(d_code, m.code.data(), m.code.size() * sizeof(CuOp), cudaMemcpyHostToDevice),
                 err, "h2d code"))
        return fail(ST_UNSUPPORTED_OP);
    if (!m.consts.empty() &&
        !cuda_ok(cudaMemcpy(d_consts, m.consts.data(), m.consts.size() * sizeof(uint64_t),
                            cudaMemcpyHostToDevice),
                 err, "h2d consts"))
        return fail(ST_UNSUPPORTED_OP);
    if (!cuda_ok(cudaMemcpy(d_funcs, m.funcs.data(), m.funcs.size() * sizeof(FuncMeta),
                            cudaMemcpyHostToDevice),
                 err, "h2d funcs"))
        return fail(ST_UNSUPPORTED_OP);
    if (!cuda_ok(cudaMemcpy(d_stack, h_stack.data(), STACK_CAP * sizeof(uint64_t), cudaMemcpyHostToDevice),
                 err, "h2d stack"))
        return fail(ST_UNSUPPORTED_OP);
    if (!cuda_ok(cudaMemcpy(d_frames, h_frames.data(), FRAME_CAP * sizeof(Frame), cudaMemcpyHostToDevice),
                 err, "h2d frames"))
        return fail(ST_UNSUPPORTED_OP);
    if (mem_max &&
        !cuda_ok(cudaMemcpy(d_mem, m.memory.data(), mem_max, cudaMemcpyHostToDevice), err, "h2d mem"))
        return fail(ST_UNSUPPORTED_OP);
    if (!m.data_blob.empty() &&
        !cuda_ok(cudaMemcpy(d_blob, m.data_blob.data(), m.data_blob.size(), cudaMemcpyHostToDevice), err,
                 "h2d blob"))
        return fail(ST_UNSUPPORTED_OP);
    if (!m.data_off.empty() &&
        !cuda_ok(cudaMemcpy(d_doff, m.data_off.data(), m.data_off.size() * sizeof(uint32_t),
                            cudaMemcpyHostToDevice),
                 err, "h2d doff"))
        return fail(ST_UNSUPPORTED_OP);
    if (!m.data_len.empty() &&
        !cuda_ok(cudaMemcpy(d_dlen, m.data_len.data(), m.data_len.size() * sizeof(uint32_t),
                            cudaMemcpyHostToDevice),
                 err, "h2d dlen"))
        return fail(ST_UNSUPPORTED_OP);
    if (!m.data_live.empty() &&
        !cuda_ok(cudaMemcpy(d_live, m.data_live.data(), m.data_live.size(), cudaMemcpyHostToDevice), err,
                 "h2d live"))
        return fail(ST_UNSUPPORTED_OP);
    if (!m.table.empty() &&
        !cuda_ok(cudaMemcpy(d_table, m.table.data(), m.table.size() * sizeof(uint32_t),
                            cudaMemcpyHostToDevice),
                 err, "h2d table"))
        return fail(ST_UNSUPPORTED_OP);
    if (!m.func_typeidx.empty() &&
        !cuda_ok(cudaMemcpy(d_tidx, m.func_typeidx.data(), m.func_typeidx.size() * sizeof(uint32_t),
                            cudaMemcpyHostToDevice),
                 err, "h2d tidx"))
        return fail(ST_UNSUPPORTED_OP);
    if (!m.type_fp.empty() &&
        !cuda_ok(cudaMemcpy(d_tfp, m.type_fp.data(), m.type_fp.size() * sizeof(uint64_t),
                            cudaMemcpyHostToDevice),
                 err, "h2d tfp"))
        return fail(ST_UNSUPPORTED_OP);
    if (!cuda_ok(cudaMemcpy(d_st, &st, sizeof(VmState), cudaMemcpyHostToDevice), err, "h2d st"))
        return fail(ST_UNSUPPORTED_OP);

    DevModule dm{};
    dm.code = d_code;
    dm.consts = d_consts;
    dm.funcs = d_funcs;
    dm.n_funcs = (uint32_t)m.funcs.size();
    dm.code_len = (uint32_t)m.code.size();
    dm.table = m.table.empty() ? nullptr : d_table;
    dm.table_len = (uint32_t)m.table.size();
    dm.func_typeidx = m.func_typeidx.empty() ? nullptr : d_tidx;
    dm.type_fp = m.type_fp.empty() ? nullptr : d_tfp;
    dm.n_types = (uint32_t)m.type_fp.size();

    HostMailbox h_mb{};
    HostCallContext ctx{&m};

    for (;;) {
        if (st.status == ST_HOSTCALL_PENDING) {
            if (!cuda_ok(cudaMemcpy(&h_mb, d_mb, sizeof(HostMailbox), cudaMemcpyDeviceToHost), err,
                         "d2h mb"))
                return fail(ST_UNSUPPORTED_OP);
            if (mem_max &&
                !cuda_ok(cudaMemcpy(m.memory.data(), d_mem, mem_max, cudaMemcpyDeviceToHost), err,
                         "d2h mem for host"))
                return fail(ST_UNSUPPORTED_OP);
            std::string herr;
            if (!host_fn(ctx, h_mb, herr)) {
                r.status = ST_UNSUPPORTED_OP;
                r.error = herr;
                r.peak_csp = st.peak_csp;
                return fail(ST_UNSUPPORTED_OP);
            }
            for (uint16_t i = 0; i < h_mb.n_results; ++i) {
                if (st.sp >= STACK_CAP)
                    return fail(ST_TRAP_STACK_OVERFLOW);
                h_stack[st.sp++] = h_mb.results[i];
            }
            st.status = ST_RUNNING;
            if (!cuda_ok(cudaMemcpy(d_st, &st, sizeof(VmState), cudaMemcpyHostToDevice), err,
                         "h2d st resume"))
                return fail(ST_UNSUPPORTED_OP);
            if (!cuda_ok(cudaMemcpy(d_stack, h_stack.data(), STACK_CAP * sizeof(uint64_t),
                                    cudaMemcpyHostToDevice),
                         err, "h2d stack resume"))
                return fail(ST_UNSUPPORTED_OP);
            if (mem_max &&
                !cuda_ok(cudaMemcpy(d_mem, m.memory.data(), mem_max, cudaMemcpyHostToDevice), err,
                         "h2d mem resume"))
                return fail(ST_UNSUPPORTED_OP);
        }

        if (st.status != ST_RUNNING)
            break;

        k_run<<<1, 1>>>(dm, d_st, d_stack, d_frames, d_globals, n_globals, d_mem, mem_max, d_blob,
                        (uint32_t)m.data_blob.size(), d_doff, d_dlen, d_live, (uint32_t)m.data_live.size(),
                        d_mb, STACK_CAP, FRAME_CAP, max_steps);
        if (!cuda_ok(cudaGetLastError(), err, "launch"))
            return fail(ST_UNSUPPORTED_OP);
        if (!cuda_ok(cudaDeviceSynchronize(), err, "sync"))
            return fail(ST_UNSUPPORTED_OP);

        if (!cuda_ok(cudaMemcpy(&st, d_st, sizeof(VmState), cudaMemcpyDeviceToHost), err, "d2h st"))
            return fail(ST_UNSUPPORTED_OP);
        if (!cuda_ok(cudaMemcpy(h_stack.data(), d_stack, STACK_CAP * sizeof(uint64_t),
                                cudaMemcpyDeviceToHost),
                     err, "d2h stack"))
            return fail(ST_UNSUPPORTED_OP);
        if (n_globals &&
            !cuda_ok(cudaMemcpy(m.globals.data(), d_globals, n_globals * sizeof(uint64_t),
                                cudaMemcpyDeviceToHost),
                     err, "d2h globals"))
            return fail(ST_UNSUPPORTED_OP);
        if (mem_max &&
            !cuda_ok(cudaMemcpy(m.memory.data(), d_mem, mem_max, cudaMemcpyDeviceToHost), err,
                     "d2h mem"))
            return fail(ST_UNSUPPORTED_OP);
        if (!m.data_live.empty() &&
            !cuda_ok(cudaMemcpy(m.data_live.data(), d_live, m.data_live.size(), cudaMemcpyDeviceToHost),
                     err, "d2h live"))
            return fail(ST_UNSUPPORTED_OP);
    }

    m.mem_size = st.mem_size;

    cudaFree(d_code);
    cudaFree(d_consts);
    cudaFree(d_funcs);
    cudaFree(d_stack);
    cudaFree(d_frames);
    cudaFree(d_st);
    cudaFree(d_globals);
    cudaFree(d_mem);
    cudaFree(d_blob);
    cudaFree(d_doff);
    cudaFree(d_dlen);
    cudaFree(d_live);
    cudaFree(d_table);
    cudaFree(d_tidx);
    cudaFree(d_tfp);
    cudaFree(d_mb);

    r.status = st.status;
    r.peak_csp = st.peak_csp;
    r.steps_bound = max_steps;
    if (st.status == ST_OK) {
        for (uint16_t i = 0; i < f.n_results; ++i)
            r.results.push_back(h_stack[i]);
    }
    return r;
}

} // namespace cuwasm

#ifdef CUWASM_GPU_MAIN
int main(int argc, char** argv) {
    if (argc < 3) {
        std::fprintf(stderr, "usage: cuwasm-run-gpu <module.wasm> <export> [i64 args...]\n");
        return 2;
    }
    std::string err;
    std::vector<uint8_t> wasm;
    if (!cuwasm::load_file(argv[1], wasm, err)) {
        std::printf("{\"status\": \"unsupported_op\", \"results\": []}\n");
        return 1;
    }
    cuwasm::HostModule m;
    if (!cuwasm::translate_wasm(wasm.data(), wasm.size(), m, err) || !cuwasm::verify_cuop(m, err)) {
        std::printf("{\"status\": \"unsupported_op\", \"results\": []}\n");
        return 1;
    }
    int fi = m.find_export(argv[2]);
    if (fi < 0) {
        std::printf("{\"status\": \"unsupported_op\", \"results\": []}\n");
        return 1;
    }
    std::vector<uint64_t> args;
    for (int i = 3; i < argc; ++i)
        args.push_back((uint64_t)std::strtoll(argv[i], nullptr, 10));
    auto r = cuwasm::run_gpu(m, (uint32_t)fi, args.data(), (uint32_t)args.size());
    std::printf("{\"status\": \"%s\", \"results\": [", cuwasm::status_name(r.status));
    for (size_t i = 0; i < r.results.size(); ++i) {
        if (i)
            std::printf(", ");
        std::printf("%lld", (long long)(int64_t)r.results[i]);
    }
    std::printf("]}\n");
    return r.status == cuwasm::ST_OK ? 0 : 1;
}
#endif
