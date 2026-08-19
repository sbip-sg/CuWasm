# CuWASM Stage 2 — running real Soroban contracts

**Goal: execute a real Soroban token contract end-to-end and match `soroban-env-host`.**
Correctness first. No batching, no SoA, no throughput work.

Version 0.1 · follows Stage 1 (60/60 `fibonacci.wast`, 1703/1740 runnable on wasmi-tests)

---

## 1. Stage 1 assessment

### 1.1 What is solid

**Zero wrong answers across 1740 runnable cases.** This is the number that matters, more
than the 97.9%. The design decision that produced it — unsupported features return
`ST_UNSUPPORTED_OP` loudly rather than degrading — is what makes every other number in the
report trustworthy. Keep that discipline absolutely intact through Stage 2; it is about to
be load-bearing, because Stage 2 adds a large surface of partially-implemented behavior.

`verify_cuop`'s stack-height CFG join is doing real work. `OP_UNWIND` landing specifically
because `replace-result.wast` failed *verify* rather than failing *execution* is the verifier
catching a lowering bug before it became a wrong answer. That is exactly the intended
behavior and it justifies the layer.

Multi-value is already implemented, which matters more than it looks: multivalue is in the
`wasm32-unknown-unknown` default feature set, so real contracts will use it.

### 1.2 Two things to check before building on it

**GPU runs 273 checks vs CPU's 337.** The report doesn't say why. Probably the GPU path
skips parse/oracle plumbing checks, which is benign — but confirm it, because a GPU test
surface that is quietly narrower than CPU is how CPU/GPU drift starts. If the delta is
plumbing-only, assert it explicitly (`EXPECT_EQ(gpu_checks, cpu_checks - parse_checks)`) so
it can't widen unnoticed.

**`br_table` as an `i32.eq` / `br_if` chain is O(n) and uses a scratch local.** Correct for
now, and the right call for a correctness stage. Two consequences to record: it will be a
divergence disaster on GPU in Stage 3 (every lane walks the chain to its own exit), and the
scratch local means `verify_cuop`'s height analysis is reasoning about a local the source
module never declared — confirm `FuncMeta.n_locals` accounts for it, and that
`test_metamorphic_unused_locals` still passes with a `br_table` present.

### 1.3 The wasmi-tests score is now a dead metric

The remaining 37 unsupported cases are 28 `torture.wast` (one kitchen-sink module needing
tables + memory + SIMD + floats + externref), 6 `audit.wast` (v128/memory), 2 funcref
globals, 1 memory64. Implementing everything in Stage 2 moves this by a handful of cases at
most, because SIMD and floats are permanently out of scope — **Soroban forbids floating
point**, so they are not merely deferred, they are never coming.

So: stop tracking the wasmi-tests score as the progress metric. It is saturated. Stage 2
needs two new metrics (§6), and a new correctness corpus (§4.1).

---

## 2. The actual gap — and why it is not mainly instructions

The premise "from testing real contracts we know which instructions to implement" is right
in spirit but will find a smaller answer than expected. The instruction gap is finite and
enumerable in advance:

| Missing | Approx. opcodes | Needed by real contracts? |
|---|---:|---|
| Linear memory: loads/stores all widths, `memory.size`/`grow` | ~25 | **Yes, unavoidably** |
| Data segments (static data init) | — | **Yes** |
| bulk-memory: `memory.copy`/`fill`/`init`, `data.drop` | 4 | Very likely (see §2.1) |
| Tables + `call_indirect` + elem segments | ~2 + infra | Probably — measure |
| Sign-extension ops (`i32.extend8_s` etc.) | 5 | Likely; may already be done |
| Floats / SIMD / externref | ~250 | **Never** |

That is roughly **40 opcodes**. A week of work, mostly mechanical.

