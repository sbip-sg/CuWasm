#include "cuwasm/host.h"
#ifdef CUWASM_TEST_GPU
#include "cuwasm/gpu.h"
#endif

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <vector>

using cuwasm::Assertion;
using cuwasm::HostModule;
using cuwasm::RunResult;
using cuwasm::ST_OK;
using cuwasm::ST_UNSUPPORTED_OP;

static int g_fails = 0;
static int g_passes = 0;

static void expect(bool cond, const std::string& msg) {
    if (cond) {
        ++g_passes;
    } else {
        ++g_fails;
        std::cerr << "FAIL: " << msg << "\n";
    }
}

#ifndef CUWASM_TEST_GPU
static std::string slurp_cmd(const std::string& cmd) {
    std::string out;
    FILE* p = popen(cmd.c_str(), "r");
    if (!p)
        return {};
    char buf[4096];
    while (fgets(buf, sizeof(buf), p))
        out += buf;
    pclose(p);
    return out;
}

static bool json_status_ok(const std::string& json, int64_t* result) {
    if (json.find("\"status\": \"ok\"") == std::string::npos &&
        json.find("\"status\":\"ok\"") == std::string::npos)
        return false;
    auto r = json.find("\"results\"");
    if (r == std::string::npos)
        return false;
    auto b = json.find('[', r);
    if (b == std::string::npos)
        return false;
    *result = std::strtoll(json.c_str() + b + 1, nullptr, 10);
    return true;
}
#endif

static bool load_mod(int idx, const std::string& gen, HostModule& m, std::string& err) {
    std::vector<uint8_t> wasm;
    if (!cuwasm::load_file(gen + "/mod" + std::to_string(idx) + ".wasm", wasm, err))
        return false;
    if (!cuwasm::translate_wasm(wasm.data(), wasm.size(), m, err))
        return false;
    if (!cuwasm::verify_cuop(m, err))
        return false;
    return true;
}

#ifdef CUWASM_TEST_GPU
static RunResult run_one(const HostModule& m, uint32_t fi, uint64_t arg) {
    return cuwasm::run_gpu(m, fi, &arg, 1);
}
#else
static RunResult run_one(const HostModule& m, uint32_t fi, uint64_t arg) {
    return cuwasm::run_cpu(m, fi, &arg, 1);
}
#endif

static int test_unsupported_stub() {
    HostModule m;
    m.consts.push_back(1);
    cuwasm::FuncMeta f{};
    f.code_off = 0;
    f.code_len = 1;
    f.n_params = 0;
    f.n_results = 0;
    f.n_locals = 0;
    m.funcs.push_back(f);
    cuwasm::CuOp bad{};
    bad.op = 0xFFFF;
    m.code.push_back(bad);
    auto r = cuwasm::run_cpu(m, 0, nullptr, 0);
    expect(r.status == ST_UNSUPPORTED_OP, "stub/unknown op -> ST_UNSUPPORTED_OP");
    return 0;
}

static int test_parse_assertions(const std::string& wast) {
    std::string err;
    std::vector<Assertion> a;
    expect(cuwasm::parse_wast_assertions(wast, a, err), "parse wast: " + err);
    expect((int)a.size() == 60, "expected 60 assertions, got " + std::to_string(a.size()));
    return 0;
}

#ifndef CUWASM_TEST_GPU
static int test_oracle(const std::string& oracle, const std::string& gen, const std::string& wast) {
    std::string err;
    std::vector<Assertion> asserts;
    if (!cuwasm::parse_wast_assertions(wast, asserts, err)) {
        expect(false, "oracle: parse " + err);
        return 1;
    }
    int n = 0;
    for (const auto& a : asserts) {
        std::string cmd = "timeout 10 " + oracle + " " + gen + "/mod" + std::to_string(a.module_index) +
                          ".wasm " + a.export_name;
        for (int64_t v : a.args)
            cmd += " " + std::to_string(v);
        std::string json = slurp_cmd(cmd);
        int64_t got = 0;
        bool ok = json_status_ok(json, &got) && got == a.expected[0];
        expect(ok, "oracle " + a.export_name + "(" + std::to_string(a.args[0]) + ") json=" + json);
        if (ok)
            ++n;
    }
    expect(n == 60, "oracle 60/60, got " + std::to_string(n));
    return 0;
}
#endif

static int test_lowering_and_verify(const std::string& gen) {
    std::string err;
    for (int i = 0; i < 3; ++i) {
        HostModule m;
        expect(load_mod(i, gen, m, err), "lower+verify mod" + std::to_string(i) + ": " + err);
        bool any_bad = false;
        for (const auto& op : m.code) {
            if (op.op > cuwasm::OP_END_FUNC)
                any_bad = true;
        }
        expect(!any_bad, "mod" + std::to_string(i) + " has unsupported cuop");
        std::string d = cuwasm::disasm(m);
        expect(!d.empty(), "disasm empty");
        if (i == 0 && std::getenv("CUWASM_DUMP"))
            std::cerr << d;
    }

    HostModule bad;
    expect(load_mod(0, gen, bad, err), "reload mod0 for corrupt: " + err);
    bool found = false;
    for (auto& op : bad.code) {
        if (op.op == cuwasm::OP_BR || op.op == cuwasm::OP_BR_IF_NOT) {
            op.b = 0xFFFFFFFFu;
            found = true;
            break;
        }
    }
    expect(found, "need a branch to corrupt");
    std::string verr;
    expect(!cuwasm::verify_cuop(bad, verr), "verify should fail on corrupted branch");
    expect(!verr.empty(), "verify error message");
    return 0;
}

