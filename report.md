# CuWASM report

CuWASM is a correctness-first WebAssembly interpreter that runs the **same** execution engine on CPU and on a **single CUDA thread**. WASM is never decoded on the GPU: the host lowers modules to a fixed-width bytecode (`CuOp`), a verifier checks stack heights and branch targets, then `run_instance()` executes that bytecode.

Stage 1 is gated on `tests/fibonacci.wast` and a broad integer/control-flow wasmi-tests suite. Stage 2 extends the interpreter to execute real Soroban smart contracts end-to-end against `soroban-env-host`.

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

### Stage 2: Host-boundary architecture

The Soroban WASM execution model treats every `import` as a host function call. CuWASM lowers each import to `OP_CALL_HOST`, which suspends execution with `ST_HOSTCALL_PENDING` and fills a `HostMailbox` struct. The CPU driver loop:

1. Detects `ST_HOSTCALL_PENDING`.
2. Calls a user-supplied `HostFn` callback that reads `mailbox.fn_id` and dispatches to the real `soroban-env-host` host function.
3. Writes the result back into the mailbox.
4. Resumes `run_instance()`.

Guest host objects are tracked via a **relative object table** (`DispatchCtx::relative_objects`): absolute `Val` handles are converted to even-numbered relative handles before being placed on the guest stack, then converted back when passed into host calls. This mirrors ContractVM's handle scheme.

### What is in / out of scope

**Stage 1 (integer interpreter):** i32/i64 arithmetic, compares, shifts, div/rem (with IEEE-style traps), wrap/extend, `drop`/`select`, integer globals, multi-value, `br_table`, wide i64 mul and 128-bit add/sub.

**Stage 2 additions:** linear memory (load/store all widths), data segments, bulk-memory ops (`memory.copy`/`fill`/`init`/`data.drop`), tables and `call_indirect`, import section lowering to `OP_CALL_HOST`, host mailbox suspend/resume, and a live dispatch shim to `soroban-env-host`.

**Not implemented (by design):** floating-point, SIMD/v128, `funcref`/`externref`, `start` section.

---

## Implementation

### Layout

```
include/cuwasm/
  cuop.h      CuOp, opcodes, FuncMeta, DevModule, RunProfile
  vmstate.h   Status, VmState, Frame, stack/frame caps
  layout.h    AoSView / AoSFrameView
  hd.h        HD = __host__ __device__
  interp.h    run_instance<> — the only interpreter
  host.h      HostModule, translate/verify/run_cpu
  gpu.h       run_gpu
  capi.h      C ABI (cuwasm_module_load / run)
src/
  translate.cpp   FFI to the Rust translator
  verify.cpp      stack-height CFG join
  run.cpp         CPU launch + wast helpers
  runner.cu       k_run<<<1,1>>>, HtoD/DtoH of code, stack, frames, globals
  disasm.cpp      CuOp dump
  capi.cpp        C ABI implementation
  main_cpu.cpp    cuwasm-run
tools/            Rust crate cuwasm-tools
  src/lib.rs              wasmparser → CuOp (C ABI)
  src/bin/oracle.rs       wasmi, same JSON as cuwasm-run
  src/bin/wastprep.rs     split .wast modules → .wasm
  src/bin/wast-catalog.rs wasmi-tests → catalog.jsonl
tools/contract-tests/     Rust crate (Stage 2)
  src/lib.rs          run_cuwasm / run_cuwasm_profile
  src/capi.rs         Rust bindings to the C ABI
  src/dispatch.rs     DispatchCtx + live host-function dispatch shim
  src/env_ids.rs      fn_id lookup from docs/soroban-env.json
  src/bin/emit-profiles.rs  emit run profiles to docs/ (FR-27/28)
tests/
  fibonacci.wast   Stage 1 gate (read-only)
  test_main.cpp    CPU/GPU fib + oracle checks
  test_suite.cpp   catalog runner
  wasmi-tests/     wasmi regression suite
docs/
  soroban-env.json          Soroban host-function catalog
  TARGET.md                 Stage 2 recon (contract inventory)
  contract-wasm-profile.txt wasm-profile output for all four contracts
  hello_world-run-profile.json   dynamic profile (FR-27/28)
  increment-run-profile.json
  token-run-profile.json
```

### Interpreter

