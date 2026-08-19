#pragma once

#include "cuop.h"
#include "layout.h"
#include "vmstate.h"

namespace cuwasm {

HD void mul_wide_u(uint64_t a, uint64_t b, uint64_t& lo, uint64_t& hi) {
    uint64_t a0 = (uint32_t)a, a1 = a >> 32;
    uint64_t b0 = (uint32_t)b, b1 = b >> 32;
    uint64_t p0 = a0 * b0;
    uint64_t p1 = a0 * b1;
    uint64_t p2 = a1 * b0;
    uint64_t p3 = a1 * b1;
    uint64_t c = (p0 >> 32) + (uint32_t)p1 + (uint32_t)p2;
    lo = (p0 & 0xffffffffull) | (c << 32);
    hi = p3 + (p1 >> 32) + (p2 >> 32) + (c >> 32);
}

HD void mul_wide_s(uint64_t a, uint64_t b, uint64_t& lo, uint64_t& hi) {
    mul_wide_u(a, b, lo, hi);
    if ((int64_t)a < 0)
        hi -= b;
    if ((int64_t)b < 0)
        hi -= a;
}

template <class StackV, class FrameV>
HD void run_instance(const DevModule m, VmState& st, StackV stack, FrameV frames,
                     uint64_t* globals, uint32_t n_globals, uint64_t max_steps) {
    uint32_t pc = st.pc, sp = st.sp, fp = st.fp, csp = st.csp;
    int64_t fuel = st.fuel;
    uint32_t peak_csp = st.peak_csp;
    if (csp > peak_csp) peak_csp = csp;

#define TRAP(s)                                                                \
    do {                                                                       \
        st.status = (s);                                                       \
        goto done;                                                             \
    } while (0)
#define CU_PUSH(v)                                                             \
    do {                                                                       \
        if (sp >= STACK_CAP)                                                   \
            TRAP(ST_TRAP_STACK_OVERFLOW);                                      \
        stack.at(sp++) = (uint64_t)(v);                                        \
    } while (0)
#define CU_POP() (stack.at(--sp))

    for (uint64_t step = 0; step < max_steps; ++step) {
        if (pc >= m.code_len)
            TRAP(ST_TRAP_UNREACHABLE);
        const CuOp in = m.code[pc++];
        switch (in.op) {
        case OP_I64_CONST:
            CU_PUSH(m.consts[in.b]);
            break;
        case OP_LOCAL_GET:
            CU_PUSH(stack.at(fp + in.a));
            break;
        case OP_LOCAL_SET:
            stack.at(fp + in.a) = CU_POP();
            break;

        case OP_I64_ADD: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH(a + b);
            break;
        }
        case OP_I64_SUB: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH(a - b);
            break;
        }
        case OP_I64_LE_S: {
            int64_t b = (int64_t)CU_POP(), a = (int64_t)CU_POP();
            CU_PUSH((uint32_t)(a <= b));
            break;
        }
        case OP_I64_LT_S: {
            int64_t b = (int64_t)CU_POP(), a = (int64_t)CU_POP();
            CU_PUSH((uint32_t)(a < b));
            break;
        }
        case OP_I64_EQ: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH((uint32_t)(a == b));
            break;
        }
        case OP_I64_EQZ: {
            uint64_t a = CU_POP();
            CU_PUSH((uint32_t)(a == 0));
            break;
        }

        case OP_DROP:
            (void)CU_POP();
            break;
        case OP_SELECT: {
            uint32_t c = (uint32_t)CU_POP();
            uint64_t b = CU_POP();
            uint64_t a = CU_POP();
            CU_PUSH(c ? a : b);
            break;
        }

