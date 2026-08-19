# WebAssembly spec-suite corpus (Stage 2)

Vendored from [WebAssembly/testsuite](https://github.com/WebAssembly/testsuite) on **2026-08-19** (`main`).

These files are the M1 gate for linear memory, data segments, bulk-memory, and tables. The catalog runner treats invoke/`assert_return`/`assert_trap`/`assert_unlinkable` as **runnable**. `assert_invalid` / `assert_malformed` stay catalog-skip (validator tests; we decode with wasmparser). Imported-spectest modules stay runnable: CuWASM must reject them as unsupported, not skip them.

| File | Why |
|---|---|
| `memory.wast` | memory section, `memory.size` |
| `memory_size.wast` / `memory_grow.wast` | page limits |
| `memory_redundancy.wast` | aliasing |
| `address.wast` | u64 effective address + OOB |
| `align.wast` | alignment is a hint |
| `endianness.wast` | little-endian stores |
| `load.wast` / `store.wast` | all widths |
| `data.wast` | data segments / unlinkable OOB init |
| `traps.wast` | OOB traps |
| `memory_copy.wast` / `memory_fill.wast` / `memory_init.wast` | bulk-memory |
| `call_indirect.wast` / `table.wast` / `elem.wast` | tables |
| `int_exprs.wast` | integer edge cases |

Run: `make spec-suite` (timeout 60s).
