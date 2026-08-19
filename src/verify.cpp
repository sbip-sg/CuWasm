#include "cuwasm/host.h"

#include <sstream>
#include <vector>

namespace cuwasm {

static bool fail(std::string& err, const std::string& msg) {
    err = msg;
    return false;
}

bool verify_cuop(HostModule& m, std::string& err) {
    if (m.funcs.empty())
        return fail(err, "no functions");
    if (m.code.empty())
        return fail(err, "empty code");

    for (uint32_t fi = 0; fi < (uint32_t)m.funcs.size(); ++fi) {
        FuncMeta& f = m.funcs[fi];
        if ((uint64_t)f.code_off + f.code_len > m.code.size())
            return fail(err, "function code out of bounds");
        if (f.code_len == 0)
            return fail(err, "empty function");
        const CuOp& last = m.code[f.code_off + f.code_len - 1];
        if (last.op != OP_RETURN && last.op != OP_END_FUNC)
            return fail(err, "function does not end in return/end_func");

        std::vector<int> height(f.code_len, -1);
        std::vector<uint32_t> work;
        height[0] = 0;
        work.push_back(0);
        int max_h = 0;

        auto join = [&](uint32_t rel, int h) -> bool {
            if (rel >= f.code_len) {
                err = "join target out of function";
                return false;
            }
            if (height[rel] < 0) {
                height[rel] = h;
                work.push_back(rel);
                return true;
            }
            if (height[rel] != h) {
                err = "stack height mismatch at pc " + std::to_string(f.code_off + rel);
                return false;
            }
            return true;
        };

        while (!work.empty()) {
            uint32_t rel = work.back();
            work.pop_back();
            const CuOp& in = m.code[f.code_off + rel];
            int h = height[rel];
            if (h < 0)
                return fail(err, "uninitialized height");
            if (h > max_h)
                max_h = h;
            bool fall = true;
            int next_h = h;

            switch (in.op) {
            case OP_I64_CONST:
            case OP_LOCAL_GET:
                next_h = h + 1;
                break;
            case OP_LOCAL_SET:
                if (h < 1)
                    return fail(err, "local.set underflow");
                next_h = h - 1;
                break;
            case OP_I64_ADD:
            case OP_I64_SUB:
            case OP_I64_EQ:
            case OP_I64_LE_S:
            case OP_I64_LT_S:
            case OP_I32_EQ:
            case OP_I32_NE:
            case OP_I32_LT_S:
            case OP_I32_LT_U:
            case OP_I32_LE_S:
            case OP_I32_LE_U:
            case OP_I32_GT_S:
            case OP_I32_GT_U:
            case OP_I32_GE_S:
            case OP_I32_GE_U:
            case OP_I32_ADD:
            case OP_I32_SUB:
            case OP_I32_MUL:
            case OP_I32_AND:
            case OP_I32_OR:
            case OP_I32_XOR:
            case OP_I32_DIV_S:
            case OP_I32_DIV_U:
            case OP_I32_REM_S:
            case OP_I32_REM_U:
            case OP_I32_SHL:
            case OP_I32_SHR_S:
            case OP_I32_SHR_U:
            case OP_I64_NE:
            case OP_I64_LT_U:
            case OP_I64_LE_U:
            case OP_I64_GT_S:
            case OP_I64_GT_U:
            case OP_I64_GE_S:
            case OP_I64_GE_U:
            case OP_I64_MUL:
            case OP_I64_AND:
            case OP_I64_OR:
            case OP_I64_XOR:
            case OP_I64_DIV_S:
            case OP_I64_DIV_U:
            case OP_I64_REM_S:
            case OP_I64_REM_U:
            case OP_I64_SHL:
            case OP_I64_SHR_S:
            case OP_I64_SHR_U:
                if (h < 2)
                    return fail(err, "binop underflow");
                next_h = h - 1;
                break;
            case OP_I64_EQZ:
            case OP_I32_EQZ:
            case OP_I32_WRAP_I64:
            case OP_I64_EXTEND_I32_S:
            case OP_I64_EXTEND_I32_U:
                if (h < 1)
                    return fail(err, "unop underflow");
                next_h = h;
                break;
            case OP_I64_MUL_WIDE_S:
            case OP_I64_MUL_WIDE_U:
                if (h < 2)
                    return fail(err, "mul_wide underflow");
                next_h = h;
                break;
            case OP_I64_ADD128:
            case OP_I64_SUB128:
                if (h < 4)
                    return fail(err, "i64.xxx128 underflow");
                next_h = h - 2;
                break;
            case OP_DROP:
                if (h < 1)
                    return fail(err, "drop underflow");
                next_h = h - 1;
                break;
            case OP_GLOBAL_GET:
                next_h = h + 1;
                break;
            case OP_GLOBAL_SET:
                if (h < 1)
                    return fail(err, "global.set underflow");
                next_h = h - 1;
                break;
            case OP_SELECT:
                if (h < 3)
                    return fail(err, "select underflow");
                next_h = h - 2;
                break;
            case OP_UNWIND: {
                int dest_h = (int)in.b - (int)f.n_params - (int)f.n_locals;
                if (dest_h < 0)
                    return fail(err, "unwind dest below locals");
                if (h < dest_h)
                    return fail(err, "unwind underflow");
                next_h = dest_h;
                break;
            }
            case OP_BR: {
                if (in.b < f.code_off || in.b >= f.code_off + f.code_len)
                    return fail(err, "br target outside function");
                if (!join(in.b - f.code_off, h))
                    return false;
                fall = false;
                break;
            }
            case OP_BR_IF_NOT: {
                if (h < 1)
                    return fail(err, "br_if_not underflow");
                next_h = h - 1;
                if (in.b < f.code_off || in.b >= f.code_off + f.code_len)
                    return fail(err, "br_if_not target outside function");
                if (!join(in.b - f.code_off, next_h))
                    return false;
                break;
            }
            case OP_CALL: {
                if (in.b >= m.funcs.size())
                    return fail(err, "call func oob");
                const FuncMeta& g = m.funcs[in.b];
                if (h < (int)g.n_params)
                    return fail(err, "call underflow");
                next_h = h - (int)g.n_params + (int)g.n_results;
                break;
            }
            case OP_RETURN_CALL: {
                if (in.b >= m.funcs.size())
                    return fail(err, "return_call func oob");
                const FuncMeta& g = m.funcs[in.b];
                if (h < (int)g.n_params)
                    return fail(err, "return_call underflow");
                fall = false;
                break;
            }
            case OP_RETURN:
            case OP_END_FUNC:
                if (h < (int)f.n_results)
                    return fail(err, "return underflow");
                fall = false;
                break;
            case OP_UNREACHABLE:
                fall = false;
                break;
            default:
                return fail(err, "unknown cuop in verify");
            }

            if (in.op == OP_LOCAL_GET && in.a >= (uint16_t)(f.n_params + f.n_locals))
                return fail(err, "local.get index oob");
            if (in.op == OP_LOCAL_SET && in.a >= (uint16_t)(f.n_params + f.n_locals))
                return fail(err, "local.set index oob");
            if (in.op == OP_I64_CONST && in.b >= m.consts.size())
                return fail(err, "const pool oob");

            if (next_h > max_h)
                max_h = next_h;

            if (fall) {
                if (rel + 1 >= f.code_len)
                    return fail(err, "fallthrough past end of function");
                if (!join(rel + 1, next_h))
                    return false;
            }
        }

        int max_sp = (int)f.n_params + (int)f.n_locals + max_h;
        if (max_sp < 0)
            max_sp = 0;
        if (max_sp > 65535)
            return fail(err, "max_stack overflow");
        f.max_stack = (uint16_t)max_sp;
    }
    return true;
}

} // namespace cuwasm