        case OP_I32_EQZ: {
            uint32_t a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a == 0));
            break;
        }
        case OP_I32_EQ: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a == b));
            break;
        }
        case OP_I32_NE: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a != b));
            break;
        }
        case OP_I32_LT_S: {
            int32_t b = (int32_t)CU_POP(), a = (int32_t)CU_POP();
            CU_PUSH((uint32_t)(a < b));
            break;
        }
        case OP_I32_LT_U: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a < b));
            break;
        }
        case OP_I32_LE_S: {
            int32_t b = (int32_t)CU_POP(), a = (int32_t)CU_POP();
            CU_PUSH((uint32_t)(a <= b));
            break;
        }
        case OP_I32_LE_U: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a <= b));
            break;
        }
        case OP_I32_GT_S: {
            int32_t b = (int32_t)CU_POP(), a = (int32_t)CU_POP();
            CU_PUSH((uint32_t)(a > b));
            break;
        }
        case OP_I32_GT_U: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a > b));
            break;
        }
        case OP_I32_GE_S: {
            int32_t b = (int32_t)CU_POP(), a = (int32_t)CU_POP();
            CU_PUSH((uint32_t)(a >= b));
            break;
        }
        case OP_I32_GE_U: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a >= b));
            break;
        }
        case OP_I32_ADD: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a + b));
            break;
        }
        case OP_I32_SUB: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a - b));
            break;
        }
        case OP_I32_MUL: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a * b));
            break;
        }
        case OP_I32_AND: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH(a & b);
            break;
        }
        case OP_I32_OR: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH(a | b);
            break;
        }
        case OP_I32_XOR: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            CU_PUSH(a ^ b);
            break;
        }
        case OP_I32_DIV_S: {
            int32_t b = (int32_t)CU_POP(), a = (int32_t)CU_POP();
            if (b == 0)
                TRAP(ST_TRAP_DIV_BY_ZERO);
            if (a == (int32_t)0x80000000 && b == -1)
                TRAP(ST_TRAP_INT_OVERFLOW);
            CU_PUSH((uint32_t)(a / b));
            break;
        }
        case OP_I32_DIV_U: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            if (b == 0)
                TRAP(ST_TRAP_DIV_BY_ZERO);
            CU_PUSH(a / b);
            break;
        }
        case OP_I32_REM_S: {
            int32_t b = (int32_t)CU_POP(), a = (int32_t)CU_POP();
            if (b == 0)
                TRAP(ST_TRAP_DIV_BY_ZERO);
            if (a == (int32_t)0x80000000 && b == -1) {
                CU_PUSH(0);
                break;
            }
            CU_PUSH((uint32_t)(a % b));
            break;
        }
        case OP_I32_REM_U: {
            uint32_t b = (uint32_t)CU_POP(), a = (uint32_t)CU_POP();
            if (b == 0)
                TRAP(ST_TRAP_DIV_BY_ZERO);
            CU_PUSH(a % b);
            break;
        }
        case OP_I32_SHL: {
            uint32_t b = (uint32_t)CU_POP() & 31u, a = (uint32_t)CU_POP();
            CU_PUSH((uint32_t)(a << b));
            break;
        }
        case OP_I32_SHR_S: {
            uint32_t b = (uint32_t)CU_POP() & 31u;
            int32_t a = (int32_t)CU_POP();
            CU_PUSH((uint32_t)(a >> b));
            break;
        }
        case OP_I32_SHR_U: {
            uint32_t b = (uint32_t)CU_POP() & 31u, a = (uint32_t)CU_POP();
            CU_PUSH(a >> b);
            break;
        }
        case OP_I32_WRAP_I64: {
            uint64_t a = CU_POP();
            CU_PUSH((uint32_t)a);
            break;
        }

        case OP_I64_NE: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH((uint32_t)(a != b));
            break;
        }
        case OP_I64_LT_U: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH((uint32_t)(a < b));
            break;
        }
        case OP_I64_LE_U: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH((uint32_t)(a <= b));
            break;
        }
        case OP_I64_GT_S: {
            int64_t b = (int64_t)CU_POP(), a = (int64_t)CU_POP();
            CU_PUSH((uint32_t)(a > b));
            break;
        }
        case OP_I64_GT_U: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH((uint32_t)(a > b));
            break;
        }
        case OP_I64_GE_S: {
            int64_t b = (int64_t)CU_POP(), a = (int64_t)CU_POP();
            CU_PUSH((uint32_t)(a >= b));
            break;
        }
        case OP_I64_GE_U: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH((uint32_t)(a >= b));
            break;
        }
        case OP_I64_MUL: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH(a * b);
            break;
        }
        case OP_I64_AND: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH(a & b);
            break;
        }
        case OP_I64_OR: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH(a | b);
            break;
        }
        case OP_I64_XOR: {
            uint64_t b = CU_POP(), a = CU_POP();
            CU_PUSH(a ^ b);
            break;
        }
        case OP_I64_DIV_S: {
            int64_t b = (int64_t)CU_POP(), a = (int64_t)CU_POP();
            if (b == 0)
                TRAP(ST_TRAP_DIV_BY_ZERO);
            if (a == (int64_t)((uint64_t)1 << 63) && b == -1)
                TRAP(ST_TRAP_INT_OVERFLOW);
            CU_PUSH((uint64_t)(a / b));
            break;
        }
        case OP_I64_DIV_U: {
            uint64_t b = CU_POP(), a = CU_POP();
            if (b == 0)
                TRAP(ST_TRAP_DIV_BY_ZERO);
            CU_PUSH(a / b);
            break;
        }
        case OP_I64_REM_S: {
            int64_t b = (int64_t)CU_POP(), a = (int64_t)CU_POP();
            if (b == 0)
                TRAP(ST_TRAP_DIV_BY_ZERO);
            if (a == (int64_t)((uint64_t)1 << 63) && b == -1) {
                CU_PUSH(0);
                break;
            }
            CU_PUSH((uint64_t)(a % b));
            break;
        }
        case OP_I64_REM_U: {
            uint64_t b = CU_POP(), a = CU_POP();
            if (b == 0)
                TRAP(ST_TRAP_DIV_BY_ZERO);
            CU_PUSH(a % b);
            break;
        }
        case OP_I64_SHL: {
            uint64_t b = CU_POP() & 63u, a = CU_POP();
            CU_PUSH(a << b);
            break;
        }
        case OP_I64_SHR_S: {
            uint64_t b = CU_POP() & 63u;
            int64_t a = (int64_t)CU_POP();
            CU_PUSH((uint64_t)(a >> b));
            break;
        }
        case OP_I64_SHR_U: {
            uint64_t b = CU_POP() & 63u, a = CU_POP();
            CU_PUSH(a >> b);
            break;
        }
        case OP_I64_EXTEND_I32_S: {
            uint64_t a = CU_POP();
            CU_PUSH((uint64_t)(int64_t)(int32_t)a);
            break;
        }
        case OP_I64_EXTEND_I32_U: {
            uint64_t a = CU_POP();
            CU_PUSH((uint64_t)(uint32_t)a);
            break;
        }
        case OP_I64_MUL_WIDE_U: {
            uint64_t b = CU_POP(), a = CU_POP();
            uint64_t lo, hi;
            mul_wide_u(a, b, lo, hi);
            CU_PUSH(lo);
            CU_PUSH(hi);
            break;
        }
        case OP_I64_MUL_WIDE_S: {
            uint64_t b = CU_POP(), a = CU_POP();
            uint64_t lo, hi;
            mul_wide_s(a, b, lo, hi);
            CU_PUSH(lo);
            CU_PUSH(hi);
            break;
        }
        case OP_I64_ADD128: {
            uint64_t rhs_hi = CU_POP(), rhs_lo = CU_POP(), lhs_hi = CU_POP(), lhs_lo = CU_POP();
            uint64_t lo = lhs_lo + rhs_lo;
            uint64_t c = (lo < lhs_lo) ? 1ull : 0ull;
            uint64_t hi = lhs_hi + rhs_hi + c;
            CU_PUSH(lo);
            CU_PUSH(hi);
            break;
        }
        case OP_I64_SUB128: {
            uint64_t rhs_hi = CU_POP(), rhs_lo = CU_POP(), lhs_hi = CU_POP(), lhs_lo = CU_POP();
            uint64_t lo = lhs_lo - rhs_lo;
            uint64_t brw = (lhs_lo < rhs_lo) ? 1ull : 0ull;
            uint64_t hi = lhs_hi - rhs_hi - brw;
            CU_PUSH(lo);
            CU_PUSH(hi);
            break;
        }

        case OP_GLOBAL_GET:
            if (in.b >= n_globals || !globals)
                TRAP(ST_UNSUPPORTED_OP);
            CU_PUSH(globals[in.b]);
            break;
        case OP_GLOBAL_SET:
            if (in.b >= n_globals || !globals)
                TRAP(ST_UNSUPPORTED_OP);
            globals[in.b] = CU_POP();
            break;

        case OP_UNWIND: {
            uint32_t nkeep = in.a;
            uint32_t dest = fp + in.b;
            if (nkeep > sp)
                TRAP(ST_TRAP_STACK_OVERFLOW);
            if (dest < nkeep || dest > STACK_CAP)
                TRAP(ST_TRAP_STACK_OVERFLOW);
            uint32_t src = sp - nkeep;
            uint32_t dst = dest - nkeep;
            if (dst != src) {
                if (dst <= src) {
                    for (uint32_t i = 0; i < nkeep; ++i)
                        stack.at(dst + i) = stack.at(src + i);
                } else {
                    for (int i = (int)nkeep - 1; i >= 0; --i)
                        stack.at(dst + (uint32_t)i) = stack.at(src + (uint32_t)i);
                }
            }
            sp = dest;
            break;
        }

        case OP_BR:
            if (in.b <= pc) {
                if ((fuel -= FUEL_BACKEDGE) <= 0)
                    TRAP(ST_OUT_OF_FUEL);
            }
            pc = in.b;
            break;

        case OP_BR_IF_NOT:
            if ((uint32_t)CU_POP() == 0)
                pc = in.b;
            break;

        case OP_CALL: {
            if (in.b >= m.n_funcs)
                TRAP(ST_UNSUPPORTED_OP);
            const FuncMeta f = m.funcs[in.b];
            if (csp >= FRAME_CAP)
                TRAP(ST_TRAP_CALL_DEPTH);
            if (sp < f.n_params)
                TRAP(ST_TRAP_STACK_OVERFLOW);
            uint32_t sp_base = sp - f.n_params;
            frames.at(csp++) = Frame{pc, fp, sp_base, f.n_results};
            if (csp > peak_csp)
                peak_csp = csp;
            fp = sp_base;
            for (uint16_t i = 0; i < f.n_locals; ++i)
                CU_PUSH(0);
            pc = f.code_off;
            break;
        }

        case OP_RETURN_CALL: {
            if (in.b >= m.n_funcs)
                TRAP(ST_UNSUPPORTED_OP);
            const FuncMeta f = m.funcs[in.b];
            if (sp < f.n_params)
                TRAP(ST_TRAP_STACK_OVERFLOW);
            uint32_t src = sp - f.n_params;
            if (fp <= src) {
                for (uint16_t i = 0; i < f.n_params; ++i)
                    stack.at(fp + i) = stack.at(src + i);
            } else {
                for (int i = (int)f.n_params - 1; i >= 0; --i)
                    stack.at(fp + (uint32_t)i) = stack.at(src + (uint32_t)i);
            }
            sp = fp + f.n_params;
            for (uint16_t i = 0; i < f.n_locals; ++i)
                CU_PUSH(0);
            if (csp > 0)
                frames.at(csp - 1).n_results = f.n_results;
            pc = f.code_off;
            break;
        }

        case OP_RETURN:
        case OP_END_FUNC: {
            if (csp == 0)
                TRAP(ST_TRAP_UNREACHABLE);
            Frame fr = frames.at(--csp);
            uint32_t nres = fr.n_results;
            if (sp < nres)
                TRAP(ST_TRAP_STACK_OVERFLOW);
            uint32_t src = sp - nres;
            if (fr.sp_base <= src) {
                for (uint32_t i = 0; i < nres; ++i)
                    stack.at(fr.sp_base + i) = stack.at(src + i);
            } else {
                for (int i = (int)nres - 1; i >= 0; --i)
                    stack.at(fr.sp_base + (uint32_t)i) = stack.at(src + (uint32_t)i);
            }
            sp = fr.sp_base + nres;
            fp = fr.fp;
            if (csp == 0) {
                st.status = ST_OK;
                goto done;
            }
            pc = fr.ret_pc;
            break;
        }

        case OP_UNREACHABLE:
            TRAP(ST_TRAP_UNREACHABLE);
        default:
            TRAP(ST_UNSUPPORTED_OP);
        }
        if (sp > STACK_CAP)
            TRAP(ST_TRAP_STACK_OVERFLOW);
    }
    st.status = ST_RUNNING;
done:
    st.pc = pc;
    st.sp = sp;
    st.fp = fp;
    st.csp = csp;
    st.fuel = fuel;
    st.peak_csp = peak_csp;
#undef CU_PUSH
#undef CU_POP
#undef TRAP
}

} // namespace cuwasm
