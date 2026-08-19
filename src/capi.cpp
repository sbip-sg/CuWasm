#include "cuwasm/capi.h"
#include "cuwasm/host.h"

#include <cstring>
#include <new>
#include <string>

struct CuwasmModule {
    cuwasm::HostModule m;
};

struct RunCtx {
    CuwasmHostFn fn = nullptr;
    void* ctx = nullptr;
    cuwasm::HostModule* module = nullptr;
};

static thread_local RunCtx* g_run_ctx = nullptr;

static void copy_err(char* err, size_t cap, const std::string& msg) {
    if (!err || cap == 0)
        return;
    std::strncpy(err, msg.c_str(), cap - 1);
    err[cap - 1] = '\0';
}

static bool capi_host_bridge(cuwasm::HostCallContext& ctx, cuwasm::HostMailbox& mb,
                             std::string& err) {
    if (g_run_ctx && g_run_ctx->fn) {
        CuwasmMailbox cmb{};
        cmb.fn_id = mb.fn_id;
        cmb.n_args = mb.n_args;
        cmb.n_results = mb.n_results;
        for (uint16_t i = 0; i < mb.n_args && i < 16; ++i)
            cmb.args[i] = mb.args[i];
        char buf[256]{};
        if (!g_run_ctx->fn(g_run_ctx->ctx, &cmb, buf, sizeof(buf))) {
            err = buf;
            return false;
        }
        mb.n_results = cmb.n_results;
        for (uint16_t i = 0; i < cmb.n_results && i < 1; ++i)
            mb.results[i] = cmb.results[i];
        return true;
    }
    ctx.module = g_run_ctx ? g_run_ctx->module : ctx.module;
    return cuwasm::default_host_fn(ctx, mb, err);
}

extern "C" CuwasmModule* cuwasm_module_load(const uint8_t* wasm, size_t len, char* err,
                                            size_t err_cap) {
    if (!wasm && len != 0) {
        copy_err(err, err_cap, "null wasm");
        return nullptr;
    }
    auto* out = new (std::nothrow) CuwasmModule();
    if (!out) {
        copy_err(err, err_cap, "oom");
        return nullptr;
    }
    std::string e;
    if (!cuwasm::translate_wasm(wasm, len, out->m, e)) {
        copy_err(err, err_cap, e);
        delete out;
        return nullptr;
    }
    if (!cuwasm::verify_cuop(out->m, e)) {
        copy_err(err, err_cap, e);
        delete out;
        return nullptr;
    }
    return out;
}

extern "C" void cuwasm_module_free(CuwasmModule* m) {
    delete m;
}

extern "C" int cuwasm_module_export_index(CuwasmModule* m, const char* name) {
    if (!m || !name)
        return -1;
    return m->m.find_export(name);
}

extern "C" uint8_t* cuwasm_module_memory(CuwasmModule* m) {
    if (!m || m->m.memory.empty())
        return nullptr;
    return m->m.memory.data();
}

extern "C" uint32_t cuwasm_module_memory_size(CuwasmModule* m) {
    if (!m)
        return 0;
    return m->m.mem_size;
}

extern "C" int cuwasm_module_run(CuwasmModule* m, uint32_t func_idx, const uint64_t* args,
                                 uint32_t n_args, uint64_t max_steps, CuwasmHostFn host, void* ctx,
                                 CuwasmRunResult* out) {
    if (!m || !out)
        return -1;
    RunCtx rc{host, ctx, &m->m};
    g_run_ctx = &rc;
    auto r = cuwasm::run_cpu(m->m, func_idx, args, n_args, max_steps, capi_host_bridge);
    g_run_ctx = nullptr;
    out->status = r.status;
    out->n_results = (uint32_t)r.results.size();
    for (size_t i = 0; i < r.results.size() && i < 8; ++i)
        out->results[i] = r.results[i];
    copy_err(out->error, sizeof(out->error), r.error);
    return r.status == cuwasm::ST_OK ? 0 : 1;
}
