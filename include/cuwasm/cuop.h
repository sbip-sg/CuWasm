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
    case OP_I64_CONST: return "i64.const";
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
    default: return "???";
    }
}

} // namespace cuwasm
