#include "cuwasm/host.h"

#include <algorithm>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <map>
#include <sstream>
#include <string>
#include <vector>

using cuwasm::HostModule;
using cuwasm::ST_OK;
using cuwasm::ST_UNSUPPORTED_OP;

static std::string json_raw_string(const std::string& line, const std::string& key) {
    std::string pat = "\"" + key + "\":";
    auto p = line.find(pat);
    if (p == std::string::npos)
        return {};
    p += pat.size();
    while (p < line.size() && (line[p] == ' '))
        ++p;
    if (p >= line.size())
        return {};
    if (line[p] == 'n' && line.compare(p, 4, "null") == 0)
        return {};
    if (line[p] != '"')
        return {};
    ++p;
    std::string out;
    while (p < line.size()) {
        char c = line[p++];
        if (c == '\\' && p < line.size()) {
            char n = line[p++];
            if (n == 'n')
                out += '\n';
            else if (n == 't')
                out += '\t';
            else
                out += n;
            continue;
        }
        if (c == '"')
            break;
        out += c;
    }
    return out;
}

static std::vector<std::string> json_str_array(const std::string& line, const std::string& key) {
    std::string pat = "\"" + key + "\":";
    auto p = line.find(pat);
    std::vector<std::string> out;
    if (p == std::string::npos)
        return out;
    p = line.find('[', p);
    if (p == std::string::npos)
        return out;
    ++p;
    while (p < line.size()) {
        while (p < line.size() && (line[p] == ' ' || line[p] == ',' || line[p] == '\n'))
            ++p;
        if (p >= line.size() || line[p] == ']')
            break;
        if (line[p] != '"')
            break;
        ++p;
        std::string s;
        while (p < line.size() && line[p] != '"') {
            if (line[p] == '\\' && p + 1 < line.size()) {
                s += line[p + 1];
                p += 2;
            } else {
                s += line[p++];
            }
        }
        if (p < line.size() && line[p] == '"')
            ++p;
        out.push_back(s);
    }
    return out;
}

static bool results_match(const std::vector<uint64_t>& got, const std::vector<std::string>& exp,
                          const std::vector<std::string>& ty) {
    if (got.size() != exp.size() || exp.size() != ty.size())
        return false;
    for (size_t i = 0; i < got.size(); ++i) {
        uint64_t want = std::stoull(exp[i]);
        if (ty[i] == "i32") {
            if ((uint32_t)got[i] != (uint32_t)want)
                return false;
        } else {
            if (got[i] != want)
                return false;
        }
    }
    return true;
}

