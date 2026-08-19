#pragma once

#include "cuop.h"
#include "layout.h"
#include "vmstate.h"

namespace cuwasm {

template <class StackV, class FrameV>
HD void run_instance(const DevModule m, VmState& st, StackV stack, FrameV frames,
                     uint64_t max_steps) {
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
