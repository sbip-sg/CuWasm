# CuWASM

A WebAssembly interpreter with one execution engine on CPU and CUDA, aimed at Soroban contracts. WASM is lowered on the host; the GPU never decodes modules.

```
.wast / .wasm
  │  Rust wasmparser
  ▼
CuOp + const pool + FuncMeta + integer globals
  │  verify_cuop() — unique stack height at every PC
  ▼
run_instance()   ←── CPU (g++) and GPU (nvcc)
```

`CuOp` is 8 bytes (`op:u16`, `a:u16`, `b:u32`). Locals and the operand stack share one AoS `u64` buffer. CPU and GPU call the same `__host__ __device__` `run_instance()` in `include/cuwasm/interp.h`. Unknown ops return `ST_UNSUPPORTED_OP`; traps are never silent.

**Implemented:** i32/i64 ALU, multi-value, control flow (`br_table`, `OP_UNWIND` for label arity), wide i64 mul / 128-bit add-sub, linear memory and bulk-memory, tables / `call_indirect`, imports as `OP_CALL_HOST`.

**Not implemented:** floating-point, SIMD/v128, `funcref`/`externref`, `start`.

Structured WASM control is flattened on the host (`if` → `OP_BR_IF_NOT`, loops branch to the header, extra values under a label use `OP_UNWIND`).

## Host calls

Every import suspends with `ST_HOSTCALL_PENDING` and a `HostMailbox`.

- **CPU (contract tests):** dispatch to live `soroban-env-host`. Guest objects use even-numbered relative handles, as in ContractVM.
- **GPU (batch):** per-thread `GpuHostState` — object heap (32 slots) plus a 16-entry K/V store. Lookup is a linear byte compare of canonical keys, not guest handles. TTL, `require_auth`, and events are no-ops.

Soroban object Vals are `(handle << 32) | tag` (env-common v22). Storage keys are ScVal contents: byte 0 is `StorageType` (0 temp / 1 persistent / 2 instance), then a SymbolSmall or a 32-byte address pubkey. `I128Small(n)` is `(n << 8) | 11`. The GPU never SHA-256s keys.

## GPU batch

`src/bench.cu`: one CUDA thread = one independent instance (private stack, frames, globals, linear memory, heap, KV). Threads with `tid >= N` return immediately. CUDA events time the kernel only; **TPS = N_ok / kernel_seconds**, and a run fails unless every thread is `ST_OK`.

VRAM is dominated by WASM `min_pages` (Stellar SDK default): **1.00 MB/thread** for increment, **1.06 MB/thread** for hello/token, plus ~5 KB of host state.

## Build

```bash
make verify                 # fibonacci CPU+GPU + gpu-host token checks
make test-contract-tests    # hello, increment, token vs soroban-env-host
make suite                  # wasmi-tests catalog (CPU)
make bench                  # increment scaling
make bench-token            # mint / transfer / balance / scenario

./build/bench contracts/wasm/soroban_token_contract.wasm transfer 8192 64
```

## Correctness


| Suite                                 | Passed | Total | Rate      |
| ------------------------------------- | ------ | ----- | --------- |
| `fibonacci.wast` CPU (`make verify`)  | 357    | 357   | **100%**  |
| `fibonacci.wast` GPU `k_run<<<1,1>>>` | 273    | 273   | **100%**  |
| wasmi-tests, runnable cases           | 1,703  | 1,740 | **97.9%** |
| wasmi-tests, full catalog             | 1,703  | 2,689 | 63.3%     |


Fibonacci is three modules (`iter` / `rec` / `tail`), `n = 0..19`, bit-identical CPU vs GPU vs wasmi. CPU’s 357 includes parse/oracle/plumbing around the same 60 `assert_return`s.

**wasmi-tests** (129 `.wast` files, `make suite`): catalog has 2,689 `assert_return` + `assert_trap`. **949 / 2,689 (35.3%)** are skipped up front (imports, non-integer values, `assert_invalid` / `assert_malformed`). Of the **1,740 runnable** cases: **1,703 pass (97.9%)**, **0 wrong answers**, **37 unsupported (2.1%)** — `torture.wast` (SIMD/float/externref), `audit.wast` (v128), `funcref`/`externref` globals, memory64.

**Contracts** (`make test-contract-tests`): `hello("World")`, `increment` × 2, and token `mint → balance → transfer → balance` match a reference `Host::call`. Token registration uses `create_contract_with_constructor` (the WASM `__constructor` needs admin/decimal/name/symbol).

**GPU token** (seeded alice=`0xA1`×32, bob=`0xB0`×32, admin=`0xAD`×32), all N through 16,384:


| Export                      | Check                                      |
| --------------------------- | ------------------------------------------ |
| `increment`                 | U32(1) = `0x100000004`                     |
| `hello`                     | VecObject `["Hello","World"]`              |
| `mint(alice, 1000)`         | alice `I128Small(1000)` = `0x3e80b`        |
| `balance(alice)`            | `0x3e80b`                                  |
| `transfer(alice, bob, 400)` | alice `0x2580b` (600), bob `0x1900b` (400) |


`compute-sanitizer` memcheck / racecheck / initcheck on increment (N=256): 0 errors.

## Throughput

NVIDIA RTX A4500 (56 SMs, 20 GB), `block_size=64`. Identical work on every thread — an upper bound; mixed keys/paths would diverge more and drop TPS.


| Workload                          | 1,024  | 4,096  | 8,192  | 16,384 | Mem @ 16,384 |
| --------------------------------- | ------ | ------ | ------ | ------ | ------------ |
| increment                         | 18.8 M | 74.0 M | 61.9 M | 90.1 M | 16.5 GB      |
| hello                             | 7.1 M  | 44.2 M | 39.6 M | 52.8 M | 17.6 GB      |
| token `balance` (seeded 1000)     | 2.3 M  | 7.4 M  | 10.2 M | 11.7 M | 17.6 GB      |
| token `mint`                      | 1.2 M  | 3.6 M  | 5.2 M  | 5.7 M  | 17.6 GB      |
| token `transfer` (1000 → 600/400) | 0.70 M | 2.1 M  | 3.1 M  | 3.5 M  | 17.6 GB      |


`token_scenario` (untimed mint, timed transfer on the same KV) matches standalone transfer TPS. Token WASM uses ~10k opcodes and 48 host calls for the full mint/balance/transfer/balance script; `transfer` alone is ~21 host calls.