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
    uint8_t* memory;
    uint32_t mem_size;
    uint32_t mem_max;
    uint8_t* data_blob;
    uint32_t data_blob_len;
    uint32_t* data_off;
    uint32_t* data_len;
    uint8_t* data_live;
    uint32_t n_data;
    uint32_t* table;
    uint32_t table_len;
    uint32_t* func_typeidx;
    uint64_t* type_fp;
    uint32_t n_types;
    uint32_t n_host_imports;
    uint32_t* host_fn_id;
    char** host_import_mod;
    char** host_import_name;
    char** host_import_env;
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
    out.mem_size = t.mem_size;
    if (t.mem_max && t.memory)
        out.memory.assign(t.memory, t.memory + t.mem_max);
    if (t.data_blob_len && t.data_blob)
        out.data_blob.assign(t.data_blob, t.data_blob + t.data_blob_len);
    if (t.n_data) {
        if (t.data_off)
            out.data_off.assign(t.data_off, t.data_off + t.n_data);
        if (t.data_len)
            out.data_len.assign(t.data_len, t.data_len + t.n_data);
        if (t.data_live)
            out.data_live.assign(t.data_live, t.data_live + t.n_data);
    }
    if (t.table_len && t.table)
        out.table.assign(t.table, t.table + t.table_len);
    if (t.n_funcs && t.func_typeidx)
        out.func_typeidx.assign(t.func_typeidx, t.func_typeidx + t.n_funcs);
    if (t.n_types && t.type_fp)
        out.type_fp.assign(t.type_fp, t.type_fp + t.n_types);
    out.n_host_imports = t.n_host_imports;
    if (t.n_host_imports && t.host_fn_id)
        out.host_fn_id.assign(t.host_fn_id, t.host_fn_id + t.n_host_imports);
    out.host_import_mod.clear();
    out.host_import_name.clear();
    out.host_import_env.clear();
    for (uint32_t i = 0; i < t.n_host_imports; ++i) {
        out.host_import_mod.push_back(t.host_import_mod && t.host_import_mod[i] ? t.host_import_mod[i]
                                                                                : "");
        out.host_import_name.push_back(t.host_import_name && t.host_import_name[i]
                                           ? t.host_import_name[i]
                                           : "");
        out.host_import_env.push_back(t.host_import_env && t.host_import_env[i] ? t.host_import_env[i]
                                                                                : "");
    }
    cuwasm_translate_free(&t);
    return true;
}

} // namespace cuwasm