**The real blocker is the host boundary.** A Soroban token contract's semantics live almost
entirely in imported env functions. The guest wasm does argument shuffling and calls out for
everything that matters: storage reads and writes, `require_auth`, i128 packing/unpacking,
map and vec operations, symbol construction from linear memory, event emission. Running
`transfer()` correctly means matching `soroban-env-host` on ~30–40 host functions, each with
its own object-store and ledger semantics.

Implementing those from scratch is a multi-month project and is not what Stage 2 should be.
§3 is how to avoid it.

### 2.1 Why linear memory must land first

Not just because Rust needs it — because **the host boundary depends on it**. Several env
functions read and write guest memory directly: `symbol_new_from_linear_memory`,
`string_new_from_linear_memory`, `bytes_copy_to_linear_memory`,
`bytes_new_from_linear_memory`, `map_new_from_linear_memory`, `vec_new_from_linear_memory`,
and their inverses. No host call works until memory works.

Rust also maintains a shadow stack in linear memory via a mutable `$__stack_pointer` i32
global, so even a contract that touches no user data will load and store constantly. Stage
1 already accepts i32/i64 const-initialized globals, which covers `$__stack_pointer`
exactly — that part is already done.

Also note the toolchain hazard from the earlier plan: `wasm32-unknown-unknown` enables
bulk-memory by default as of Rust 1.87 / LLVM 20, while Soroban's wasmi config may disable
it. Whether `memory.copy`/`fill` appear in the binary depends on the exact toolchain and
flags Stellar uses. This is measured in S2.0, not guessed.

---

## 3. Strategy: trampoline first, device-native later

**Do not implement Soroban host functions in Stage 2.** Implement the *protocol* for calling
them, and route every call to the real `soroban-env-host`.

```
guest wasm hits an imported call
        │
        ▼
OP_CALL_HOST → write (fn_id, args[]) to mailbox, set ST_HOSTCALL_PENDING, return
        │
        ▼
driver: sync guest linear memory ↔ host, dispatch to soroban-env-host Env method
        │
        ▼
write result Val into mailbox, status = ST_RUNNING, resume interpreter
```

Why this is the right shape:

- **Host semantics are correct by construction**, because they *are* the reference
  implementation. Zero risk of subtly diverging object-store or ledger behavior.
- **A real token contract runs at the end of S2.4**, not at the end of a host-env rewrite.
- **It produces the measurement the user actually wants** — a ranked profile of which host
  functions real contracts call, and how often, which is the input to Stage 3 scoping.
- **It is the same protocol** that a device-native host tier will use later. Migrating
  individual host functions onto the device in Stage 3 is then a per-function change,
  each one validated against the trampoline it replaces.
- Stage 1's architecture already supports it: `VmState` is fully serializable and the
  interpreter is `max_steps`-bounded with checkpointing, so suspend/resume needs no
  redesign. This was designed in; now it gets used.

The cost is one round trip per host call, which for a correctness stage at N = 1 is
irrelevant. Record it as a known Stage 3 problem and move on.

### 3.1 The dispatch shim is the core deliverable

The one genuinely new piece of engineering: a Rust shim mapping
`(module_name, fn_name, args) → soroban_env_host::Host` method call.

What makes it tractable: **every Soroban env function takes and returns 64-bit `Val`
payloads.** There is no type marshalling in the general case — arguments arrive as `u64`,
get wrapped with `Val::from_payload`, and results unwrap back to `u64`. The shim is
mechanical: roughly 35 functions, ~400 lines.

Two ways to build it, in order of preference:

1. **Generate it** from `soroban-env-common`'s machine-readable env description (`env.json`
   or equivalent — the `Env` trait is macro-generated from it). Locate it in S2.0. Generated
   is strongly preferred: it cannot drift, and it covers functions the first contract
   doesn't happen to call.
2. **Hand-write the measured subset** from the S2.0 import list. Faster to first result,
   but must be paired with a hard error on any unlisted import — never a stub returning
   `Void`.

Known fiddly bits, to be resolved in the S2.1 spike:

- Host functions must execute inside a **contract frame** so storage knows the contract ID.
  Expect to need `Host::with_frame` / a test frame push around each call, or around the
  whole invocation.
