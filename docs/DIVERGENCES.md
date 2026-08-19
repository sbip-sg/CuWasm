# Divergences from Soroban / WASM

- **`return_call`:** implemented for Stage 1 `fibonacci-tail`. Not in `wasm32v1-none` / Soroban guest output. Harmless extra opcode.
- **Budget:** Stage 2 trampoline runs with an unlimited / reset host budget. Programs the real network would reject on metering may still succeed. Cost model is Stage 3.
- **`br_table`:** lowered to an `i32.eq` / `br_if` chain plus a scratch local (not declared in the source module). `FuncMeta.n_locals` includes the extra local. Fine for N=1 correctness; warp-divergent in Stage 3.
- **Floats / SIMD:** rejected by Soroban wasmi and absent from `wasm32v1-none` contracts. Permanently out of scope.
