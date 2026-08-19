# CuWASM report

CuWASM is a correctness-first WebAssembly interpreter that runs the **same** execution engine on CPU and on a **single CUDA thread**. WASM is never decoded on the GPU: the host lowers modules to a fixed-width bytecode (`CuOp`), a verifier checks stack heights and branch targets, then `run_instance()` executes that bytecode.

Stage 1 is gated on `tests/fibonacci.wast`. After that gate, the interpreter was extended to pass as many integer/control-flow cases as possible in `tests/wasmi-tests`.

---

## Design

### Goals

- **Correctness over speed.** One instance, AoS stack, `k_run<<<1,1>>>`. No batching, SoA, or throughput work.
- **One interpreter body.** CPU and GPU call the same `__host__ __device__` `run_instance()` in `include/cuwasm/interp.h`. A CPU/GPU mismatch is a plumbing bug, not a second implementation.
- **Loud failure.** Unknown ops and unimplemented features return `ST_UNSUPPORTED_OP`. Traps (`div_by_zero`, `int_overflow`, `unreachable`, stack/call-depth) are never silent.

### Pipeline

```
.wast / .wasm
    │  host (Rust wasmparser)
    ▼
CuOp stream + const pool + FuncMeta + integer globals
    │  verify_cuop() — unique stack height at every PC
    ▼
run_instance()   ←── CPU (g++) and GPU (nvcc, N=1)
```

`CuOp` is 8 bytes: `op:u16`, `a:u16`, `b:u32`. Values live in `u64` stack slots (i32 results occupy the low 32 bits). Locals and the operand stack share one AoS buffer; a frame pointer (`fp`) addresses params/locals, and `sp` is the operand top.

### Control-flow lowering

WASM structured control is flattened on the host:

| WASM | CuOp |
|---|---|
| `if` / `else` | `OP_BR_IF_NOT` (skip `then` when cond == 0); `else` emits `OP_BR` to the join |
| `loop` + `br` | `OP_BR` to the loop header PC |
| `br` / `br_if` to a block/if/func | patch to the join PC |
| extra values under a label | `OP_UNWIND` copies the label arity down and sets `sp` |
| `br_table` | scratch local + `i32.eq` / `br_if` chain, then default `br` |
| `return_call` | `OP_RETURN_CALL` — overwrite the current frame in place; `csp` does not grow |
| `call` | push a `Frame`, zero extra locals, jump |

`OP_UNWIND` is required for `block`/`loop`/`if` with `(param)`/`(result)` when the branch site has extra operands. Without it, `replace-result.wast` and similar tests fail verify (stack-height mismatch) or execute incorrectly.

### What is in / out of scope

**Implemented:** i32/i64 arithmetic, compares, shifts, div/rem (with IEEE-style traps), wrap/extend, `drop`/`select`, integer globals, multi-value, `br_table`, wide i64 mul and 128-bit add/sub.

**Not implemented (by design for this stage):** linear memory, tables/`call_indirect`, SIMD/v128, floats, `funcref`/`externref`, host imports, `start` section.

---

## Implementation

### Layout

```
include/cuwasm/
  cuop.h      CuOp, opcodes, FuncMeta, DevModule
  vmstate.h   Status, VmState, Frame, stack/frame caps
  layout.h    AoSView / AoSFrameView
  hd.h        HD = __host__ __device__
  interp.h    run_instance<> — the only interpreter
  host.h      HostModule, translate/verify/run_cpu
  gpu.h       run_gpu
src/
  translate.cpp   FFI to the Rust translator
  verify.cpp      stack-height CFG join
  run.cpp         CPU launch + wast helpers
  runner.cu       k_run<<<1,1>>>, HtoD/DtoH of code, stack, frames, globals
  disasm.cpp      CuOp dump
  main_cpu.cpp    cuwasm-run
tools/            Rust crate cuwasm-tools
  src/lib.rs              wasmparser → CuOp (C ABI)
  src/bin/oracle.rs       wasmi, same JSON as cuwasm-run
  src/bin/wastprep.rs     split .wast modules → .wasm
  src/bin/wast-catalog.rs wasmi-tests → catalog.jsonl
tests/
  fibonacci.wast   Stage 1 gate (read-only)
  test_main.cpp    CPU/GPU fib + oracle checks
  test_suite.cpp   catalog runner
  wasmi-tests/     wasmi regression suite
```

