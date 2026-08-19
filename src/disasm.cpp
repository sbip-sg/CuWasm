#include "cuwasm/host.h"

#include <cstdio>
#include <sstream>

namespace cuwasm {

std::string disasm(const HostModule& m) {
    std::ostringstream os;
    for (uint32_t fi = 0; fi < (uint32_t)m.funcs.size(); ++fi) {
        const FuncMeta& f = m.funcs[fi];
        os << "func " << fi << " params=" << f.n_params << " results=" << f.n_results
           << " locals=" << f.n_locals << " max_stack=" << f.max_stack
           << " off=" << f.code_off << " len=" << f.code_len << "\n";
        for (uint32_t i = 0; i < f.code_len; ++i) {
            uint32_t pc = f.code_off + i;
            const CuOp& in = m.code[pc];
            os << "  " << pc << ": " << opcode_name(in.op) << " a=" << in.a << " b=" << in.b;
            if (in.op == OP_I64_CONST && in.b < m.consts.size())
                os << " imm=" << (int64_t)m.consts[in.b];
            os << "\n";
        }
    }
    os << "exports:\n";
    for (const auto& e : m.exports)
        os << "  " << e.first << " -> " << e.second << "\n";
    return os.str();
}

} // namespace cuwasm
