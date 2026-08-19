#include "cuwasm/host.h"

#include <cctype>
#include <fstream>
#include <sstream>

namespace cuwasm {

bool load_file(const std::string& path, std::vector<uint8_t>& out, std::string& err) {
    std::ifstream f(path, std::ios::binary);
    if (!f) {
        err = "cannot open " + path;
        return false;
    }
    f.seekg(0, std::ios::end);
    std::streamoff n = f.tellg();
    f.seekg(0, std::ios::beg);
    if (n < 0) {
        err = "tellg failed";
        return false;
    }
    out.resize((size_t)n);
    if (n > 0)
        f.read(reinterpret_cast<char*>(out.data()), n);
    return true;
}

int count_assert_returns(const std::string& wast_path, std::string& err) {
    std::vector<Assertion> a;
    if (!parse_wast_assertions(wast_path, a, err))
        return -1;
    return (int)a.size();
}

static bool parse_int64_token(const std::string& s, size_t& i, int64_t& v) {
    while (i < s.size() && std::isspace((unsigned char)s[i]))
        ++i;
    if (i >= s.size())
        return false;
    size_t start = i;
    if (s[i] == '+' || s[i] == '-')
        ++i;
    if (i >= s.size() || !std::isdigit((unsigned char)s[i]))
        return false;
    while (i < s.size() && std::isdigit((unsigned char)s[i]))
        ++i;
    try {
        v = std::stoll(s.substr(start, i - start));
    } catch (...) {
        return false;
    }
    return true;
}

bool parse_wast_assertions(const std::string& wast_path, std::vector<Assertion>& out, std::string& err) {
    std::ifstream f(wast_path);
    if (!f) {
        err = "cannot open " + wast_path;
        return false;
    }
    std::ostringstream ss;
    ss << f.rdbuf();
    const std::string s = ss.str();

    out.clear();
    int module = 0;
    bool seen_module = false;
    for (size_t pos = 0; pos < s.size();) {
        if (s.compare(pos, 7, "(module") == 0) {
            if (seen_module)
                ++module;
            seen_module = true;
            pos += 7;
            continue;
        }
        if (s.compare(pos, 14, "(assert_return") == 0) {
            size_t start = pos;
            int depth = 0;
            size_t j = start;
            for (; j < s.size(); ++j) {
                if (s[j] == '(')
                    ++depth;
                else if (s[j] == ')') {
                    --depth;
                    if (depth == 0) {
                        ++j;
                        break;
                    }
                }
            }
            std::string chunk = s.substr(start, j - start);
            auto inv = chunk.find("invoke \"");
            if (inv == std::string::npos) {
                err = "assert_return missing invoke";
                return false;
            }
            size_t n0 = inv + 8;
            auto n1 = chunk.find('"', n0);
            if (n1 == std::string::npos) {
                err = "unterminated export name";
                return false;
            }
            Assertion a;
            a.module_index = module;
            a.export_name = chunk.substr(n0, n1 - n0);
            std::vector<int64_t> nums;
            size_t k = 0;
            while (true) {
                auto p = chunk.find("i64.const", k);
                if (p == std::string::npos)
                    break;
                size_t t = p + 9;
                int64_t v = 0;
                if (!parse_int64_token(chunk, t, v)) {
                    err = "bad i64.const";
                    return false;
                }
                nums.push_back(v);
                k = t;
            }
            if (nums.size() < 2) {
                err = "assert_return expected arg and result";
                return false;
            }
            a.expected.push_back(nums.back());
            a.args.assign(nums.begin(), nums.end() - 1);
            out.push_back(std::move(a));
            pos = j;
            continue;
        }
        ++pos;
    }
    return true;
}

RunResult run_cpu(HostModule& m, uint32_t func_idx, const uint64_t* args, uint32_t n_args,
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

    std::vector<uint64_t> stack(STACK_CAP, 0);
    std::vector<Frame> frames(FRAME_CAP);

    VmState st{};
    st.pc = f.code_off;
    st.sp = 0;
    st.fp = 0;
    st.csp = 1;
    st.fuel = 1000000000000LL;
    st.status = ST_RUNNING;
    st.peak_csp = 1;
    st.mem_size = m.mem_size;
    frames[0] = Frame{0, 0, 0, f.n_results};

    for (uint32_t i = 0; i < n_args; ++i)
        stack[st.sp++] = args[i];
    for (uint16_t i = 0; i < f.n_locals; ++i)
        stack[st.sp++] = 0;

    AoSView sv{stack.data(), STACK_CAP, 0};
    AoSFrameView fv{frames.data(), FRAME_CAP, 0};
    uint64_t dummy_g = 0;
    uint64_t* gptr = m.globals.empty() ? &dummy_g : m.globals.data();
    uint8_t dummy_m = 0;
    MemView mem{m.memory.empty() ? &dummy_m : m.memory.data(), m.mem_size,
                m.memory.empty() ? 0u : (uint32_t)m.memory.size()};
    DataView data{};
    data.blob = m.data_blob.empty() ? nullptr : m.data_blob.data();
    data.blob_len = (uint32_t)m.data_blob.size();
    data.off = m.data_off.empty() ? nullptr : m.data_off.data();
    data.len = m.data_len.empty() ? nullptr : m.data_len.data();
    data.live = m.data_live.empty() ? nullptr : m.data_live.data();
    data.n = (uint32_t)m.data_live.size();
    HostMailbox mb{};
    run_instance(m.dev(), st, sv, fv, gptr, (uint32_t)m.globals.size(), mem, data, &mb, max_steps);

    m.mem_size = st.mem_size;
    r.status = st.status;
    r.peak_csp = st.peak_csp;
    r.steps_bound = max_steps;
    if (st.status == ST_OK) {
        for (uint16_t i = 0; i < f.n_results; ++i)
            r.results.push_back(stack[i]);
    }
    return r;
}

} // namespace cuwasm
