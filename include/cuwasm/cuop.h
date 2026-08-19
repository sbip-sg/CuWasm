#pragma once

#include <cstdint>

namespace cuwasm {

enum CuOpcode : uint16_t {
    OP_UNREACHABLE = 0,
    OP_I64_CONST,
    OP_LOCAL_GET,
    OP_LOCAL_SET,
    OP_I64_ADD,
    OP_I64_SUB,
    OP_I64_EQ,
    OP_I64_EQZ,
    OP_I64_LE_S,
    OP_I64_LT_S,
    OP_BR,
    OP_BR_IF_NOT,
    OP_CALL,
    OP_RETURN_CALL,
    OP_RETURN,
    OP_END_FUNC,
    OP_DROP,
    OP_SELECT,
    OP_I32_EQZ,
    OP_I32_EQ,
    OP_I32_NE,
    OP_I32_LT_S,
    OP_I32_LT_U,
    OP_I32_LE_S,
    OP_I32_LE_U,
    OP_I32_GT_S,
    OP_I32_GT_U,
    OP_I32_GE_S,
    OP_I32_GE_U,
    OP_I32_ADD,
    OP_I32_SUB,
    OP_I32_MUL,
    OP_I32_AND,
    OP_I32_OR,
    OP_I32_XOR,
    OP_I32_DIV_S,
    OP_I32_DIV_U,
    OP_I32_REM_S,
    OP_I32_REM_U,
    OP_I32_SHL,
    OP_I32_SHR_S,
    OP_I32_SHR_U,
    OP_I32_WRAP_I64,
    OP_I64_NE,
    OP_I64_LT_U,
    OP_I64_LE_U,
    OP_I64_GT_S,
    OP_I64_GT_U,
    OP_I64_GE_S,
    OP_I64_GE_U,
    OP_I64_MUL,
    OP_I64_AND,
    OP_I64_OR,
    OP_I64_XOR,
    OP_I64_DIV_S,
    OP_I64_DIV_U,
    OP_I64_REM_S,
    OP_I64_REM_U,
    OP_I64_SHL,
    OP_I64_SHR_S,
    OP_I64_SHR_U,
    OP_I64_EXTEND_I32_S,
    OP_I64_EXTEND_I32_U,
    OP_GLOBAL_GET,
    OP_GLOBAL_SET,
    OP_UNWIND,
    OP_I64_MUL_WIDE_S,
    OP_I64_MUL_WIDE_U,
    OP_I64_ADD128,
    OP_I64_SUB128,
};

struct alignas(8) CuOp {
    uint16_t op;
    uint16_t a;
    uint32_t b;
};

struct FuncMeta {
    uint32_t code_off, code_len;
    uint16_t n_params, n_results, n_locals, max_stack;
};

struct DevModule {
    const CuOp* code;
    const uint64_t* consts;
    const FuncMeta* funcs;
    uint32_t n_funcs, code_len;
};

inline const char* opcode_name(uint16_t op) {
    switch (op) {
    case OP_UNREACHABLE: return "unreachable";
    case OP_I64_CONST: return "const";
    case OP_LOCAL_GET: return "local.get";
    case OP_LOCAL_SET: return "local.set";
    case OP_I64_ADD: return "i64.add";
    case OP_I64_SUB: return "i64.sub";
    case OP_I64_EQ: return "i64.eq";
    case OP_I64_EQZ: return "i64.eqz";
    case OP_I64_LE_S: return "i64.le_s";
    case OP_I64_LT_S: return "i64.lt_s";
    case OP_BR: return "br";
    case OP_BR_IF_NOT: return "br_if_not";
    case OP_CALL: return "call";
    case OP_RETURN_CALL: return "return_call";
    case OP_RETURN: return "return";
    case OP_END_FUNC: return "end_func";
    case OP_DROP: return "drop";
    case OP_SELECT: return "select";
    case OP_I32_EQZ: return "i32.eqz";
    case OP_I32_EQ: return "i32.eq";
    case OP_I32_NE: return "i32.ne";
    case OP_I32_LT_S: return "i32.lt_s";
    case OP_I32_LT_U: return "i32.lt_u";
    case OP_I32_LE_S: return "i32.le_s";
    case OP_I32_LE_U: return "i32.le_u";
    case OP_I32_GT_S: return "i32.gt_s";
    case OP_I32_GT_U: return "i32.gt_u";
    case OP_I32_GE_S: return "i32.ge_s";
    case OP_I32_GE_U: return "i32.ge_u";
    case OP_I32_ADD: return "i32.add";
    case OP_I32_SUB: return "i32.sub";
    case OP_I32_MUL: return "i32.mul";
    case OP_I32_AND: return "i32.and";
    case OP_I32_OR: return "i32.or";
    case OP_I32_XOR: return "i32.xor";
    case OP_I32_DIV_S: return "i32.div_s";
    case OP_I32_DIV_U: return "i32.div_u";
    case OP_I32_REM_S: return "i32.rem_s";
    case OP_I32_REM_U: return "i32.rem_u";
    case OP_I32_SHL: return "i32.shl";
    case OP_I32_SHR_S: return "i32.shr_s";
    case OP_I32_SHR_U: return "i32.shr_u";
    case OP_I32_WRAP_I64: return "i32.wrap_i64";
    case OP_I64_NE: return "i64.ne";
    case OP_I64_LT_U: return "i64.lt_u";
    case OP_I64_LE_U: return "i64.le_u";
    case OP_I64_GT_S: return "i64.gt_s";
    case OP_I64_GT_U: return "i64.gt_u";
    case OP_I64_GE_S: return "i64.ge_s";
    case OP_I64_GE_U: return "i64.ge_u";
    case OP_I64_MUL: return "i64.mul";
    case OP_I64_AND: return "i64.and";
    case OP_I64_OR: return "i64.or";
    case OP_I64_XOR: return "i64.xor";
    case OP_I64_DIV_S: return "i64.div_s";
    case OP_I64_DIV_U: return "i64.div_u";
    case OP_I64_REM_S: return "i64.rem_s";
    case OP_I64_REM_U: return "i64.rem_u";
    case OP_I64_SHL: return "i64.shl";
    case OP_I64_SHR_S: return "i64.shr_s";
    case OP_I64_SHR_U: return "i64.shr_u";
    case OP_I64_EXTEND_I32_S: return "i64.extend_i32_s";
    case OP_I64_EXTEND_I32_U: return "i64.extend_i32_u";
    case OP_GLOBAL_GET: return "global.get";
    case OP_GLOBAL_SET: return "global.set";
    case OP_UNWIND: return "unwind";
    case OP_I64_MUL_WIDE_S: return "i64.mul_wide_s";
    case OP_I64_MUL_WIDE_U: return "i64.mul_wide_u";
    case OP_I64_ADD128: return "i64.add128";
    case OP_I64_SUB128: return "i64.sub128";
    default: return "???";
    }
}

} // namespace cuwasm
