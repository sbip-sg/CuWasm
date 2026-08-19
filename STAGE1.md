# Stage 1 — correctness on one GPU thread

**Done when:** all 60 `assert_return`s in `tests/fibonacci.wast` pass on the GPU at **N = 1** (one CUDA thread, one instance) and match the CPU interpreter and wasmi.

**Not this stage:** batching, SoA layout, fuel, resume, mutation testing, metamorphic transforms, golden files, `ncu`, throughput vs wasmi. Those stay in `CUWASM_PLAN_V3.md` for later.

Correctness first. One instance. Slow is fine.

---

## Gate

```
wasmi(module, n) == cpu_interp(module, n) == gpu_n1(module, n)
for n = 0..19 on fibonacci-iter, fibonacci-rec, fibonacci-tail
```

60/60 or it is not done. A trap is never silent.

---

## What the test file actually needs

Three modules, 20 asserts each (`n = 0..19`). No memory, globals, imports, `block`, `br_if`, `br_table`, or `call_indirect`. 100% i64.

| CuOp | Why |
|---|---|
| `OP_I64_CONST` | literals |
| `OP_LOCAL_GET` / `OP_LOCAL_SET` | params + locals |
| `OP_I64_ADD` / `OP_I64_SUB` | wrapping |
| `OP_I64_EQ` / `OP_I64_EQZ` | tail base cases |
| `OP_I64_LE_S` / `OP_I64_LT_S` | signed compares (result i32 in a u64 slot) |
| `OP_BR` | `loop $continue` back-edge in iter |
| `OP_BR_IF_NOT` | lowered `if` (jump past `then` when cond == 0) |
| `OP_CALL` | rec |
| `OP_RETURN_CALL` | tail — **reuse** the frame, do not push |
| `OP_RETURN` / `OP_END_FUNC` | pop frame, move results |
| `OP_UNREACHABLE` | trap sentinel; not in the file |

Anything else → `ST_UNSUPPORTED_OP` (loud fail, never a no-op).

`return_call` is required: `fibonacci-tail` uses it. Implement it. Do not degrade it to `call`+`return`.

---

## Design to keep (do not reopen)

1. **Lower WASM to fixed 8-byte `CuOp` on the host.** Do not interpret raw WASM bytes on the GPU.
2. **One interpreter body.** `run_instance()` is `__host__ __device__` in `include/cuwasm/interp.h`. CPU and GPU share it. CPU/GPU disagreement is a plumbing bug.
3. **AoS only.** One instance owns a contiguous stack. SoA is a later performance experiment.
4. Use `wasmparser` (or wabt) for decode/validate. Hand-write only the lowering.

```
cuwasm/
├── Makefile
├── include/cuwasm/
│   ├── cuop.h          # CuOp, FuncMeta, DevModule, opcodes
│   ├── vmstate.h       # Status, VmState, Frame
│   ├── layout.h        # AoSView only
│   └── interp.h        # run_instance<> — host AND device, one body
├── src/
│   ├── translate.cpp   # wasm → CuOp
│   ├── verify.cpp      # verify_cuop() — bounds + stack height
│   ├── main_cpu.cpp    # cuwasm-run CPU
│   └── runner.cu       # k_run, N=1 launch
├── tools/oracle/       # wasmi → same JSON as cuwasm-run
└── tests/
    ├── fibonacci.wast  # vendored, read-only
    └── test_fib.cpp
```

`cuwasm-run <module.wasm> <export> <args...>` prints:

```json
{"status": "ok", "results": [13]}
```

---

## Tasks

Do them in order. Each task has a test that must fail before the code goes in.

### T0 — Scaffold

- Makefile: `make test-cpu`, `make test-gpu`, `make verify`
- Headers: `CuOp`, opcodes, `VmState`, `Frame`, `AoSView`, `Status`
- Stub `run_instance` that traps `ST_UNSUPPORTED_OP`
- Vendor `tests/fibonacci.wast` unchanged

**Test:** project compiles (CPU). Interpreter returns `ST_UNSUPPORTED_OP`.

### T1 — Assertions + oracle

- Split the three `(module …)` blocks; `wat2wasm` (or `wast` crate) → `.wasm`
- Parse the 60 `assert_return`s into `(export, args, expected)`
- Small wasmi oracle binary using the same JSON schema as `cuwasm-run`

