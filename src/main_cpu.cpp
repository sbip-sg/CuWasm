#include "cuwasm/host.h"

#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <string>
#include <vector>

static int usage() {
    std::cerr << "usage: cuwasm-run <module.wasm> <export> [i64 args...]\n";
    return 2;
}

int main(int argc, char** argv) {
    if (argc < 3)
        return usage();
    std::string err;
    std::vector<uint8_t> wasm;
    if (!cuwasm::load_file(argv[1], wasm, err)) {
        std::printf("{\"status\": \"unsupported_op\", \"results\": [], \"error\": \"%s\"}\n",
                    err.c_str());
        return 1;
    }
    cuwasm::HostModule m;
    if (!cuwasm::translate_wasm(wasm.data(), wasm.size(), m, err)) {
        std::printf("{\"status\": \"unsupported_op\", \"results\": [], \"error\": \"%s\"}\n",
                    err.c_str());
        return 1;
    }
    if (!cuwasm::verify_cuop(m, err)) {
        std::printf("{\"status\": \"unsupported_op\", \"results\": [], \"error\": \"%s\"}\n",
                    err.c_str());
        return 1;
    }
    int fi = m.find_export(argv[2]);
    if (fi < 0) {
        std::printf("{\"status\": \"unsupported_op\", \"results\": [], \"error\": \"missing export\"}\n");
        return 1;
    }
    std::vector<uint64_t> args;
    for (int i = 3; i < argc; ++i) {
        args.push_back((uint64_t)std::strtoll(argv[i], nullptr, 10));
    }
    auto r = cuwasm::run_cpu(m, (uint32_t)fi, args.data(), (uint32_t)args.size());
    std::printf("{\"status\": \"%s\", \"results\": [", cuwasm::status_name(r.status));
    for (size_t i = 0; i < r.results.size(); ++i) {
        if (i)
            std::printf(", ");
        std::printf("%lld", (long long)(int64_t)r.results[i]);
    }
    std::printf("]}\n");
    return r.status == cuwasm::ST_OK ? 0 : 1;
}
