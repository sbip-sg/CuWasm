# CuWASM — CUDA WASM interpreter

**Stage 1 spec: pass `fibonacci.wast` on GPU, then benchmark.**
Written as an executable specification for agentic coding.

Version 0.3 · supersedes v0.2 · long-term target is Soroban WASM.

---

## 1. Objective

Run WebAssembly on a GPU, one VM instance per CUDA thread, so that N independent
invocations of the same module execute concurrently.

**Stage 1 is done when the GPU interpreter passes all 60 assertions in
[`wasmi-tests/tests/fibonacci.wast`](https://raw.githubusercontent.com/wasmi-labs/wasmi-tests/refs/heads/main/tests/fibonacci.wast)
and a throughput benchmark against wasmi exists.**

### 1.1 What `fibonacci.wast` is and is not

It is a **bring-up smoke test**: it exercises lowering, dispatch, call frames, batching, and
the benchmark harness with the smallest possible opcode surface.

It is **not a subset of real Soroban contract code**, and the plan must not pretend
otherwise. The file is 100% i64, with no memory section, no globals, no imports, no `br_if`,
no `block`, no `br_table`, no `call_indirect`. Real `wasm32` output from Rust is close to the
opposite of every one of those. See §2.3 for the specifics, and S0 for the recon step that
replaces assumption with measurement.

---

## 2. The target

### 2.1 Stage 1 opcode inventory — complete, sixteen entries

| CuOp | From | Notes |
|---|---|---|
| `OP_I64_CONST` | `i64.const` | 64-bit immediate → constant pool |
| `OP_LOCAL_GET` | `local.get` | `stack[fp + idx]` |
| `OP_LOCAL_SET` | `local.set` | |
| `OP_I64_ADD` / `OP_I64_SUB` | | wrapping |
| `OP_I64_EQ` / `OP_I64_EQZ` | | result i32 |
| `OP_I64_LE_S` / `OP_I64_LT_S` | | **signed** |
| `OP_BR` | `br $continue` | loop back-edge |
| `OP_BR_IF_NOT` | lowered `if` | jump past `then` when condition is 0 |
| `OP_CALL` | `call` | push frame |
| `OP_RETURN_CALL` | `return_call` | **reuse** frame, don't push |
| `OP_RETURN` / `OP_END_FUNC` | | pop frame, move results |
| `OP_UNREACHABLE` | — | trap sentinel, not in the file |

Do not implement anything else speculatively. An unlisted opcode yields
`ST_UNSUPPORTED_OP`, which is a loud test failure — never a silent no-op.

### 2.2 `return_call` — a deliberate divergence

`fibonacci-tail` uses `return_call`, the **tail-call proposal**, which is not MVP and is
almost certainly disabled in Soroban's wasmi config. It is also the one feature in the file
that real Soroban contracts will definitely never contain, since tail calls are not in the
wasm32 default feature set.

**Decision: implement it**, behind `CUWASM_FEATURE_TAIL_CALL` (default on). A clean 60/60
milestone is worth more than subset purity at this stage, and the flag makes the
Soroban-subset build a one-line change. Record it in `docs/DIVERGENCES.md`.

The property worth testing: correct `return_call` runs in **constant frame-stack depth**.
Test at n = 100,000. If it overflows, the implementation silently degraded to
`call`+`return`.

### 2.3 What real Soroban contract WASM actually contains

Soroban contracts are Rust compiled to a wasm32 target. Expect:

- **i32 dominance.** Pointers are 32-bit, so address arithmetic, indices and loop counters
  are i32. Stage 1 implements zero i32 opcodes.
- **Linear memory is mandatory.** Rust maintains a shadow stack in linear memory via a
  mutable `$__stack_pointer` global, with `i32.load`/`i32.store` throughout. The Soroban SDK
  pushes data into host objects to reduce guest memory use, but the shadow stack remains.
  Memory and globals — the two things Stage 1 omits — are unavoidable for any real contract.
- **Host imports carry the semantics.** Every interesting behavior is a call into the env
  module (`i`, `b`, `v`, `m`, `c`, `l`, `a`, `x`, `d`).
- **Dense branching.** `block`, `br_if`, `br_table`, and `call_indirect` (from trait objects
  and function pointers) appear routinely.
- **Data segments** for static data, and Soroban custom sections (`contractspecv0`,
  `contractenvmetav0`).

### 2.4 Target-feature hazard — resolve this in S0

Per the rustc book, `wasm32-unknown-unknown` inherits LLVM's defaults, and the proposals
enabled by default now include multivalue, mutable-globals, reference-types, sign-ext,
nontrapping-fptoint, and — since Rust 1.87 / LLVM 20 — **bulk-memory**.

Soroban's wasmi config very likely *disables* reference-types and bulk-memory. So Soroban
builds must either pin RUSTFLAGS, pin a toolchain, or use a different target. The
`wasm32v1-none` target exists precisely for this case: it enables none of the post-MVP
proposals by default, including bulk-memory, sign-ext, multivalue, and reference types.

**Which triple and flags the Stellar toolchain actually uses directly determines the opcode
set for Stage 2.** Do not guess it — measure it in S0.

---

## 3. Design

### 3.1 Why lower to `CuOp` instead of interpreting WASM bytes directly

This is the load-bearing decision, so the rationale is recorded here rather than assumed.

**LEB128 kills indexed dispatch.** Every immediate — local index, constant, function index,
branch depth, memory offset — is variable-length. Decoding is a serial byte-at-a-time loop
with a data-dependent trip count: a loop inside every instruction fetch, divergence inside
the decode itself, and no way to compute the next PC without decoding the current
instruction. `pc` stops being an index and becomes a scan. With fixed 8-byte ops,
`code[pc]` is one aligned load that broadcasts across a warp when PCs agree.

**`br N` does not know where it is going.** It means "exit N enclosing blocks," and the
target address appears nowhere in the bytecode. At runtime you must either maintain a
control stack — push/pop on every `block`/`loop`/`if`, i.e. per-instance memory traffic on
the most frequent constructs — or scan forward counting nesting depth, which is O(code size)
per branch. `if` without `else` needs the same forward scan. Resolving once on the host
turns `br` into a register assignment.

**Validation already computes what lowering needs.** You cannot execute untrusted WASM
without type-checking it, and validation derives block nesting, block arities, and stack
heights at every branch — exactly the lowering inputs. Lowering is a few hundred lines on
top of a pass you must run regardless.

**It amortizes.** Lowering runs once per module, over N invocations. At N = 65536 the
per-invocation cost vanishes. The alternative is paying decode cost 65536 times, on the
worse processor.

You are not writing a WASM parser: `wasmparser` (Rust) or wabt supplies the decode, and
`translate.cpp` for the Stage-1 subset is roughly 300 lines.

*Alternatives considered.* Keeping raw bytecode with only a branch side-table fixes the
second problem but not LEB128 or variable-width PC — a half measure. Lowering to a
**register machine** instead of a stack machine (what wasmi moved to in 0.32+) executes
fewer instructions with less value-stack traffic; it is a real Stage-2 upgrade if dispatch
overhead dominates the benchmark, but it is more translator complexity than Stage 1 needs.

### 3.2 One interpreter, two compile targets

Write `run_instance()` as a `__host__ __device__` function in a header so it compiles for
CPU and GPU from one source. This gives a fast edit-test loop without a GPU, a third oracle
(CPU-scalar vs GPU vs wasmi), and it makes any CPU/GPU disagreement mean *parallel plumbing
bug*, not *interpreter logic bug*. For an agentic workflow this is the highest-leverage
structural choice in the document.

### 3.3 Data structures

```c
// include/cuwasm/cuop.h
struct alignas(8) CuOp {
    uint16_t op;   // CuOpcode
    uint16_t a;    // local index | arity | result count
    uint32_t b;    // target pc | func index | constant-pool index
};

struct FuncMeta {
    uint32_t code_off, code_len;
    uint16_t n_params, n_results, n_locals, max_stack;
};

struct DevModule {          // read-only, shared by all instances
    const CuOp*     code;
    const uint64_t* consts;
    const FuncMeta* funcs;
    uint32_t n_funcs, code_len;
};
```

```c
// include/cuwasm/vmstate.h
enum Status : uint16_t {
    ST_RUNNING = 0, ST_OK,
    ST_TRAP_UNREACHABLE, ST_TRAP_STACK_OVERFLOW, ST_TRAP_CALL_DEPTH,
    ST_OUT_OF_FUEL, ST_UNSUPPORTED_OP
};

struct VmState { uint32_t pc, sp, fp, csp; int64_t fuel; uint16_t status; };
struct Frame   { uint32_t ret_pc, fp, sp_base; uint16_t n_results; };
```

Every WASM value fits in 64 bits and there are no floats, so one `uint64_t` slot serves the
whole value stack. Locals live at the bottom of the frame in that same stack, so
`local.get n` is `stack[fp + n]`.

### 3.4 Memory layout — the one performance knob in Stage 1

```c
struct AoSView {   // instance i owns a contiguous block; warp stride = CAP, uncoalesced
    uint64_t* base; uint32_t cap, inst;
    HD uint64_t& at(uint32_t i) const { return base[(size_t)inst * cap + i]; }
};

struct SoAView {   // lane-interleaved; threads at equal sp coalesce perfectly
    uint64_t* base; uint32_t n_inst, inst;
    HD uint64_t& at(uint32_t i) const { return base[(size_t)i * n_inst + inst]; }
};
```

`SoAView` should win, since instances running the same module sit at correlated stack
depths. S4 measures whether that's true. Both must pass every correctness test — that
equivalence is itself a test.

### 3.5 Dispatch loop

```c
// include/cuwasm/interp.h   — compiles for host and device
#define HD __host__ __device__ __forceinline__

template <class StackV, class FrameV>
HD void run_instance(const DevModule m, VmState& st, StackV stack, FrameV frames,
                     uint64_t max_steps)
{
    uint32_t pc = st.pc, sp = st.sp, fp = st.fp, csp = st.csp;
    int64_t fuel = st.fuel;

    #define PUSH(v) (stack.at(sp++) = (uint64_t)(v))
    #define POP()   (stack.at(--sp))
    #define TRAP(s) { st.status = (s); goto done; }

    for (uint64_t step = 0; step < max_steps; ++step) {
        const CuOp in = m.code[pc++];
        switch (in.op) {
        case OP_I64_CONST:  PUSH(m.consts[in.b]); break;
        case OP_LOCAL_GET:  PUSH(stack.at(fp + in.a)); break;
        case OP_LOCAL_SET:  stack.at(fp + in.a) = POP(); break;

        case OP_I64_ADD: { uint64_t b = POP(), a = POP(); PUSH(a + b); } break;
        case OP_I64_SUB: { uint64_t b = POP(), a = POP(); PUSH(a - b); } break;
        case OP_I64_LE_S:{ int64_t  b = POP(), a = POP(); PUSH((uint32_t)(a <= b)); } break;
        case OP_I64_LT_S:{ int64_t  b = POP(), a = POP(); PUSH((uint32_t)(a <  b)); } break;
        case OP_I64_EQ:  { uint64_t b = POP(), a = POP(); PUSH((uint32_t)(a == b)); } break;
        case OP_I64_EQZ: { uint64_t a = POP();            PUSH((uint32_t)(a == 0)); } break;

        case OP_BR:
            if (in.b <= pc) {                        // back-edge: charge fuel here only
                if ((fuel -= FUEL_BACKEDGE) <= 0) TRAP(ST_OUT_OF_FUEL);
            }
            pc = in.b; break;

        case OP_BR_IF_NOT:
            if ((uint32_t)POP() == 0) pc = in.b; break;

        case OP_CALL:        /* TODO: push Frame{pc, fp, sp - n_params, n_results};
                                      fp = sp - n_params; zero n_locals; pc = code_off */ break;
        case OP_RETURN_CALL: /* TODO: overwrite current frame's params in place,
                                      reset sp, jump — csp MUST NOT grow */ break;
        case OP_RETURN:
        case OP_END_FUNC:    /* TODO: if csp == 0 -> ST_OK; else move n_results down,
                                      restore pc/fp/sp from Frame */ break;

        case OP_UNREACHABLE: TRAP(ST_TRAP_UNREACHABLE);
        default:             TRAP(ST_UNSUPPORTED_OP);
        }
        if (sp >= STACK_CAP) TRAP(ST_TRAP_STACK_OVERFLOW);
    }
    st.status = ST_RUNNING;      // step budget spent — host relaunches
done:
    st.pc = pc; st.sp = sp; st.fp = fp; st.csp = csp; st.fuel = fuel;   // always checkpoint
}
```

```c
template <class StackV, class FrameV>
__global__ void k_run(DevModule m, Batch b, uint32_t n_inst, uint64_t max_steps) {
    for (uint32_t i = blockIdx.x * blockDim.x + threadIdx.x; i < n_inst;
         i += gridDim.x * blockDim.x) {
        if (b.state[i].status != ST_RUNNING) continue;
        run_instance(m, b.state[i], bind_stack(b, i), bind_frames(b, i), max_steps);
    }
}
```

All state lives in device buffers; nothing survives in registers across a launch. Resuming
an instance is just relaunching with the same pointers, which makes `max_steps` (watchdog
avoidance) free rather than a redesign. Fuel is charged only on back-edges, so straight-line
time is bounded by code size and termination is still guaranteed without a per-instruction
branch.

---

## 4. How the translator is verified

The translator is the only component with no external oracle. wasmi validates and executes,
but its internal IR differs from `CuOp`, so IRs cannot be diffed directly. End-to-end
differential testing is necessary but **not sufficient**: it tests the translator and
interpreter as a composite, so a lowering bug can be masked by a compensating interpreter
bug (an off-by-one branch target against an off-by-one PC increment is the classic pair),
and coverage is only as good as the inputs.

Six layers, weakest to strongest. Layers 1 and 6 are the ones usually skipped, and they are
the ones that matter.

### Layer 0 — Make it small

Use `wasmparser`/wabt for decoding and validation. The hand-written surface is the lowering
itself, ~300 lines for the Stage-1 subset. Fixed-width instructions also mean "branch target
lands on an instruction boundary" is true by construction rather than by check.

### Layer 1 — `verify_cuop()`: the lowered IR re-validates itself

A linear pass over every lowered module, run in CI on every corpus entry, before any
execution. This is the strongest single check and it catches errors that execution tests
miss entirely.

```c
// Runs on every lowered module. Failure = translator bug, always.
bool verify_cuop(const Module& m, Diagnostics& d);
```

Assertions:

1. **Bounds.** Every `OP_BR` / `OP_BR_IF_NOT` target is within the enclosing function's
   `[code_off, code_off + code_len)`. No branch crosses a function boundary. Every
   `OP_CALL` / `OP_RETURN_CALL` index is `< n_funcs`. Every constant-pool index is in range.
2. **Abstract stack-height simulation.** Walk the CuOp stream computing the value-stack
   height at every PC. Because validated WASM is a structured, statically-typed stack
   machine, every PC must have a **single well-defined height regardless of the path
   reaching it**. If two predecessors of a PC disagree, that is a translator bug, full stop.
   This one check kills nearly every branch-arity and drop/keep error.
3. **Abstract type simulation.** The same walk tracking i32/i64 per slot. The original
   module type-checked, so the lowered stream must too. A mismatch is a lowering bug.
4. **No underflow, and `max_stack` is derived, not asserted.** Height never goes negative;
   `FuncMeta.max_stack` is *computed* by this pass rather than guessed elsewhere, so the two
   cannot drift apart.
5. **Termination shape.** Every function's code region ends in `OP_RETURN` or
   `OP_END_FUNC`; no fallthrough past the end.
6. **Reachability.** Every instruction is reachable, or is explicitly marked dead by the
   lowering (unreachable-code elimination after `br`/`return` is legal — but it must be
   deliberate and flagged, not accidental).

Make this a hard gate: `verify_cuop()` failing fails the build even if execution tests pass.

### Layer 2 — Golden disassembly, with a guardrail

Checked-in `CuOp` disassembly for every corpus module. Catches regressions, not initial
correctness — you eyeball it once, then it's frozen.

**The failure mode is an agent regenerating the golden file to make a test pass.** So:
golden files live under `tests/golden/`, regeneration requires `make golden-update` which
prints a diff and refuses to run in CI, and any commit touching them must state why in the
message. Treat a golden diff as a design change requiring review, not a test fix.

### Layer 3 — End-to-end differential vs wasmi

The semantic oracle. `cuwasm(module, args) == wasmi(module, args)` on canonical JSON, across
the corpus and all argument sets. Necessary, insufficient on its own — see the preamble.

### Layer 4 — Metamorphic testing

Apply semantics-preserving transformations to a module; the lowered `CuOp` differs but
results must be bit-identical. This explores translator control-flow paths without needing
new oracles or new expected values.

Transformations: wrap a function body in a redundant `block` · wrap it in *n* nested blocks
and `br` out from depth *n* · insert `br` to the immediately following instruction ·
convert `if C then A` into `block; br_if_not; A; end` by hand · add unused locals before,
between, and after used ones · add a dead function before the tested one, shifting all
function indices.

That last one is worth calling out: index-shift transforms catch off-by-one errors in
function-index resolution that a single-function test can never expose.

### Layer 5 — `wasm-smith` fuzzing, targeted at control flow

Generate random valid modules restricted to the supported feature set, biased toward deep
block nesting and high branch density. For each: `verify_cuop()`, then differential-execute
against wasmi. Minimize failures with `wasm-tools shrink`. This is where coverage actually
comes from; the corpus is far too small to trust alone.

### Layer 6 — Mutation testing: proving the tests have teeth

Everything above tells you whether the translator passes your tests. Mutation testing tells
you whether your tests would notice if it were wrong. Seed a known bug, confirm a **named**
test fails, revert.

| Seeded mutation | Must be caught by |
|---|---|
| Branch target `+1` | `verify_cuop` (height mismatch) or `test_fib_cpu` |
| Branch target `-1` | same |
| Swap the two operands of `i64.sub` | `test_fib_cpu` (rec) |
| Off-by-one on local index resolution | `test_metamorphic_unused_locals` |
| Wrong `n_results` on `OP_RETURN` | `verify_cuop` (height mismatch) |
| Function index off-by-one | `test_metamorphic_index_shift` |
| `OP_RETURN_CALL` lowered as `OP_CALL` | `test_tail_depth` |
| Drop the back-edge fuel charge | `test_fuel` |
| `le_s` lowered as `lt_s` | `test_fib_cpu` (n = 1 boundary) |
| `n_locals` not zeroed on call | `test_fib_cpu` (rec) |

Every row must be caught by a named test. A row that nothing catches is a missing test, and
writing it is the task — not adjusting the table. Keep these as a scripted suite
(`make mutation`) so it reruns as the translator grows.

---

## 5. Stages

Each stage has a single binary gate. A red gate is not partially passed.

| Stage | Deliverable | Gate |
|---|---|---|
| **S0** | Toolchain recon — no code | `docs/TARGET.md` exists with opcode histogram, import list, feature flags, target triple |
| **S1** | Lowering + `verify_cuop` + CPU interpreter | 60/60 on the **CPU** build; `verify_cuop` clean; golden files match; `make mutation` fully caught |
| **S2** | GPU, single instance | 60/60 on GPU at N = 1, bit-identical to CPU |
| **S3** | GPU, batched | 60/60 at N ∈ {1, 31, 32, 33, 1024, 65536}; batch invariance holds; `compute-sanitizer` clean |
| **S4** | Benchmark | CSV + report vs wasmi 1-thread and all-cores, both layout policies, `ncu` counters |

Effort: S0 ≈ half a day · S1 ≈ 3–4 days · S2 ≈ 1 day · S3 ≈ 2 days · S4 ≈ 3 days. If S1
runs past a week the lowering pass is wrong — stop and re-read §3.1 rather than pushing on.

**Stage 1 of the project ends at S4.** Stage 2's scope is written from S0's data, not from
assumptions in this document.

### 5.1 S0 — Toolchain recon (do this first; it does not block S1)

Build one real Soroban contract and count. Ten minutes, and it replaces every guess in §2.3
and §2.4 with measurement.

```bash
stellar contract build          # or: cargo build --target wasm32-unknown-unknown --release

# sections, imports, globals, memory — is there a memory section? which env fns?
wasm-objdump -x hello.wasm | head -60

# opcode histogram, ranked — this IS the Stage 2 opcode set
wasm-objdump -d hello.wasm \
  | grep -oE '\| [a-z0-9_.]+' | sed 's/| //' \
  | sort | uniq -c | sort -rn | head -40

# feature detection: does bulk-memory / reference-types appear?
wasm-tools print hello.wasm | grep -E '^\s*\(import|\(global|\(memory'
wasm-opt --print-features hello.wasm

# and the authoritative answer for the engine side:
#   read the wasmi::Config construction in soroban-env-host/src/vm.rs
```

`docs/TARGET.md` records: target triple, RUSTFLAGS, enabled proposals present in the binary,
the opcode histogram, the full import list, and the wasmi `Config` from the host source.
Repeat for `hello_world`, `increment`, and `token` so the histogram reflects more than one
contract shape.

---

## 6. Requirements for agentic coding

### 6.1 Functional

| ID | Requirement |
|---|---|
| FR-1 | `cuwasm-run <module.wasm> <export> <args...>` prints canonical JSON: `{"status": "...", "results": [...]}` |
| FR-2 | The lowering pass accepts the three `fibonacci.wast` modules with zero unsupported opcodes |
| FR-3 | The interpreter implements exactly the 16 opcodes in §2.1 — no more, no fewer |
| FR-4 | `run_instance()` is `__host__ __device__` and compiles under both `g++` and `nvcc` from one source |
| FR-5 | Both `AoSView` and `SoAView` are instantiated and pass the full test set identically |
| FR-6 | `return_call` executes in constant frame-stack depth (verified at n = 100,000) |
| FR-7 | Any opcode outside §2.1 yields `ST_UNSUPPORTED_OP` and fails the run |
| FR-8 | An instance's result is independent of batch size, its position, and every other instance's input |
| FR-9 | The kernel bounds itself with `max_steps` and resumes correctly across relaunches |
| FR-10 | `make bench` emits CSV with one row per (workload, N, policy), no hand-edited numbers |
| **FR-11** | `verify_cuop()` runs on every lowered module and is a hard build gate |
| **FR-12** | Every mutation in §4 Layer 6 is caught by a named test; `make mutation` reports zero survivors |
| **FR-13** | `FuncMeta.max_stack` is computed by `verify_cuop()`, never set independently |

### 6.2 Non-functional

| ID | Requirement |
|---|---|
| NFR-1 | `make verify` runs every test and exits 0 or non-zero. It is the *only* thing that decides done |
| NFR-2 | Full CPU test suite runs in under 60 s |
| NFR-3 | No dynamic allocation (`malloc`/`new`) inside a kernel |
| NFR-4 | Register usage from `-Xptxas -v`; target ≤ 64/thread, recorded in `docs/RESULTS.md` |
| NFR-5 | `compute-sanitizer --tool memcheck` and `--tool racecheck` clean at N = 65536 |
| NFR-6 | Every test deterministic and seeded; no wall-clock or RNG in correctness paths |

### 6.3 Guardrails — the agent must not

Each of these is a way to make tests green without making the code right. Violating one is a
failed task even if `make verify` passes.

1. **Never modify the oracle or expected values.** `fibonacci.wast` and the wasmi oracle are
   read-only ground truth. If they disagree with the implementation, the implementation is
   wrong.
2. **Never regenerate a golden file to make a test pass.** A golden diff is a design change
   requiring review, not a test fix. `make golden-update` refuses to run in CI.
3. **Never widen the opcode set** beyond §2.1 without an explicit spec change here.
4. **Never delete, skip, `#if 0`, or mark-expected-failure a failing test.**
5. **Never weaken `verify_cuop()`** to accommodate translator output. The verifier encodes
   what "correct lowering" means; if it fires, the lowering is wrong.
6. **Never stub `run_instance` differently for host and device.** One body, two targets.
7. **Never hardcode fibonacci values** in `src/` or `include/`.
8. **Never let a trap be silent.** Every non-`ST_OK` status reaches the JSON output.
9. **Never edit the mutation table (§4 Layer 6) to remove an uncaught row.** Write the
   missing test instead.
10. **Write the failing test first.** Commit red, then make it green.

### 6.4 Acceptance test set

| Test | Asserts |
|---|---|
| `test_verify_cuop` | `verify_cuop` clean on all corpus modules; and *fires* on 6 hand-corrupted IR fixtures |
| `test_lowering` | Golden `CuOp` disassembly for all three modules |
| `test_fib_cpu` | 60/60, CPU build |
| `test_fib_gpu_n1` | 60/60, GPU, N = 1, bit-identical to CPU |
| `test_tail_depth` | `fibonacci-tail(100000)` completes; peak `csp` ≤ 2 |
| `test_rec_depth` | `fibonacci-rec(25)` correct; peak `csp` tracks recursion depth |
| `test_fuel` | Infinite-loop module halts `ST_OUT_OF_FUEL`; checkpointed PC resumes correctly |
| `test_unsupported` | A module using `i32.add` returns `ST_UNSUPPORTED_OP` |
| `test_metamorphic_*` | The six transforms in §4 Layer 4, results unchanged |
| `test_batch_invariance` | Probe at position {0, mid, N−1} for N ∈ {1,31,32,33,1024,65536}, adversarial neighbours (n = 0, n = 19, infinite loop), all bit-identical to solo |
| `test_layout_equivalence` | Everything above re-run under `SoAView` |
| `test_resume` | `max_steps = 10` forces many relaunches; results unchanged |

`test_verify_cuop` must include the negative cases. A verifier that has never been observed
to fire is not known to work.

`test_batch_invariance` is the highest-value test in the repo — roughly forty lines, and it
kills essentially every cross-instance indexing bug, view-binding error, and shared-state
leak.

### 6.5 Task breakdown

One agent session per task. Done = named test green + `make verify` green + no guardrail
violated.

| # | Task | Test that must go red first |
|---|---|---|
| T0 | S0 recon → `docs/TARGET.md` | none — a document, reviewed by a human |
| T1 | `.wast` parser → module bytes + assertion list (`wast` crate or `wast2json`) | `test_parse_assertions` (expects 60) |
| T2 | wasmi oracle binary emitting the FR-1 JSON schema | `test_oracle_fib` |
| T3 | `CuOp` types + lowering pass for the 16 opcodes | `test_lowering` |
| T4 | **`verify_cuop()`** — bounds, height, type, max_stack, termination | `test_verify_cuop` incl. corrupted fixtures |
| T5 | `CuOp` disassembler + golden files | golden output readable |
| T6 | CPU `run_instance`: arithmetic, locals, `br`, `br_if_not` | `test_fib_cpu` (iter) |
| T7 | Frames: `call`, `return`, `end_func` | `test_fib_cpu` (rec), `test_rec_depth` |
| T8 | `return_call` with frame reuse | `test_fib_cpu` (tail), `test_tail_depth` |
| T9 | Fuel + `max_steps` checkpoint/resume | `test_fuel` |
| T10 | **Metamorphic transform harness** | `test_metamorphic_*` |
| T11 | **Mutation suite** (`make mutation`) | every row in §4 Layer 6 caught |
| T12 | CUDA build, `k_run`, N = 1 | `test_fib_gpu_n1` |
| T13 | Batched launch, per-instance views, `SoAView` | `test_batch_invariance`, `test_layout_equivalence` |
| T14 | Benchmark harness + CSV | `make bench` produces rows |

T1–T11 need no GPU. By the time CUDA enters at T12 the interpreter semantics are already
proven and the translator is verified three ways.

### 6.6 Repo layout and commands

```
cuwasm/
├── Makefile
├── include/cuwasm/
│   ├── cuop.h, vmstate.h, layout.h
│   └── interp.h              # run_instance<> — HOST AND DEVICE, one body
├── src/
│   ├── translate.cpp         # wasm -> CuOp lowering
│   ├── verify.cpp            # verify_cuop()
│   ├── disasm.cpp
│   ├── main_cpu.cpp
│   └── runner.cu             # k_run + launch/resume driver
├── tools/
│   ├── oracle/               # Rust: wasmi + JSON schema
│   ├── metamorph/            # semantics-preserving transforms
│   └── mutate/               # seeded-bug harness
├── tests/
│   ├── golden/               # CuOp disassembly — review-gated
│   ├── fixtures/             # deliberately corrupted IR for verify negatives
│   ├── fibonacci.wast        # vendored, read-only
│   └── test_*.cpp / .cu
├── bench/
└── docs/{TARGET.md, DIVERGENCES.md, RESULTS.md}
```

```
make verify         # everything; the only gate that counts
make test-cpu       # no GPU needed — T1..T11
make test-gpu       # T12..T13
make mutation       # seeded-bug suite; zero survivors required
make golden-update  # refuses to run in CI
make bench          # emits bench/results.csv
```

---

## 7. Benchmark (S4)

The three fibonacci variants are three different stress profiles.

| Workload | Profile |
|---|---|
| `fib-iter(n)` | tight loop — ALU and dispatch overhead |
| `fib-rec(n)` | call/return — frame-stack traffic |
| `fib-tail(n)` | tail calls — branch-dominated, constant stack |
| `fib-iter(n_i)`, `n_i` varying per instance | **divergence sweep** — the real experiment |

The divergence sweep is the finding worth plotting: give instance *i* a different `n` so loop
trip counts differ across a warp, and sweep the spread from zero (all instances equal) to
wide (uniform over 0..10⁶). Expect a cliff; locating it is the point.

Sweep N ∈ {1, 256, 4096, 65536, 262144} × {AoS, SoA} × four workloads.

Baselines: wasmi single-thread, and wasmi across all physical cores. Not a JIT —
interpreter-versus-JIT is a different claim, and saying so is better than omitting it.

Metrics: invocations/s, WASM instructions/s, plus `ncu` counters for warp execution
efficiency (`smsp__thread_inst_executed_per_inst_executed.ratio`), achieved occupancy, and
L2 hit rate on the shared instruction stream.

Report end-to-end wall-clock including H2D and lowering, and state the batch size at which
GPU overtakes CPU. Single-invocation latency will be worse than CPU — say so plainly;
volunteering it is what makes the throughput number credible.

---

## 8. Deferred (not Stage 1)

Scope for these comes from `docs/TARGET.md`, not from this document: the i32 opcode family ·
linear memory with an 8-byte lane-interleaved layout · globals and the Rust shadow stack ·
`br_if` / `br_table` / `call_indirect` / `select` · bulk-memory ops if S0 finds them ·
the full WebAssembly spec test suite · Soroban host functions and the `Val` tagged-value
ABI · exit-resume trampoline for CPU-serviced host calls · lazy page pool for large
linear memories · register-machine lowering if S4 shows dispatch-bound behavior.

The design accommodates each without restructuring. Adding one early is the most likely way
this slips.