**Test:** `test_parse_assertions` expects 60. `test_oracle_fib` is 60/60 on wasmi.

### T2 — Lowering

- `translate.cpp`: WASM bytes → `DevModule` (`code`, `consts`, `funcs`)
- Lower `if` to `OP_BR_IF_NOT` (skip `then` when cond == 0)
- Lower `loop` + `br $continue` to `OP_BR` with an absolute PC
- Lower `return_call` to `OP_RETURN_CALL` (not `OP_CALL`)
- Unknown opcode → fail the translation (do not emit a silent skip)

**Test:** all three modules lower with zero unsupported ops. Dump of `CuOp` stream is readable (eyeball branch targets once).

### T3 — `verify_cuop` (cheap, catches translator bugs)

Linear pass, hard fail:

1. Every `OP_BR` / `OP_BR_IF_NOT` target is inside the same function
2. Every `OP_CALL` / `OP_RETURN_CALL` index is `< n_funcs`
3. Abstract stack height is unique at every PC (two predecessors disagree → translator bug)
4. Function code ends in `OP_RETURN` or `OP_END_FUNC`
5. `FuncMeta.max_stack` is **computed here**, not guessed

**Test:** clean on the three modules. Fires on at least one hand-corrupted fixture (bad branch target).

### T4 — CPU interpreter: iter

Implement in `interp.h`: const, locals, add/sub, compares, `OP_BR`, `OP_BR_IF_NOT`, `OP_RETURN` (csp == 0 → `ST_OK`).

**Test:** `fibonacci-iter` n = 0..19 on CPU. Rec and tail still fail.

### T5 — CPU interpreter: rec

Implement `OP_CALL` and `OP_RETURN` / `OP_END_FUNC`:

- `CALL`: push `Frame{ret_pc, fp, sp - n_params, n_results}`; `fp = sp - n_params`; zero extra locals; `pc = funcs[idx].code_off`
- `RETURN`: if `csp == 0` → `ST_OK`; else move `n_results` down onto `sp_base`; restore `pc`/`fp`/`sp` from the frame

**Test:** `fibonacci-rec` n = 0..19 on CPU.

### T6 — CPU interpreter: tail

Implement `OP_RETURN_CALL`: overwrite current frame params in place, reset `sp`, jump. **`csp` must not grow.**

**Test:** `fibonacci-tail` n = 0..19 on CPU. Spot-check `csp` stays ≤ 2 (if it tracks recursion depth, you lowered it as `call`).

### T7 — CPU gate

**Test:** 60/60 CPU vs wasmi. `make test-cpu` exits 0.

Do not start CUDA until this is green.

### T8 — GPU, N = 1

- `runner.cu`: copy `DevModule` + one `VmState` + one AoS stack/frame buffer to device
- `k_run<<<1,1>>>` calls the **same** `run_instance`
- Copy `VmState` + result slots back
- `max_steps` large enough for `fib-rec(19)` (exponential calls). No resume loop needed yet.

**Test:** `test_fib_gpu_n1` — 60/60, bit-identical to CPU.

### T9 — Stage-1 gate

`make verify` = CPU 60/60 + GPU N=1 60/60. That is the only done signal.

---

## Explicitly skip

| Cut | Why |
|---|---|
| S0 / `docs/TARGET.md` | Soroban recon; does not block fib |
| SoA vs AoS | N=1; coalescing is irrelevant |
| Batch N ∈ {31,32,33,…} | later |
| Fuel / `max_steps` resume | no infinite loops in this file |
| Golden files, mutation, metamorphic, wasm-smith | translator insurance for later |
| Benchmark / `ncu` | speed is not the question |
| i32, memory, globals, `br_if`, host imports | not in `fibonacci.wast` |

---

## Guardrails

1. Do not edit `fibonacci.wast` or the wasmi expected values.
2. Do not widen the opcode set.
3. Do not skip or `#if 0` a failing test.
4. Do not stub `run_instance` differently for host and device.
5. Do not hardcode fibonacci numbers in `src/` or `include/`.
6. Write the failing test first.

---

## Suggested session split

| Session | Tasks | Exit |
|---|---|---|
| 1 | T0 T1 T2 T3 | modules lower; verifier clean |
| 2 | T4 T5 T6 T7 | 60/60 CPU |
| 3 | T8 T9 | 60/60 GPU N=1 |