### Interpreter

`run_instance()` is a `max_steps`-bounded fetch/decode/execute loop. A sentinel frame sits at `csp == 1` on entry; returning from it yields `ST_OK` and copies results to the bottom of the stack. Back-edges charge fuel so a runaway `loop` cannot hang.

Call: `Frame{ret_pc, fp, sp_base, n_results}` is pushed; params already sit at `sp_base`. Return copies `n_results` values onto `sp_base` and restores `pc`/`fp`. Tail call (`OP_RETURN_CALL`) moves params onto the current `fp` and jumps without pushing a frame.

Globals are a host `vector<u64>` copied to the device for `k_run`. Only i32/i64 const-initialized globals are accepted.

### Translator and verifier

`tools/src/lib.rs` decodes with wasmparser 0.227. Unknown operators fail translation (no silent skip). `verify_cuop` walks each function as a CFG, joins stack height at every PC, and fills `FuncMeta.max_stack`. A height mismatch is a translator bug.

### How to run

```
make verify   # fibonacci.wast: CPU vs wasmi vs GPU N=1
make suite    # tests/wasmi-tests catalog + CPU runner
```

Interpreter and test binaries are wrapped in `timeout` to bound infinite loops.

---

## Tests

### Stage 1 gate — `fibonacci.wast`

Three modules (`fibonacci-iter`, `fibonacci-rec`, `fibonacci-tail`), `n = 0..19`:

| Check | Result |
|---|---|
| `assert_return` vs wasmi | **60 / 60** |
| CPU interpreter | **60 / 60** (337 checks including parse/oracle/plumbing) |
| GPU `k_run<<<1,1>>>` | **60 / 60** (273 checks), bit-identical to CPU |

`make verify` is green. A trap is never treated as success.

### wasmi-tests suite

129 `.wast` files. Catalog: **2689** cases (`assert_return` + `assert_trap`). The catalog **skips** imports, non-integer args/results, and `assert_invalid` / `assert_malformed` (949 cases). Those are not attempted.

Latest `make suite` (CPU):

| | Count | Share of catalog | Share of runnable |
|---|---:|---:|---:|
| Pass | 1687 | 62.7% | 97.0% |
| Trap OK | 16 | 0.6% | 0.9% |
| **Score (pass + trap OK)** | **1703** | **63.3%** | **97.9%** |
| Fail / trap fail | 0 | 0% | 0% |
| Unsupported | 37 | 1.4% | 2.1% |
| Skip (catalog) | 949 | 35.3% | — |
| Total | 2689 | 100% | 1740 runnable |

**Runnable pass rate: 1703 / 1740 = 97.9%**, with **zero wrong answers**.

Unsupported remainder (37) is not missing integer ALU or control flow:

| File | Cases | Why |
|---|---:|---|
| `torture.wast` | 28 | One kitchen-sink module: tables, memory, SIMD, floats, externref |
| `audit.wast` | 6 | v128 / memory |
| `global-set.wast` | 2 | `funcref` / `externref` globals |
| `memory64.wast` | 1 | linear memory |

Integer/control-flow coverage that was added after Stage 1 includes `op/*.wast`, `fuse-br` / `fuse-if` / `fuse-select` (integer files), `replace-result.wast`, `if.wast`, `select.wast`, `wide-arithmetic.wast`, `br-table` / `audit.0`, and integer globals.

### Score history (wasmi-tests)

| Checkpoint | Score | What landed |
|---|---:|---|
| Catalog runner only | 151 | i64.add/sub + fibonacci |
| Integer ALU / compare / div, `drop`, `select` | 1658 | i32 + rest of i64 integer ops |
| Integer `global.get` / `global.set` | 1669 | |
| `OP_UNWIND` on `br` / `br_if` | 1683 | block/loop/if arity |
| Wide i64 mul / add128 / sub128 | 1701 | |
| `br_table` | **1703** | |

Each checkpoint was kept only if the score did not drop and `make verify` stayed green.