static int test_fib(const std::string& gen, const std::string& wast, const char* only_export) {
    std::string err;
    std::vector<Assertion> asserts;
    if (!cuwasm::parse_wast_assertions(wast, asserts, err)) {
        expect(false, "fib parse " + err);
        return 1;
    }
    HostModule mods[3];
    for (int i = 0; i < 3; ++i)
        expect(load_mod(i, gen, mods[i], err), "load mod" + std::to_string(i) + " " + err);

    int n = 0;
    int want = 0;
    for (const auto& a : asserts) {
        if (only_export && a.export_name != only_export)
            continue;
        ++want;
        int fi = mods[a.module_index].find_export(a.export_name);
        expect(fi >= 0, "missing export " + a.export_name);
        uint64_t arg = (uint64_t)a.args[0];
        auto r = run_one(mods[a.module_index], (uint32_t)fi, arg);
        bool ok = r.status == ST_OK && r.results.size() == 1 &&
                  (int64_t)r.results[0] == a.expected[0];
        expect(ok, a.export_name + "(" + std::to_string(a.args[0]) + ") status=" +
                       cuwasm::status_name(r.status) + " got=" +
                       (r.results.empty() ? "?" : std::to_string((int64_t)r.results[0])) +
                       " want=" + std::to_string(a.expected[0]));
        if (ok)
            ++n;
    }
    expect(n == want, std::string(only_export ? only_export : "all") + " " + std::to_string(n) +
                          "/" + std::to_string(want));
    return 0;
}

static int test_tail_depth(const std::string& gen) {
    std::string err;
    HostModule m;
    expect(load_mod(2, gen, m, err), "tail module: " + err);
    int fi = m.find_export("fibonacci-tail");
    expect(fi >= 0, "fibonacci-tail export");
    uint64_t arg = 19;
    auto r = run_one(m, (uint32_t)fi, arg);
    expect(r.status == ST_OK && r.results.size() == 1 && (int64_t)r.results[0] == 4181,
           "tail(19) value");
    expect(r.peak_csp <= 2, "tail peak_csp=" + std::to_string(r.peak_csp) + " expected <= 2");
    return 0;
}

int main(int argc, char** argv) {
    std::string wast = "tests/fibonacci.wast";
    std::string gen = "build/gen";
    std::string oracle = "build/cuwasm-oracle";
    std::string which = "all";
    for (int i = 1; i < argc; ++i) {
        std::string a = argv[i];
        if (a == "--wast" && i + 1 < argc)
            wast = argv[++i];
        else if (a == "--gen" && i + 1 < argc)
            gen = argv[++i];
        else if (a == "--oracle" && i + 1 < argc)
            oracle = argv[++i];
        else if (a == "--t0")
            which = "t0";
        else if (a == "--t1")
            which = "t1";
        else if (a == "--t2" || a == "--t3")
            which = "t23";
        else if (a == "--t4")
            which = "t4";
        else if (a == "--t5")
            which = "t5";
        else if (a == "--t6")
            which = "t6";
        else if (a == "--t7")
            which = "t7";
        else if (a == "--t8" || a == "--gpu")
            which = "t8";
        else if (a == "--cpu")
            which = "cpu";
        else if (a == "--all")
            which = "all";
    }

    if (which == "t0" || which == "all" || which == "cpu")
        test_unsupported_stub();
    if (which == "t1" || which == "all" || which == "cpu") {
        test_parse_assertions(wast);
#ifndef CUWASM_TEST_GPU
        test_oracle(oracle, gen, wast);
#endif
    }
    if (which == "t23" || which == "all" || which == "cpu" || which == "t7" || which == "t8")
        test_lowering_and_verify(gen);
    if (which == "t4" || which == "all" || which == "cpu" || which == "t7" || which == "t8")
        test_fib(gen, wast, "fibonacci-iter");
    if (which == "t5" || which == "all" || which == "cpu" || which == "t7" || which == "t8")
        test_fib(gen, wast, "fibonacci-rec");
    if (which == "t6" || which == "all" || which == "cpu" || which == "t7" || which == "t8") {
        test_fib(gen, wast, "fibonacci-tail");
        test_tail_depth(gen);
    }
    if (which == "t7" || which == "all" || which == "cpu")
        test_fib(gen, wast, nullptr);
    if (which == "t8")
        test_fib(gen, wast, nullptr);

    std::cerr << "passed=" << g_passes << " failed=" << g_fails << "\n";
    return g_fails ? 1 : 0;
}
