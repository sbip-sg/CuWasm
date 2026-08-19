#pragma once

#include "cuop.h"
#include "interp.h"
#include "layout.h"
#include "vmstate.h"

#include <cstdint>
#include <string>
#include <utility>
#include <vector>

namespace cuwasm {

struct HostModule {
    std::vector<CuOp> code;
    std::vector<uint64_t> consts;
    std::vector<FuncMeta> funcs;
    std::vector<std::pair<std::string, uint32_t>> exports;
    std::vector<uint64_t> globals;
    std::vector<uint8_t> memory;
    uint32_t mem_size = 0;
    std::vector<uint8_t> data_blob;
    std::vector<uint32_t> data_off;
    std::vector<uint32_t> data_len;
    std::vector<uint8_t> data_live;
    std::vector<uint32_t> table;
    std::vector<uint32_t> func_typeidx;
    std::vector<uint64_t> type_fp;
    uint32_t n_host_imports = 0;
    std::vector<uint32_t> host_fn_id;
    std::vector<std::string> host_import_mod;
    std::vector<std::string> host_import_name;
    std::vector<std::string> host_import_env;

    DevModule dev() const {
        DevModule d{};
        d.code = code.data();
        d.consts = consts.data();
        d.funcs = funcs.data();
        d.n_funcs = (uint32_t)funcs.size();
        d.code_len = (uint32_t)code.size();
        d.table = table.empty() ? nullptr : table.data();
        d.table_len = (uint32_t)table.size();
        d.func_typeidx = func_typeidx.empty() ? nullptr : func_typeidx.data();
        d.type_fp = type_fp.empty() ? nullptr : type_fp.data();
        d.n_types = (uint32_t)type_fp.size();
        return d;
    }

    int find_export(const std::string& name) const {
        for (const auto& e : exports) {
            if (e.first == name)
                return (int)e.second;
        }
        return -1;
    }
};

struct RunResult {
    uint16_t status = ST_UNSUPPORTED_OP;
    std::vector<uint64_t> results;
    uint32_t peak_csp = 0;
    uint64_t steps_bound = 0;
    std::string error;
};


struct HostCallContext {
    const HostModule* module = nullptr;
};

/// Return true if the host call was handled. On false, `err` describes the failure.
using HostFn = bool (*)(HostCallContext& ctx, HostMailbox& mb, std::string& err);

bool default_host_fn(HostCallContext& ctx, HostMailbox& mb, std::string& err);

bool translate_wasm(const uint8_t* data, size_t len, HostModule& out, std::string& err);
bool verify_cuop(HostModule& m, std::string& err);
std::string disasm(const HostModule& m);

RunResult run_cpu(HostModule& m, uint32_t func_idx, const uint64_t* args,
                  uint32_t n_args, uint64_t max_steps = DEFAULT_MAX_STEPS,
                  HostFn host_fn = default_host_fn, RunProfile* profile = nullptr);

bool load_file(const std::string& path, std::vector<uint8_t>& out, std::string& err);

int count_assert_returns(const std::string& wast_path, std::string& err);

struct Assertion {
    int module_index = 0;
    std::string export_name;
    std::vector<int64_t> args;
    std::vector<int64_t> expected;
};

bool parse_wast_assertions(const std::string& wast_path, std::vector<Assertion>& out,
                           std::string& err);

} // namespace cuwasm