- The **budget** will meter every call. Likely need to reset it to unlimited for the
  correctness stage (`budget_ref()`), and note it as a divergence.
- **Storage footprint / ledger snapshot** must be set up before invocation.
  `soroban-env-host`'s test utilities (`register_test_contract_wasm` and friends) are the
  intended path — use them rather than constructing ledger state by hand.
- **Argument construction.** Invoking `transfer(Address, Address, i128)` requires building
  Address and i128 *objects*, which only the host can create. So the driver must construct
  args through the same `Host` instance it services calls with. This is another reason to
  embed the real host rather than fake it.

---

## 4. Stages and gates

Each gate is binary. Correctness-first: **no gate is passed with a known wrong answer**, and
`make verify` (Stage 1's fibonacci gate) must stay green at every checkpoint.

| Stage | Deliverable | Gate |
|---|---|---|
| **S2.0** | Recon — no interpreter code | `docs/TARGET.md`: opcode histogram, full import list, section inventory, feature flags, toolchain triple, for ≥ 3 contracts |
| **S2.1** | Host-embedding spike (timeboxed 2 days) | A Rust binary that constructs a `Host`, registers `token.wasm`, and successfully calls **one** env method directly (e.g. `obj_from_u64`) outside `Host::call` |
| **S2.2** | Linear memory + data segments | Spec-suite memory corpus green (§4.1); `make verify` still green |
| **S2.3** | Tables + `call_indirect` + bulk-memory *(only what S2.0 found)* | Spec-suite `call_indirect` / `memory_copy` / `memory_fill` / `memory_init` green |
| **S2.4** | Import declarations, `OP_CALL_HOST`, trampoline protocol, stub host | A contract runs until its first host call, suspends with the correct `fn_id` and args, and the stub's loud failure names the function |
| **S2.5** | Real dispatch shim → `soroban-env-host` | `hello_world` returns the correct `Val`, matching `Host::call` |
| **S2.6** | Contract corpus | `increment` (storage), `auth` (require_auth), and a **token** (`mint`/`transfer`/`balance`) all match `Host::call` over a scripted scenario |

Effort estimate: S2.0 half a day · S2.1 2 days (timeboxed) · S2.2 4 days · S2.3 2 days ·
S2.4 2 days · S2.5 3 days · S2.6 3 days. Roughly three weeks.

**Order matters.** S2.1 comes before the memory work despite being "later" logically, because
it is the only task with genuine unknown-unknowns. If embedding `soroban-env-host` outside
`Host::call` turns out to be impractical, the whole strategy changes and you want to know
that in week one, not week three. See §7 for the fallback.

### 4.1 New correctness corpus

wasmi-tests will not exercise memory adequately. Vendor these files from the official
`WebAssembly/testsuite` into `tests/spec/`:

`memory.wast · memory_size.wast · memory_grow.wast · memory_redundancy.wast · address.wast ·
align.wast · endianness.wast · load.wast · store.wast · data.wast · traps.wast ·
memory_copy.wast · memory_fill.wast · memory_init.wast · call_indirect.wast · table.wast ·
elem.wast · int_exprs.wast`

These are thousands of assertions and they are the real gate for S2.2/S2.3. Extend the
existing catalog runner to ingest them — the machinery already exists.

Two memory bugs that these files catch and that hand-written tests usually miss:

- **Effective address overflow.** `addr` (u32 from the stack) + `offset` (u32 immediate) can
  exceed 2³². Compute in `u64` before bounds-checking. Computing in u32 wraps and turns an
  out-of-bounds access into a valid in-bounds one — a silent wrong answer, the exact failure
  mode Stage 1 was built to avoid. `address.wast` covers this directly.
- **Alignment immediates are hints, not constraints.** An `align=2` on a misaligned address
  must still work, not trap. `align.wast` covers it.

`endianness.wast` is worth running even though both x86 and NVIDIA GPUs are little-endian —
it catches implementations that accidentally rely on host byte order rather than
implementing WASM's specified little-endian semantics explicitly.

---

## 5. Requirements for agentic coding

Stage 1's guardrails carry forward unchanged. These are additions.

### 5.1 Functional

| ID | Requirement |
|---|---|
| FR-20 | Linear memory is a per-instance flat byte buffer behind a `MemView` abstraction, so Stage 3 can swap the layout without touching `interp.h` |
| FR-21 | Effective addresses are computed in `u64` and bounds-checked before any access |
| FR-22 | Alignment immediates are ignored for correctness (hint only); misaligned access succeeds |
| FR-23 | Data segments initialize memory at instantiation; out-of-bounds segment init traps |
| FR-24 | `OP_CALL_HOST` suspends with `ST_HOSTCALL_PENDING`, writing `fn_id` and args to a per-instance mailbox; the interpreter never interprets a `Val` |
| FR-25 | Resume after a host call restores exactly the pre-call state plus the result on the stack; `test_resume` extends to cover it |
| FR-26 | Any import not in the dispatch shim is a **hard error naming the module and function** — never a stub, never `Void` |
| FR-27 | The driver records a host-call profile per invocation: ordered `(fn_name, arg_count)` plus totals, emitted as JSON |
| FR-28 | The driver records residual unsupported opcodes per contract, emitted as JSON |
| FR-29 | Contract-level differential: `cuwasm_invoke(contract, fn, args) == Host::call(contract, fn, args)` on the returned `Val` **and** on the resulting storage state |
| FR-30 | `make verify` (Stage 1 fibonacci gate) stays green at every commit |

FR-29's second half matters. A token `transfer` that returns the right `Val` while writing
the wrong balance to storage is a wrong answer that a result-only comparison misses entirely.
Compare the post-invocation ledger snapshot, not just the return value.

### 5.2 Guardrails — additions to Stage 1's list

11. **Never stub a host function.** If the shim lacks it, the run fails loudly with the
    function name. A stub that returns `Void` or `0` produces a plausible-looking wrong
    answer, which is the one outcome Stage 1's whole design exists to prevent.
12. **Never implement Soroban host semantics in Stage 2.** Every env call routes to
    `soroban-env-host`. "I implemented `vec_push_back` on the device because it was easy" is
    a rejected diff — it belongs in Stage 3, behind a differential test against the
    trampoline.
13. **Never skip a spec-suite memory case to make the corpus green.** Unsupported is fine
    and is counted; skipped is not.
14. **Never relax `verify_cuop`** to accept memory or call_indirect lowering. Extend it: new
    opcodes get new height/type rules, added to the verifier in the *same commit* as the
    lowering.
15. **Never compare only the return value** for a contract test. Storage state is part of the
    answer.

### 5.3 Tasks

| # | Task | Gate test |
|---|---|---|
| T20 | S2.0 recon → `docs/TARGET.md` for ≥ 3 contracts | human review |
| T21 | Host-embedding spike: `Host` + `register_test_contract_wasm` + one direct env call | `test_host_spike` |
| T22 | Vendor spec-suite memory/table files; extend catalog runner | catalog counts them as runnable |
| T23 | `MemView`, memory section, load/store all widths | `address.wast`, `load.wast`, `store.wast`, `endianness.wast`, `align.wast` |
| T24 | `memory.size` / `memory.grow`, page limits | `memory_size.wast`, `memory_grow.wast` |
| T25 | Data segments + instantiation-time init | `data.wast` |
| T26 | bulk-memory ops *(if S2.0 found them)* | `memory_copy/fill/init.wast` |
| T27 | Tables, elem segments, `call_indirect` with type check *(if needed)* | `call_indirect.wast`, `table.wast`, `elem.wast` |
| T28 | Import section → `OP_CALL_HOST` lowering + `fn_id` table + verifier rules | `test_import_lowering` |
| T29 | Mailbox, suspend/resume protocol, stub host that fails loudly | `test_hostcall_suspend`, `test_resume` extension |
| T30 | Dispatch shim → `soroban-env-host` (generated if possible) | `test_hello_world` vs `Host::call` |
| T31 | Contract driver: arg construction, frame, budget, storage setup | `test_increment` vs `Host::call` incl. storage |
| T32 | Token scenario: mint → balance → transfer → balance | `test_token` vs `Host::call` incl. storage |
| T33 | Profiling output: host-call profile + residual opcode report | JSON emitted, checked into `docs/` |

T21 is deliberately second. Everything after T22 is mechanical; T21 is where the unknowns
are.

---

## 6. Metrics for Stage 2

Replace the wasmi-tests score with these three.

**M1 — Spec memory/table corpus.** Pass / trap-ok / unsupported / fail, same shape as the
Stage 1 catalog. Target: zero fail, unsupported only for float/SIMD cases.

**M2 — Contract coverage.** A table of (contract, entrypoint, scenario) → matches
`Host::call` on return value and storage. Target for S2.6: `hello_world`, `increment`,
`auth`, `token{mint,balance,transfer}` all green.

**M3 — The two profiles.** These are the deliverable that scopes Stage 3, and the direct
answer to "which instructions do we still need":

- *Host-call profile*: ranked `(env module, function, call count)` across the contract
  corpus. The head of this distribution is Stage 3's device-native host tier.
- *Opcode profile*: ranked dynamic opcode counts across the contract corpus, plus any
  residual `ST_UNSUPPORTED_OP` occurrences. This tells you what the GPU dispatch loop
  actually spends its time on, which is very unlikely to resemble `fibonacci.wast`.

Emit both as JSON from the driver, checked in per contract. Do not hand-summarize them.

---

## 7. Risks and fallbacks

**Embedding `soroban-env-host` outside `Host::call` is impractical.** The main risk, hence
the S2.1 timebox. If frame management, budget, or storage setup cannot be driven externally:

*Fallback A — record and replay.* Run the contract normally under `Host::call`, capture the
ordered trace of `(fn_id, args, result, guest-memory writes)`, then replay those results into
CuWASM's mailbox in order. Our interpreter must issue exactly the same call sequence with
the same arguments; divergence in the sequence is itself the bug signal. This validates the
entire guest-side execution — memory, control flow, argument marshalling — without
implementing any host semantics. It requires the same dispatch shim to do the recording, so
it is not a shortcut around T30, but it *is* a shortcut around frame/budget/storage
plumbing. Weaker than the live trampoline (only replays one scripted invocation) but enough
to gate S2.5.

*Fallback B — result-level differential only.* Implement a minimal CPU-side host with our own
object store, and compare only final return value and storage against `Host::call`. Much
weaker oracle, much more work. Last resort.

**Soroban's env interface has no machine-readable description available.** Then the shim is
hand-written from the S2.0 import list, and guardrail 11 becomes critical — the hand-written
subset must fail loudly on anything unlisted, or coverage gaps become wrong answers.

**Contracts need `call_indirect` via panic machinery.** Rust's `core::fmt` and panic paths
introduce indirect calls. If S2.0 finds them, T27 is not optional. Mitigation if it becomes
a time sink: build the test contracts with `panic = "abort"` and minimal formatting, which
is what Soroban contracts do anyway.

**Budget divergence.** Running with an unlimited budget means our execution accepts programs
the real host would reject on metering. Record in `docs/DIVERGENCES.md`; do not attempt to
match Soroban's cost model in Stage 2.

---

## 8. Explicitly deferred to Stage 3

Batching and N > 1 · SoA / interleaved memory layouts · device-native host functions (scoped
from M3) · `br_table` as a real jump table instead of an `eq`/`br_if` chain · register-machine
lowering · per-instance page pool · fuel matched to Soroban's cost model · benchmarking of
any kind.

Nothing in Stage 2 should be justified by a performance argument. If a design choice is
being made for speed, it belongs in Stage 3.
