# CuWASM

A correctness-first WebAssembly interpreter that runs on **CPU and CUDA GPU** with a single shared execution engine, targeting Soroban smart contracts.

## Design

```
.wasm
  │  Rust (wasmparser)
  ▼
CuOp bytecode + const pool + FuncMeta + globals
  │  verify_cuop() — stack-height CFG join
  ▼
run_instance()   ←── CPU (g++) and GPU (nvcc, __host__ __device__)
```

**`CuOp`** is 8 bytes: `op:u16, a:u16, b:u32`.  
**`run_instance()`** is the single interpreter body shared between CPU and GPU.  
**`HostMailbox`** suspends GPU execution for host calls; on CPU, these are dispatched to `soroban-env-host`. On GPU (batch mode), they are handled by a per-thread K/V storage simulation.

## Stage 2: Multi-thread GPU batch benchmark

Each CUDA thread runs a fully independent contract instance with **private** stack, frames, globals, WASM linear memory, and K/V storage. No data sharing between threads.

Host functions (`has/get/put_contract_data`, TTL extensions, auth, events) are simulated on-GPU — no host boundary crossing during the kernel.

### Results (increment contract, RTX A4500 20GB)

| N threads | Device mem | Kernel ms | TPS |
|---:|---:|---:|---:|
| 256 | 257 MB | 0.040 | 6.4 M |
| 1,024 | 1,029 MB | 0.038 | 27.0 M |
| 4,096 | 4,117 MB | 0.035 | 118.5 M |
| 8,192 | 8,235 MB | 0.036 | 229 M |
| 16,384 | 16,471 MB | 0.068 | 242 M |

All threads complete with `ST_OK`. Correctness verified: GPU thread[0] result matches CPU trace exactly.

The `increment` contract executes 90 WASM opcodes + 3 GPU-side host calls per invocation — a very light workload. Heavier contracts (e.g., token with 10K+ opcodes) would show lower TPS.

## Building and testing

```bash
make verify              # fibonacci: CPU 357/357, GPU 273/273
make test-contract-tests # 3/3: hello_world, increment, token
make bench               # increment scaling sweep
make build/bench         # build benchmark binary only

# Custom benchmark
./build/bench contracts/wasm/soroban_increment_contract.wasm increment 8192 64
```

## What is implemented

- i32/i64 integer ALU, multi-value, control flow, `br_table`, wide mul
- Linear memory (all load/store widths), data segments, bulk-memory ops
- Tables and `call_indirect`
- Import dispatch → `OP_CALL_HOST` + mailbox suspend/resume
- Live dispatch shim to `soroban-env-host` (CPU tests)
- GPU-side storage simulation (batch benchmark)
- N-thread GPU batch kernel with per-thread private state

**Not implemented:** floating-point, SIMD/v128, `funcref`/`externref`, `start` section.

See `report.md` for detailed design, test results, and benchmark analysis.
