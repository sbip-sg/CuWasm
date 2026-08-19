# Divergences from Soroban / WASM

- **`return_call`:** implemented for Stage 1 `fibonacci-tail`. Not in `wasm32v1-none` / Soroban guest output. Harmless extra opcode.
- **Host spike crate** (`tools/host-spike`): `soroban-env-host` 22.1.3 `testutils` needs `ed25519-dalek` 2.1.x. Cargo resolves `>=2.0.0` to 3.x which does not compile. `[patch.crates-io]` pins dalek 2.1.1 from the local cargo registry.

- **`br_table`:** lowered to an `i32.eq` / `br_if` chain plus a scratch local (not declared in the source module). `FuncMeta.n_locals` includes the extra local. Fine for N=1 correctness; warp-divergent in Stage 3.
- **Floats / SIMD:** rejected by Soroban wasmi and absent from `wasm32v1-none` contracts. Permanently out of scope.