`run_instance()` is a `max_steps`-bounded fetch/decode/execute loop. A sentinel frame sits at `csp == 1` on entry; returning from it yields `ST_OK` and copies results to the bottom of the stack. Back-edges charge fuel so a runaway `loop` cannot hang.

Call: `Frame{ret_pc, fp, sp_base, n_results}` is pushed; params already sit at `sp_base`. Return copies `n_results` values onto `sp_base` and restores `pc`/`fp`. Tail call (`OP_RETURN_CALL`) moves params onto the current `fp` and jumps without pushing a frame.

Globals are a host `vector<u64>` copied to the device for `k_run`. Only i32/i64 const-initialized globals are accepted.

`RunProfile` (in `cuop.h`) counts executed opcodes and first-seen unsupported opcodes per `run_instance()` call, and is threaded through the C ABI for profiling.

### Translator and verifier

`tools/src/lib.rs` decodes with wasmparser 0.227. Unknown operators fail translation (no silent skip). `verify_cuop` walks each function as a CFG, joins stack height at every PC, and fills `FuncMeta.max_stack`. A height mismatch is a translator bug.

### Host dispatch shim (Stage 2)

`DispatchCtx::dispatch()` in `tools/contract-tests/src/dispatch.rs` handles all imports used by the three Soroban test contracts:

| Host function | Notes |
|---|---|
| `string_new_from_linear_memory` | reads raw bytes, calls `Host::string_new_from_slice` |
| `symbol_new_from_linear_memory` | same, `SymbolObject` |
| `vec_new_from_linear_memory` | reads relative Val array, calls `Host::vec_new_from_slice` |
| `map_new_from_linear_memory` | reads `(ptr,len)` key descriptors + Val array |
| `map_unpack_to_linear_memory` | reads key descriptors, writes relative Val payloads back to guest memory |
| `has_contract_data` / `get_contract_data` / `put_contract_data` | ledger K/V |
| `extend_contract_data_ttl` | 4-arg TTL extension |
| `extend_current_contract_instance_and_code_ttl` | instance TTL |
| `contract_event` | records a contract event |
| `get_ledger_sequence` | returns current ledger sequence number |
| `obj_from_i128_pieces` | hi:i64 + lo:u64 → I128Object |
| `obj_to_i128_lo64` / `obj_to_i128_hi64` | extract i128 halves |
| `require_auth` | delegates to the host's recording-auth manager |

Void host imports always push a void `Val` payload (not `n_results = 0`), matching the Soroban guest ABI.

### Token contract registration

The token contract has a `__constructor(admin, decimal, name, symbol)`. `register_test_contract_wasm` panics because it passes no constructor args. Registration uses `create_contract_with_constructor` with recording auth instead, which correctly runs the constructor.

### How to run

```
make verify              # fibonacci.wast: CPU 357/357 and GPU 273/273
make test-contract-tests # hello_world + increment + token, all green
make emit-profiles       # write run-profile JSON to docs/
make suite               # wasmi-tests catalog + CPU runner
```

Interpreter and test binaries are wrapped in `timeout` to bound infinite loops.

---

## Tests

### Stage 1 gate — `fibonacci.wast`

Three modules (`fibonacci-iter`, `fibonacci-rec`, `fibonacci-tail`), `n = 0..19`:

| Check | Result |
|---|---|
| `assert_return` vs wasmi | **60 / 60** |
| CPU interpreter | **60 / 60** (357 checks including parse/oracle/plumbing) |
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

### Stage 2 — Soroban contract tests

All three contracts execute end-to-end against a live `soroban-env-host`. Return values and storage state are compared to a reference `Host::call` sequence.

| Test | Scenario | Result |
|---|---|---|
| `test_hello_world` | `hello("World")` | ✅ matches `Host::call` |
| `test_increment` | `increment()` × 2 (separate hosts) | ✅ matches `Host::call` (storage correct) |
| `test_token` | `mint` → `balance` → `transfer` → `balance` | ✅ all return values match `Host::call` |

`make test-contract-tests`: **3 / 3 passed**, 0 failed.

### Stage 2 — Run profiles (FR-27 / FR-28)

Dynamic profiles captured from `run_cuwasm_profile()` and written to `docs/`:

