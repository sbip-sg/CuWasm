# Divergences from Soroban / WASM

- **`return_call`:** implemented for Stage 1 `fibonacci-tail`. Not in `wasm32v1-none` / Soroban guest output. Harmless extra opcode.
- **Host spike crate** (`tools/host-spike`): `soroban-env-host` 22.1.3 `testutils` needs `ed25519-dalek` 2.1.x. Cargo resolves `>=2.0.0` to 3.x which does not compile. `[patch.crates-io]` pins dalek 2.1.1 from the local cargo registry.

- **`br_table`:** lowered to an `i32.eq` / `br_if` chain plus a scratch local (not declared in the source module). `FuncMeta.n_locals` includes the extra local. Fine for N=1 correctness; warp-divergent in Stage 3.
- **Floats / SIMD:** rejected by Soroban wasmi and absent from `wasm32v1-none` contracts. Permanently out of scope. Spec-suite `f32`/`f64` **load/store/const/reinterpret** are lowered as raw little-endian bit copies so integer memory tests in mixed modules still run. Float ALU is not implemented.
- **Host memory cap:** `CUWASM_MEM_MAX_PAGES = 1024` (64MiB). `memory.grow` beyond that returns `-1`. WASM allows host-defined limits.
- **Multi-memory / imported spectest memory:** current `memory_grow.wast` from testsuite `main` is a multi-memory file. Single-memory grow/size coverage is `memory_size.wast` (green).
- **`align.wast`:** upstream file is validator (`assert_malformed`) tests and fails to parse as a whole under wast 227 (`i64 constant out of range`). Alignment-as-hint is covered by `address.wast`.
- **Budget:** Stage 2 runs with unlimited fuel; not matched to Soroban metering.