int main(int argc, char** argv) {
    if (argc < 3) {
        std::cerr << "usage: test_suite <catalog.jsonl> <suite-dir>\n";
        return 2;
    }
    std::string catalog_path = argv[1];
    std::string suite_dir = argv[2];
    std::ifstream in(catalog_path);
    if (!in) {
        std::cerr << "cannot open " << catalog_path << "\n";
        return 1;
    }

    std::map<std::string, HostModule> mods;
    std::map<std::string, std::string> mod_err;

    int n_pass = 0, n_fail = 0, n_unsup = 0, n_skip = 0, n_trap_ok = 0, n_trap_fail = 0;
    std::map<std::string, int> fail_by_file;
    std::map<std::string, int> unsup_reason;
    std::map<std::string, int> unsup_by_file;

    std::string line;
    int n_cases = 0;
    while (std::getline(in, line)) {
        if (line.empty())
            continue;
        ++n_cases;
        std::string kind = json_raw_string(line, "kind");
        std::string file = json_raw_string(line, "file");
        std::string skip = json_raw_string(line, "skip");
        if (kind == "skip" || !skip.empty()) {
            ++n_skip;
            continue;
        }
        if (kind == "unlinkable") {
            std::string wasm_rel = json_raw_string(line, "wasm");
            std::vector<uint8_t> bytes;
            std::string err;
            std::string path = suite_dir + "/" + wasm_rel;
            HostModule m;
            bool linked = cuwasm::load_file(path, bytes, err) &&
                          cuwasm::translate_wasm(bytes.data(), bytes.size(), m, err) &&
                          cuwasm::verify_cuop(m, err);
            if (linked) {
                ++n_fail;
                fail_by_file[file]++;
                if (n_fail <= 15)
                    std::cerr << "FAIL unlinkable linked " << file << "\n";
            } else if (err.find("out of bounds") != std::string::npos ||
                       err.find("unlinkable") != std::string::npos) {
                ++n_trap_ok;
            } else {
                ++n_unsup;
                if (err.size() > 60)
                    err = err.substr(0, 60);
                unsup_reason[err]++;
                unsup_by_file[file]++;
            }
            continue;
        }
        std::string wasm_rel = json_raw_string(line, "wasm");
        std::string expname = json_raw_string(line, "export");
        auto args_s = json_str_array(line, "args");
        auto arg_ty = json_str_array(line, "arg_ty");
        auto exp_s = json_str_array(line, "expected");
        auto exp_ty = json_str_array(line, "exp_ty");

        if (mods.find(wasm_rel) == mods.end() && mod_err.find(wasm_rel) == mod_err.end()) {
            std::vector<uint8_t> bytes;
            std::string err;
            std::string path = suite_dir + "/" + wasm_rel;
            if (!cuwasm::load_file(path, bytes, err)) {
                mod_err[wasm_rel] = err;
            } else {
                HostModule m;
                if (!cuwasm::translate_wasm(bytes.data(), bytes.size(), m, err)) {
                    mod_err[wasm_rel] = err;
                } else if (!cuwasm::verify_cuop(m, err)) {
                    mod_err[wasm_rel] = "verify: " + err;
                } else {
                    mods[wasm_rel] = std::move(m);
                }
            }
        }
        if (mod_err.count(wasm_rel)) {
            ++n_unsup;
            std::string r = mod_err[wasm_rel];
            if (r.size() > 60)
                r = r.substr(0, 60);
            unsup_reason[r]++;
            unsup_by_file[file]++;
            continue;
        }
        HostModule& m = mods[wasm_rel];
        int fi = m.find_export(expname);
        if (fi < 0) {
            ++n_unsup;
            unsup_reason["missing export"]++;
            unsup_by_file[file]++;
            continue;
        }
        std::vector<uint64_t> args;
        for (auto& s : args_s)
            args.push_back(std::stoull(s));
        auto r = cuwasm::run_cpu(m, (uint32_t)fi, args.data(), (uint32_t)args.size());
        if (r.status == ST_UNSUPPORTED_OP) {
            ++n_unsup;
            unsup_reason["runtime unsupported"]++;
            unsup_by_file[file]++;
            continue;
        }
        if (kind == "invoke") {
            if (r.status != ST_OK) {
                ++n_fail;
                fail_by_file[file]++;
                if (n_fail <= 15)
                    std::cerr << "FAIL invoke " << file << " " << expname
                              << " status=" << cuwasm::status_name(r.status) << "\n";
            }
            continue;
        }
        if (kind == "trap") {
            if (r.status != ST_OK && r.status != cuwasm::ST_RUNNING) {
                ++n_trap_ok;
            } else {
                ++n_trap_fail;
                fail_by_file[file]++;
                if (n_trap_fail <= 8)
                    std::cerr << "TRAP_FAIL " << file << " " << expname
                              << " status=" << cuwasm::status_name(r.status) << "\n";
            }
            continue;
        }
        if (r.status != ST_OK) {
            ++n_fail;
            fail_by_file[file]++;
            if (n_fail <= 15)
                std::cerr << "FAIL " << file << " " << expname
                          << " status=" << cuwasm::status_name(r.status) << "\n";
            continue;
        }
        if (!results_match(r.results, exp_s, exp_ty)) {
            ++n_fail;
            fail_by_file[file]++;
            if (n_fail <= 15) {
                std::cerr << "FAIL " << file << " " << expname << " got";
                for (auto v : r.results)
                    std::cerr << " " << (int64_t)v;
                std::cerr << "\n";
            }
            continue;
        }
        ++n_pass;
    }

    std::cerr << "suite cases=" << n_cases << " pass=" << n_pass << " fail=" << n_fail
              << " trap_ok=" << n_trap_ok << " trap_fail=" << n_trap_fail
              << " unsupported=" << n_unsup << " skip=" << n_skip << "\n";
    std::cerr << "score " << (n_pass + n_trap_ok) << "\n";
    if (!unsup_reason.empty()) {
        std::cerr << "unsupported reasons (top):\n";
        std::vector<std::pair<int, std::string>> rs;
        for (auto& kv : unsup_reason)
            rs.push_back({kv.second, kv.first});
        std::sort(rs.begin(), rs.end(), [](auto& a, auto& b) { return a.first > b.first; });
        for (size_t i = 0; i < rs.size() && i < 12; ++i)
            std::cerr << "  " << rs[i].first << "  " << rs[i].second << "\n";
    }
    if (!unsup_by_file.empty()) {
        std::cerr << "unsupported by file (top):\n";
        std::vector<std::pair<int, std::string>> rs;
        for (auto& kv : unsup_by_file)
            rs.push_back({kv.second, kv.first});
        std::sort(rs.begin(), rs.end(), [](auto& a, auto& b) { return a.first > b.first; });
        for (size_t i = 0; i < rs.size() && i < 12; ++i)
            std::cerr << "  " << rs[i].first << "  " << rs[i].second << "\n";
    }
    if (!fail_by_file.empty()) {
        std::cerr << "fails by file (top):\n";
        std::vector<std::pair<int, std::string>> rs;
        for (auto& kv : fail_by_file)
            rs.push_back({kv.second, kv.first});
        std::sort(rs.begin(), rs.end(), [](auto& a, auto& b) { return a.first > b.first; });
        for (size_t i = 0; i < rs.size() && i < 12; ++i)
            std::cerr << "  " << rs[i].first << "  " << rs[i].second << "\n";
    }
    std::cout << n_pass + n_trap_ok << "\n";
    return n_fail || n_trap_fail ? 1 : 0;
}