| Contract | Scenario | Host calls | Opcodes executed | Unsupported ops |
|---|---|---:|---:|---:|
| `hello_world` | `hello("World")` | 2 | 129 | 0 |
| `increment` | `increment()` × 2 | 7 | 90 | 0 |
| `token` | mint/balance/transfer/balance | 48 | 10,060 | 0 |

Zero unsupported opcodes across all contract executions — every opcode issued by the Soroban-compiled token contract is handled.

**Top host calls for `token`** (ranked by call count):

| Function | Calls |
|---|---:|
| `vec_new_from_linear_memory` | 17 |
| `extend_contract_data_ttl` | 6 |
| `has_contract_data` | 6 |
| `extend_current_contract_instance_and_code_ttl` | 4 |
| `get_contract_data` | 4 |
| `put_contract_data` | 3 |
| `contract_event` | 2 |
| `obj_to_i128_hi64` | 2 |
| `obj_to_i128_lo64` | 2 |
| `require_auth` | 2 |

**Top opcodes for `token`** (ranked by execution count):

| Opcode | Count |
|---|---:|
| `local.get` | 2,336 |
| `i64.const` | 2,291 |
| `local.set` | 925 |
| `i32.add` | 751 |
| `br_if_not` | 659 |
| `br` | 330 |
| `i32.and` | 299 |
| `load` | 271 |
| `store` | 212 |
| `i32.lt_u` | 206 |

Full profiles are in `docs/hello_world-run-profile.json`, `docs/increment-run-profile.json`, and `docs/token-run-profile.json`.

---

## Stage 2 — Multi-threaded GPU Batch Benchmark

### Design

Each CUDA thread runs a fully independent contract instance with private:
- **Stack** (512 slots × 8 B = 4 KB)
- **Frame buffer** (64 frames × 16 B = 1 KB)
- **Globals** (3 × 8 B = 24 B)
- **WASM linear memory** (1 MB per thread — see note below)
- **GPU-side K/V storage** (`GpuStorage`, 16 entries × 24 B + header = 392 B)

No state is shared between threads. Each instance can (and does) mutate its own memory, globals, and storage independently.

Host functions are handled **entirely on-GPU** via `gpu_host_dispatch()` (`include/cuwasm/gpu_host.h`):
- `has_contract_data` / `get_contract_data` / `put_contract_data`: per-thread K/V store lookup/insert
- `extend_contract_data_ttl` / `extend_current_contract_instance_and_code_ttl`: no-op (TTL irrelevant for compute benchmark)
- `require_auth`: stub (always succeeds)
- `contract_event`: no-op

CUDA events (`cudaEventRecord`) measure kernel time only, excluding H2D/D2H memory transfers.

**TPS = N_completed / kernel_seconds**, where N_completed is the number of threads that reached `ST_OK`.

### Why 1 MB per thread?

The 1 MB comes from each contract's **WASM memory section `min_pages` declaration**:

| Contract | min_pages | Bytes per thread |
|---|---:|---:|
| `soroban_hello_world_contract.wasm` | 17 | 1,114,112 B (~1.06 MB) |
| `soroban_increment_contract.wasm` | 16 | 1,048,576 B (1.00 MB) |
| `soroban_token_contract.wasm` | 17 | 1,114,112 B (~1.06 MB) |

This is the Stellar Rust SDK's default allocation — all SDK-compiled contracts reserve 16–17 pages upfront. CuWASM copies only `mem_size = min_pages × 65536` bytes per thread (not the full 64 MB backing buffer). The size cannot be reduced without recompiling the contract with a smaller initial memory.

**Is it configurable?** Yes, at the contract-compilation level: a contract author could pass `--initial-memory=N` to the linker to use fewer pages. CuWASM would then allocate fewer bytes per thread automatically (it uses `hm.mem_size` which tracks `min_pages × 65536`). For these Soroban SDK contracts the minimum is fixed at 16–17 pages.

### Why only `increment` is benchmarked here

The `hello` and `token` contracts require **Soroban object handles** as arguments:
- `hello(to: Vec<Symbol>)` — the `to` arg is a `VecObject`, a heap-allocated host object
- `token::balance(id: Address)` — the `id` arg is an `AddressObject`

These handles are created by the host (`host.string_new_from_slice(...)`, `host.vec_new_from_slice(...)`) and reference entries in the host's object heap. Without a host context on the GPU, there is no valid object heap, so any handle passed will fail the contract's own inline tag-check (triggering `trap_unreachable`).

