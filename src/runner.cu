#include "cuwasm/gpu.h"
#include "cuwasm/host.h"

#include <cuda_runtime.h>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

namespace cuwasm {

__global__ void k_run(DevModule m, VmState* st, uint64_t* stack, Frame* frames,
                      uint64_t* globals, uint32_t n_globals, uint32_t stack_cap, uint32_t frame_cap,
                      uint64_t max_steps) {
    if (blockIdx.x != 0 || threadIdx.x != 0)
        return;
    AoSView sv{stack, stack_cap, 0};
    AoSFrameView fv{frames, frame_cap, 0};
    run_instance(m, *st, sv, fv, globals, n_globals, max_steps);
}

static bool cuda_ok(cudaError_t e, std::string& err, const char* what) {
    if (e == cudaSuccess)
        return true;
    err = std::string(what) + ": " + cudaGetErrorString(e);
    return false;
}

RunResult run_gpu(const HostModule& m, uint32_t func_idx, const uint64_t* args, uint32_t n_args,
                  uint64_t max_steps) {
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

    auto fail = [&](uint16_t status) {
        r.status = status;
        cudaFree(d_code);
        cudaFree(d_consts);
        cudaFree(d_funcs);
        cudaFree(d_stack);
        cudaFree(d_frames);
        cudaFree(d_st);
        cudaFree(d_globals);
        return r;
    };

    if (m.code.empty() || !cuda_ok(cudaMalloc(&d_code, m.code.size() * sizeof(CuOp)), err, "code"))
        return fail(ST_UNSUPPORTED_OP);
    if (!m.consts.empty()) {
        if (!cuda_ok(cudaMalloc(&d_consts, m.consts.size() * sizeof(uint64_t)), err, "consts"))
            return fail(ST_UNSUPPORTED_OP);
    } else {
        if (!cuda_ok(cudaMalloc(&d_consts, sizeof(uint64_t)), err, "consts"))
            return fail(ST_UNSUPPORTED_OP);
    }
    if (!cuda_ok(cudaMalloc(&d_funcs, m.funcs.size() * sizeof(FuncMeta)), err, "funcs"))
        return fail(ST_UNSUPPORTED_OP);
    if (!cuda_ok(cudaMalloc(&d_stack, STACK_CAP * sizeof(uint64_t)), err, "stack"))
        return fail(ST_UNSUPPORTED_OP);
    if (!cuda_ok(cudaMalloc(&d_frames, FRAME_CAP * sizeof(Frame)), err, "frames"))
        return fail(ST_UNSUPPORTED_OP);
    if (!cuda_ok(cudaMalloc(&d_st, sizeof(VmState)), err, "state"))
        return fail(ST_UNSUPPORTED_OP);
    uint32_t n_globals = (uint32_t)m.globals.size();
    if (n_globals == 0) {
        if (!cuda_ok(cudaMalloc(&d_globals, sizeof(uint64_t)), err, "globals"))
            return fail(ST_UNSUPPORTED_OP);
    } else {
        if (!cuda_ok(cudaMalloc(&d_globals, n_globals * sizeof(uint64_t)), err, "globals"))
            return fail(ST_UNSUPPORTED_OP);
        if (!cuda_ok(cudaMemcpy(d_globals, m.globals.data(), n_globals * sizeof(uint64_t),
                                cudaMemcpyHostToDevice),
                     err, "h2d globals"))
            return fail(ST_UNSUPPORTED_OP);
    }

    if (!cuda_ok(cudaMemcpy(d_code, m.code.data(), m.code.size() * sizeof(CuOp), cudaMemcpyHostToDevice),
                 err, "h2d code"))
        return fail(ST_UNSUPPORTED_OP);
    if (!m.consts.empty()) {
        if (!cuda_ok(cudaMemcpy(d_consts, m.consts.data(), m.consts.size() * sizeof(uint64_t),
                                cudaMemcpyHostToDevice),
                     err, "h2d consts"))
            return fail(ST_UNSUPPORTED_OP);
    }
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
    if (!cuda_ok(cudaMemcpy(d_st, &st, sizeof(VmState), cudaMemcpyHostToDevice), err, "h2d st"))
        return fail(ST_UNSUPPORTED_OP);

    DevModule dm{};
    dm.code = d_code;
    dm.consts = d_consts;
    dm.funcs = d_funcs;
    dm.n_funcs = (uint32_t)m.funcs.size();
    dm.code_len = (uint32_t)m.code.size();

    k_run<<<1, 1>>>(dm, d_st, d_stack, d_frames, d_globals, n_globals, STACK_CAP, FRAME_CAP,
                    max_steps);
    if (!cuda_ok(cudaGetLastError(), err, "launch"))
        return fail(ST_UNSUPPORTED_OP);
    if (!cuda_ok(cudaDeviceSynchronize(), err, "sync"))
        return fail(ST_UNSUPPORTED_OP);

    if (!cuda_ok(cudaMemcpy(&st, d_st, sizeof(VmState), cudaMemcpyDeviceToHost), err, "d2h st"))
        return fail(ST_UNSUPPORTED_OP);
    if (!cuda_ok(cudaMemcpy(h_stack.data(), d_stack, STACK_CAP * sizeof(uint64_t), cudaMemcpyDeviceToHost),
                 err, "d2h stack"))
        return fail(ST_UNSUPPORTED_OP);

    cudaFree(d_code);
    cudaFree(d_consts);
    cudaFree(d_funcs);
    cudaFree(d_stack);
    cudaFree(d_frames);
    cudaFree(d_st);
    cudaFree(d_globals);

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
