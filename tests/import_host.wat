(module
  (import "b" "i" (func $host (param i64 i64) (result i64)))
  (func (export "run") (result i64)
    i64.const 100
    i64.const 5
    call $host)
)
