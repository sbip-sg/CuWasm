#include "cuwasm/host.h"

#include <cstring>

extern "C" {
struct CuOpC {
    uint16_t op;
    uint16_t a;
    uint32_t b;
};
struct FuncMetaC {
    uint32_t code_off, code_len;
    uint16_t n_params, n_results, n_locals, max_stack;
};
struct TranslateOut {
    CuOpC* code;
    uint32_t n_code;
    uint64_t* consts;
    uint32_t n_consts;
    FuncMetaC* funcs;
    uint32_t n_funcs;
    char** export_names;
    uint32_t* export_idxs;
    uint32_t n_exports;
    uint64_t* globals;
    uint32_t n_globals;
    char* err;
};
int cuwasm_translate_wasm(const uint8_t* data, size_t len, TranslateOut* out);
void cuwasm_translate_free(TranslateOut* out);
}

namespace cuwasm {

bool translate_wasm(const uint8_t* data, size_t len, HostModule& out, std::string& err) {
    TranslateOut t{};
    if (cuwasm_translate_wasm(data, len, &t) != 0) {
        err = t.err ? t.err : "translate failed";
        cuwasm_translate_free(&t);
        return false;
    }
    out = HostModule{};
    out.code.resize(t.n_code);
    for (uint32_t i = 0; i < t.n_code; ++i) {
        out.code[i].op = t.code[i].op;
        out.code[i].a = t.code[i].a;
        out.code[i].b = t.code[i].b;
    }
    out.consts.assign(t.consts, t.consts + t.n_consts);
    out.funcs.resize(t.n_funcs);
    for (uint32_t i = 0; i < t.n_funcs; ++i) {
        out.funcs[i].code_off = t.funcs[i].code_off;
        out.funcs[i].code_len = t.funcs[i].code_len;
        out.funcs[i].n_params = t.funcs[i].n_params;
        out.funcs[i].n_results = t.funcs[i].n_results;
        out.funcs[i].n_locals = t.funcs[i].n_locals;
        out.funcs[i].max_stack = t.funcs[i].max_stack;
    }
    for (uint32_t i = 0; i < t.n_exports; ++i) {
        out.exports.emplace_back(t.export_names[i] ? t.export_names[i] : "", t.export_idxs[i]);
    }
    if (t.n_globals && t.globals)
        out.globals.assign(t.globals, t.globals + t.n_globals);
    cuwasm_translate_free(&t);
    return true;
}

} // namespace cuwasm
