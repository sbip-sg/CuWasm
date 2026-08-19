# Stage 2 target — real Soroban contracts

Measured 2026-08-19. Replaces guesses in Stage 1 §2.3 / §2.4.

## Toolchain

| | |
|---|---|
| rustc | 1.88.0 |
| Contract examples | [stellar/soroban-examples](https://github.com/stellar/soroban-examples) **v22.0.1** |
| soroban-sdk | 22.0.1 / 22.0.11 |
| **Target triple** | **`wasm32v1-none`** (Stellar CLI default; no bulk-memory / reference-types / sign-ext by LLVM default) |
| Profile | `release`, `opt-level = "z"`, `panic = "abort"`, LTO, strip |
| Host env | `soroban-env-guest` 22.1.x; guest imports short names from `env.json` |

v23 examples require rustc **1.89**. This machine is 1.88, so recon used v22. Opcode mix is the same class of contract (hello / increment / auth / token).

Build:

```
rustup target add wasm32v1-none
cargo build --target wasm32v1-none --release --manifest-path hello_world/Cargo.toml
```

Artifacts: `contracts/wasm/*.wasm`. Full dump: `docs/contract-wasm-profile.txt`.

## GPU vs CPU test surface (Stage 1 leftover)

`make test-cpu` = 337 checks; `make test-gpu --t8` = 273. The delta is plumbing-only:

- CPU runs `test_parse_assertions` + wasmi `test_oracle` (`#ifndef CUWASM_TEST_GPU`).
- GPU `--t8` skips those and does not run the extra `test_fib(..., nullptr)` pass that `--cpu` runs as t7.

Interpreter fib assertions are the same 60/60 on both.

## Host wasmi policy (from `soroban-env-host` `Vm`)

- Interpreter: **wasmi**. Floating-point modules and `start` functions are **rejected**.
- Imports resolve **only** to `Env` host functions (`HOST_FUNCTIONS` table). Anything else fails.
- Guest functions take/return **64-bit `Val` payloads** (`wasmi::Value::I64`).
- Linear memory export must be named `memory`.
- Fuel is supplied at the host→VM boundary, not at instantiation.

`wasm32v1-none` matches this: **no floats, no SIMD, no bulk-memory, no tables** in the four measured contracts.

## Section inventory (all four)

| | hello | increment | auth | token |
|---|---:|---:|---:|---:|
| bytes | 574 | 582 | 1013 | 7317 |
| memory pages (min, no max) | 17 | 16 | 17 | 17 |
| memory64 / shared | no | no | no | no |
| funcs (defined) | 2 | 5 | 5 | 49 |
| globals | 3 | 3 | 3 | 3 |
| tables / elem | **0** | **0** | **0** | **0** |
| data segments | 1 | 0 | 1 | 1 |
| start | no | no | no | no |
| float / SIMD ops | 0 | 0 | 0 | 0 |
| custom | `contractspecv0`, `contractenvmetav0`, `contractmetav0` | same | same | same |

Globals are the Rust shadow stack: `$__stack_pointer` (mutable i32, already accepted by Stage 1) plus exported `__data_end` / `__heap_base`.

**No `call_indirect`. No `memory.copy` / `fill` / `init`. No `memory.grow` / `memory.size` in these binaries** (size is static). Token uses `br_table` once (already lowered).

## Opcode histogram (union, ranked by token which dominates)

Must-have for token that Stage 1 **does not** execute yet:

| Operator | hello | incr | auth | token | Notes |
|---|---:|---:|---:|---:|---|
| `i64.store` | 4 | 0 | 4 | 74 | linear memory |
| `i64.load` | 1 | 0 | 0 | 53 | |
| `i32.load` | 0 | 0 | 0 | 22 | |
| `i32.store` | 0 | 0 | 0 | 4 | |
| `i32.load8_u` | 0 | 0 | 1 | 1 | |
| `i64.load32_u` | 0 | 0 | 0 | 1 | |
| `call` (imports) | 2 | 7 | 10 | 148 | host boundary |

Already implemented in Stage 1 and used: `local.*`, `i32/i64` const/add/sub/and/or/xor/shl/shr, compares, `block`/`loop`/`br`/`br_if`, `drop`, `select`, `global.get`/`set`, `i64.extend_i32_u`, `i32.wrap_i64`, `return`, `unreachable`, `br_table`.

Sign-extension (`i32.extend8_s` etc.) **did not appear**.

## Imports (full list from the four contracts)

Module letters from `soroban-env-common` `env.json` (199 functions total across 11 modules). Observed **16** unique imports:

| Import | Env module | Function | Args | Used by |
|---|---|---|---:|---|
| `b::i` | buf | `string_new_from_linear_memory` | 2 | hello |
| `b::j` | buf | `symbol_new_from_linear_memory` | 2 | auth, token |
| `v::g` | vec | `vec_new_from_linear_memory` | 2 | hello, auth, token |
| `l::0` | ledger | `has_contract_data` | 2 | increment, auth, token |
| `l::1` | ledger | `get_contract_data` | 2 | increment, auth, token |
| `l::_` | ledger | `put_contract_data` | 3 | increment, auth, token |
| `l::7` | ledger | `extend_contract_data_ttl` | 4 | token |
| `l::8` | ledger | `extend_current_contract_instance_and_code_ttl` | 2 | increment, token |
| `a::0` | address | `require_auth` | 1 | auth, token |
| `x::1` | context | `contract_event` | 2 | token |
| `x::3` | context | `get_ledger_sequence` | 0 | token |
| `i::6` | int | `obj_from_i128_pieces` | 2 | token |
| `i::7` | int | `obj_to_i128_lo64` | 1 | token |
| `i::8` | int | `obj_to_i128_hi64` | 1 | token |
| `m::9` | map | `map_new_from_linear_memory` | 3 | token |
| `m::a` | map | `map_unpack_to_linear_memory` | 4 | token |

Every one of these either **reads/writes guest linear memory** or **touches ledger/auth/events**. Memory must land before any host trampoline can be correct (`string_new_from_linear_memory`, `vec_new_from_linear_memory`, `map_*_linear_memory`).

`env.json` modules: `x` context, `i` int, `m` map, `v` vec, `l` ledger, `d` call, `b` buf, `c` crypto, `a` address, `t` test, `p` prng.

## Stage 2 opcode / feature implications

1. **Linear memory + data segments are unavoidable** (T23–T25).
2. **Host trampoline is the real work** (T28–T32). Do not reimplement ledger/auth.
3. **Tables / `call_indirect` not required** for this corpus (T27 skip unless a later contract needs them).
4. **bulk-memory not present** under `wasm32v1-none` (T26 skip for contracts; spec files still vendored for M1).
5. **`memory.grow` / `memory.size` not used** in these four, but `memory.size` is cheap and spec-gated; implement with load/store.
6. Floats / SIMD remain **never**.

## Contract entrypoints for S2.6

| Contract | WASM | Entrypoint | Why |
|---|---|---|---|
| hello_world | `soroban_hello_world_contract.wasm` | `hello(to: String) -> Vec<String>` | host objects + linear memory, no storage |
| increment | `soroban_increment_contract.wasm` | `increment() -> u32` | instance storage + TTL |
| auth | `soroban_auth_contract.wasm` | `increment(user, value) -> u32` | `require_auth` + persistent storage |
| token | `soroban_token_contract.wasm` | `mint` / `balance` / `transfer` | i128, maps, auth, events |