`increment()` takes **zero arguments** — it only reads/writes its own storage — which is why it works end-to-end with the GPU K/V simulation.

Extending to `token` would require adding a per-thread GPU object heap (strings, vecs, addresses) to `gpu_host.h`. This is future work.

### Hardware

GPU: NVIDIA RTX A4500 (Ampere SM86, 56 SMs, 20 GB GDDR6, ~1.5 GHz boost)

### Correctness verification

Single-thread GPU execution matches CPU trace exactly:
- CPU with simulated K/V store: `has_contract_data → false`, `put_contract_data(key, U32(1))`, `extend_ttl` → return `0x100000004` (U32 val 1)
- GPU thread[0]: result = `0x100000004`, storage = `{key=0xe6a065f41d0e, val=0x100000004}` — **identical**

### Benchmark results (increment contract)

| N threads | Device mem | Kernel ms | TPS | ok/total |
|---:|---:|---:|---:|---:|
| 256 | 257 MB | 0.040 | 6.4 M | 256/256 |
| 1,024 | 1,029 MB | 0.038 | 27.0 M | 1024/1024 |
| 4,096 | 4,117 MB | 0.035 | 118.5 M | 4096/4096 |
| 8,192 | 8,235 MB | 0.045 | 181.8 M | 8192/8192 |
| 16,384 | 16,471 MB | 0.068 | 242 M | 16384/16384 |

All threads complete with `ST_OK`. Device memory is dominated by per-thread WASM linear memory (1 MB × N).

### Why the numbers are high

The `increment` contract executes only **90 WASM opcodes and 3 GPU-side host calls** per invocation. This is an extremely light workload — a single function call that:
1. Checks if a counter key exists (K/V lookup, ~5 cycles)
2. Stores counter=1 (K/V insert, ~10 cycles)
3. Extends TTL (no-op stub, ~2 cycles)

The kernel time (0.068 ms at N=16384) reflects the GPU parallelism: 56 SMs × multiple warps can complete 16384 trivial tasks very quickly. The numbers would be much lower for contracts with heavier computation (e.g., the token contract with 10,060 opcodes and 48 host calls per invocation).

### Block size tuning

Sweep at N=8192:

| block_size | Kernel ms | TPS |
|---:|---:|---:|
| 32 | 0.037 | 223 M |
| **64** | **0.036** | **229 M** |
| 128 | 0.040 | 205 M |
| 256 | 0.046 | 178 M |
| 512 | 0.056 | 145 M |

Smaller blocks (32–64) are optimal because the switch-based interpreter has high warp divergence; fewer threads per block allows the GPU to schedule more blocks per SM, hiding divergence latency.

### Memory scaling limit

At N=16384, per-thread memory is 1,006 KB (dominated by the 1 MB WASM linear memory image), totaling ~16.1 GB out of 20 GB available. This is the practical limit for this GPU; higher N requires either a GPU with more memory or a contract with a smaller initial memory footprint.

### Compute sanitizer results

`compute-sanitizer` was run on the `increment` benchmark (N=256, block_size=64) to check for memory safety issues:

```
compute-sanitizer --tool memcheck  build/bench ... → ERROR SUMMARY: 0 errors
compute-sanitizer --tool racecheck build/bench ... → RACECHECK SUMMARY: 0 hazards (0 errors, 0 warnings)
compute-sanitizer --tool initcheck build/bench ... → ERROR SUMMARY: 0 errors
```

No out-of-bounds accesses, race conditions, or reads from uninitialized memory. Each thread's private arrays (stack, frames, globals, memory, storage) are indexed by `tid × stride` and never overlap.

### Possible future optimizations

1. **GPU object heap**: implement per-thread heap for Soroban object types (strings, vecs, addresses, i128) in `gpu_host.h` to enable benchmarking `hello` and `token` contracts.
2. **SoA memory layout**: transform per-thread arrays from Array-of-Structs to Struct-of-Arrays for better memory coalescing.
3. **Persistent-thread kernel**: amortize kernel launch overhead by having threads process multiple contract instances in sequence.
4. **Reduced linear memory**: contracts that only use a small fraction of their initial pages could use demand-paging or compressed memory; this would directly reduce VRAM usage and allow more threads.
