#pragma once

#include <cstdint>

namespace cuwasm {

enum Status : uint16_t {
    ST_RUNNING = 0,
    ST_OK,
    ST_TRAP_UNREACHABLE,
    ST_TRAP_STACK_OVERFLOW,
    ST_TRAP_CALL_DEPTH,
    ST_OUT_OF_FUEL,
    ST_UNSUPPORTED_OP,
    ST_TRAP_DIV_BY_ZERO,
    ST_TRAP_INT_OVERFLOW,
    ST_TRAP_MEM_OOB,
    ST_TRAP_INDIRECT_CALL,
    ST_HOSTCALL_PENDING
};

inline const char* status_name(uint16_t s) {
    switch (s) {
    case ST_RUNNING: return "running";
    case ST_OK: return "ok";
    case ST_TRAP_UNREACHABLE: return "trap_unreachable";
    case ST_TRAP_STACK_OVERFLOW: return "trap_stack_overflow";
    case ST_TRAP_CALL_DEPTH: return "trap_call_depth";
    case ST_OUT_OF_FUEL: return "out_of_fuel";
    case ST_UNSUPPORTED_OP: return "unsupported_op";
    case ST_TRAP_DIV_BY_ZERO: return "trap_div_by_zero";
    case ST_TRAP_INT_OVERFLOW: return "trap_int_overflow";
    case ST_TRAP_MEM_OOB: return "trap_mem_oob";
    case ST_TRAP_INDIRECT_CALL: return "trap_indirect_call";
    case ST_HOSTCALL_PENDING: return "hostcall_pending";
    default: return "unknown";
    }
}

struct VmState {
    uint32_t pc, sp, fp, csp;
    int64_t fuel;
    uint16_t status;
    uint32_t peak_csp;
    uint32_t mem_size;
    uint32_t host_fn;
    uint16_t host_n_args;
    uint16_t host_n_results;
};

struct HostMailbox {
    uint32_t fn_id;
    uint16_t n_args;
    uint16_t n_results;
    uint64_t args[16];
    uint64_t results[1];
};

struct Frame {
    uint32_t ret_pc, fp, sp_base;
    uint16_t n_results;
};

static constexpr uint32_t STACK_CAP = 4096;
static constexpr uint32_t FRAME_CAP = 256;
static constexpr int64_t FUEL_BACKEDGE = 1;
static constexpr uint64_t DEFAULT_MAX_STEPS = 10000000ull;

} // namespace cuwasm
