//! WASM → CuOp lowering. Decode with wasmparser; emit fixed-width ops.

mod env_fn_id;

use libc::c_char;
use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::ptr;
use wasmparser::{
    BlockType, CompositeInnerType, DataKind, ElementItems, ElementKind, ExternalKind, MemArg,
    Operator, Parser, Payload, TypeRef, ValType,
};

pub const OP_UNREACHABLE: u16 = 0;
pub const OP_I64_CONST: u16 = 1;
pub const OP_LOCAL_GET: u16 = 2;
pub const OP_LOCAL_SET: u16 = 3;
pub const OP_I64_ADD: u16 = 4;
pub const OP_I64_SUB: u16 = 5;
pub const OP_I64_EQ: u16 = 6;
pub const OP_I64_EQZ: u16 = 7;
pub const OP_I64_LE_S: u16 = 8;
pub const OP_I64_LT_S: u16 = 9;
pub const OP_BR: u16 = 10;
pub const OP_BR_IF_NOT: u16 = 11;
pub const OP_CALL: u16 = 12;
pub const OP_RETURN_CALL: u16 = 13;
pub const OP_RETURN: u16 = 14;
pub const OP_END_FUNC: u16 = 15;
pub const OP_DROP: u16 = 16;
pub const OP_SELECT: u16 = 17;
pub const OP_I32_EQZ: u16 = 18;
pub const OP_I32_EQ: u16 = 19;
pub const OP_I32_NE: u16 = 20;
pub const OP_I32_LT_S: u16 = 21;
pub const OP_I32_LT_U: u16 = 22;
pub const OP_I32_LE_S: u16 = 23;
pub const OP_I32_LE_U: u16 = 24;
pub const OP_I32_GT_S: u16 = 25;
pub const OP_I32_GT_U: u16 = 26;
pub const OP_I32_GE_S: u16 = 27;
pub const OP_I32_GE_U: u16 = 28;
pub const OP_I32_ADD: u16 = 29;
pub const OP_I32_SUB: u16 = 30;
pub const OP_I32_MUL: u16 = 31;
pub const OP_I32_AND: u16 = 32;
pub const OP_I32_OR: u16 = 33;
pub const OP_I32_XOR: u16 = 34;
pub const OP_I32_DIV_S: u16 = 35;
pub const OP_I32_DIV_U: u16 = 36;
pub const OP_I32_REM_S: u16 = 37;
pub const OP_I32_REM_U: u16 = 38;
pub const OP_I32_SHL: u16 = 39;
pub const OP_I32_SHR_S: u16 = 40;
pub const OP_I32_SHR_U: u16 = 41;
pub const OP_I32_WRAP_I64: u16 = 42;
pub const OP_I64_NE: u16 = 43;
pub const OP_I64_LT_U: u16 = 44;
pub const OP_I64_LE_U: u16 = 45;
pub const OP_I64_GT_S: u16 = 46;
pub const OP_I64_GT_U: u16 = 47;
pub const OP_I64_GE_S: u16 = 48;
pub const OP_I64_GE_U: u16 = 49;
pub const OP_I64_MUL: u16 = 50;
pub const OP_I64_AND: u16 = 51;
pub const OP_I64_OR: u16 = 52;
pub const OP_I64_XOR: u16 = 53;
pub const OP_I64_DIV_S: u16 = 54;
pub const OP_I64_DIV_U: u16 = 55;
pub const OP_I64_REM_S: u16 = 56;
pub const OP_I64_REM_U: u16 = 57;
pub const OP_I64_SHL: u16 = 58;
pub const OP_I64_SHR_S: u16 = 59;
pub const OP_I64_SHR_U: u16 = 60;
pub const OP_I64_EXTEND_I32_S: u16 = 61;
pub const OP_I64_EXTEND_I32_U: u16 = 62;
pub const OP_GLOBAL_GET: u16 = 63;
pub const OP_GLOBAL_SET: u16 = 64;
pub const OP_UNWIND: u16 = 65;
pub const OP_I64_MUL_WIDE_S: u16 = 66;
pub const OP_I64_MUL_WIDE_U: u16 = 67;
pub const OP_I64_ADD128: u16 = 68;
pub const OP_I64_SUB128: u16 = 69;
pub const OP_LOAD: u16 = 70;
pub const OP_STORE: u16 = 71;
pub const OP_MEMORY_SIZE: u16 = 72;
pub const OP_MEMORY_GROW: u16 = 73;
pub const OP_MEMORY_COPY: u16 = 74;
pub const OP_MEMORY_FILL: u16 = 75;
pub const OP_MEMORY_INIT: u16 = 76;
pub const OP_DATA_DROP: u16 = 77;
pub const OP_CALL_HOST: u16 = 78;
pub const OP_CALL_INDIRECT: u16 = 79;
pub const OP_CLZ: u16 = 80;

const WASM_PAGE: u32 = 65536;
const CUWASM_MEM_MAX_PAGES: u64 = 1024;
const TABLE_NULL: u32 = 0xFFFF_FFFF;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CuOpC {
    pub op: u16,
    pub a: u16,
    pub b: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FuncMetaC {
    pub code_off: u32,
    pub code_len: u32,
    pub n_params: u16,
    pub n_results: u16,
    pub n_locals: u16,
    pub max_stack: u16,
}

#[repr(C)]
pub struct TranslateOut {
    pub code: *mut CuOpC,
    pub n_code: u32,
    pub consts: *mut u64,
    pub n_consts: u32,
    pub funcs: *mut FuncMetaC,
    pub n_funcs: u32,
    pub export_names: *mut *mut c_char,
    pub export_idxs: *mut u32,
    pub n_exports: u32,
    pub globals: *mut u64,
    pub n_globals: u32,
    pub memory: *mut u8,
    pub mem_size: u32,
    pub mem_max: u32,
    pub data_blob: *mut u8,
    pub data_blob_len: u32,
    pub data_off: *mut u32,
    pub data_len: *mut u32,
    pub data_live: *mut u8,
    pub n_data: u32,
    pub table: *mut u32,
    pub table_len: u32,
    pub func_typeidx: *mut u32,
    pub type_fp: *mut u64,
    pub n_types: u32,
    pub n_host_imports: u32,
    pub host_fn_id: *mut u32,
    pub host_import_mod: *mut *mut c_char,
    pub host_import_name: *mut *mut c_char,
    pub host_import_env: *mut *mut c_char,
    pub err: *mut c_char,
}

impl Default for TranslateOut {
    fn default() -> Self {
        Self {
            code: ptr::null_mut(),
            n_code: 0,
            consts: ptr::null_mut(),
            n_consts: 0,
            funcs: ptr::null_mut(),
            n_funcs: 0,
            export_names: ptr::null_mut(),
            export_idxs: ptr::null_mut(),
            n_exports: 0,
            globals: ptr::null_mut(),
            n_globals: 0,
            memory: ptr::null_mut(),
            mem_size: 0,
            mem_max: 0,
            data_blob: ptr::null_mut(),
            data_blob_len: 0,
            data_off: ptr::null_mut(),
            data_len: ptr::null_mut(),
            data_live: ptr::null_mut(),
            n_data: 0,
            table: ptr::null_mut(),
            table_len: 0,
            func_typeidx: ptr::null_mut(),
            type_fp: ptr::null_mut(),
            n_types: 0,
            n_host_imports: 0,
            host_fn_id: ptr::null_mut(),
            host_import_mod: ptr::null_mut(),
            host_import_name: ptr::null_mut(),
            host_import_env: ptr::null_mut(),
            err: ptr::null_mut(),
        }
    }
}

#[derive(Debug)]
pub struct HostModule {
    pub code: Vec<CuOpC>,
    pub consts: Vec<u64>,
    pub funcs: Vec<FuncMetaC>,
    pub exports: Vec<(String, u32)>,
    pub globals: Vec<u64>,
    pub memory: Vec<u8>,
    pub mem_size: u32,
    pub mem_max: u32,
    pub data_blob: Vec<u8>,
    pub data_off: Vec<u32>,
    pub data_len: Vec<u32>,
    pub data_live: Vec<u8>,
    pub table: Vec<u32>,
    pub func_typeidx: Vec<u32>,
    pub type_fp: Vec<u64>,
    pub n_host_imports: u32,
    pub host_fn_id: Vec<u32>,
    pub host_import_mod: Vec<String>,
    pub host_import_name: Vec<String>,
    pub host_import_env: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CtrlKind {
    Func,
    Block,
    Loop,
    If,
}

struct Ctrl {
    kind: CtrlKind,
    start: u32,
    br_if_not: Option<u32>,
    else_br: Option<u32>,
    patches: Vec<u32>,
    has_else: bool,
    base: i32,
    n_params: u16,
    n_results: u16,
    dead_on_entry: bool,
}

fn type_fingerprint(params: &[ValType], results: &[ValType]) -> Result<u64, String> {
    if params.len() > 8 || results.len() > 8 {
        return Err("too many params/results for type fingerprint".into());
    }
    let tag = |t: ValType| -> Result<u64, String> {
        Ok(match t {
            ValType::I32 => 0,
            ValType::I64 => 1,
            ValType::F32 => 2,
            ValType::F64 => 3,
            _ => return Err(format!("unsupported valtype {:?}", t)),
        })
    };
    let mut fp = params.len() as u64 | ((results.len() as u64) << 8);
    for (i, t) in params.iter().enumerate() {
        fp |= tag(*t)? << (16 + 2 * i);
    }
    for (i, t) in results.iter().enumerate() {
        fp |= tag(*t)? << (32 + 2 * i);
    }
    Ok(fp)
}

fn load_flags(nbytes: u16, signed: bool, i64dest: bool) -> u16 {
    nbytes | (if signed { 0x10 } else { 0 }) | (if i64dest { 0x20 } else { 0 })
}

fn check_memarg(memarg: MemArg) -> Result<u32, String> {
    if memarg.memory != 0 {
        return Err("multi-memory not supported".into());
    }
    if memarg.offset > u32::MAX as u64 {
        return Err("memory64 offset".into());
    }
    Ok(memarg.offset as u32)
}

fn eval_i32_expr<'a, I>(ops: I) -> Result<u32, String>
where
    I: IntoIterator<Item = Result<Operator<'a>, wasmparser::BinaryReaderError>>,
{
    let mut st: Vec<i32> = Vec::new();
    for op in ops {
        let op = op.map_err(|e| format!("constexpr: {e}"))?;
        match op {
            Operator::I32Const { value } => st.push(value),
            Operator::I32Add => {
                let b = st.pop().ok_or("constexpr underflow")?;
                let a = st.pop().ok_or("constexpr underflow")?;
                st.push(a.wrapping_add(b));
            }
            Operator::I32Sub => {
                let b = st.pop().ok_or("constexpr underflow")?;
                let a = st.pop().ok_or("constexpr underflow")?;
                st.push(a.wrapping_sub(b));
            }
            Operator::I32Mul => {
                let b = st.pop().ok_or("constexpr underflow")?;
                let a = st.pop().ok_or("constexpr underflow")?;
                st.push(a.wrapping_mul(b));
            }
            Operator::End => {}
            Operator::GlobalGet { .. } => return Err("imported global in offset expr".into()),
            other => return Err(format!("unsupported constexpr {other:?}")),
        }
    }
    st.last()
        .copied()
        .map(|v| v as u32)
        .ok_or_else(|| "empty constexpr".into())
}

fn intern(consts: &mut Vec<u64>, v: u64) -> u32 {
    if let Some(i) = consts.iter().position(|&x| x == v) {
        return i as u32;
    }
    consts.push(v);
    (consts.len() - 1) as u32
}

fn emit(code: &mut Vec<CuOpC>, op: u16, a: u16, b: u32) -> u32 {
    let pc = code.len() as u32;
    code.push(CuOpC { op, a, b });
    pc
}

fn patch_b(code: &mut [CuOpC], pc: u32, b: u32) {
    code[pc as usize].b = b;
}

fn only_int(ts: &[ValType]) -> Result<(), String> {
    for t in ts {
        match *t {
            ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64 => {}
            _ => return Err(format!("unsupported valtype {:?}", t)),
        }
    }
    Ok(())
}

fn resolve_br(ctrl: &[Ctrl], depth: u32) -> Result<usize, String> {
    let i = ctrl
        .len()
        .checked_sub(1 + depth as usize)
        .ok_or_else(|| format!("br depth {} out of range", depth))?;
    Ok(i)
}

fn consume(h: &mut i32, dead: bool, pops: i32, pushes: i32) -> Result<(), String> {
    if dead {
        return Ok(());
    }
    if *h < pops {
        return Err("operand stack underflow".into());
    }
    *h = *h - pops + pushes;
    Ok(())
}

fn block_arity(
    ty: BlockType,
    func_types: &[(Vec<ValType>, Vec<ValType>)],
) -> Result<(u16, u16), String> {
    match ty {
        BlockType::Empty => Ok((0, 0)),
        BlockType::Type(t) => {
            if t != ValType::I32 && t != ValType::I64 {
                return Err(format!("unsupported block valtype {:?}", t));
            }
            Ok((0, 1))
        }
        BlockType::FuncType(idx) => {
            let i = idx as usize;
            if i >= func_types.len() {
                return Err("block type index oob".into());
            }
            Ok((func_types[i].0.len() as u16, func_types[i].1.len() as u16))
        }
    }
}

fn func_arity(
    function_index: u32,
    func_types: &[(Vec<ValType>, Vec<ValType>)],
    func_typeidx: &[u32],
) -> Result<(u16, u16), String> {
    let i = function_index as usize;
    if i >= func_typeidx.len() {
        return Err("call function index oob".into());
    }
    let t = func_typeidx[i] as usize;
    if t >= func_types.len() {
        return Err("call type index oob".into());
    }
    Ok((func_types[t].0.len() as u16, func_types[t].1.len() as u16))
}

fn label_arity(c: &Ctrl) -> i32 {
    if c.kind == CtrlKind::Loop {
        c.n_params as i32
    } else {
        c.n_results as i32
    }
}

fn emit_unwind(
    code: &mut Vec<CuOpC>,
    h: i32,
    c: &Ctrl,
    fn_params: u16,
    fn_locals: u16,
) -> Result<i32, String> {
    let dest_h = c.base + label_arity(c);
    if h < dest_h {
        return Err("branch stack underflow".into());
    }
    if h > dest_h {
        let n_keep = label_arity(c) as u16;
        let dest_rel = fn_params as u32 + fn_locals as u32 + dest_h as u32;
        emit(code, OP_UNWIND, n_keep, dest_rel);
    }
    Ok(dest_h)
}

fn push_ctrl(
    ctrl: &mut Vec<Ctrl>,
    kind: CtrlKind,
    start: u32,
    np: u16,
    nr: u16,
    h: i32,
    dead: bool,
) -> Result<(), String> {
    if !dead && h < np as i32 {
        return Err("block params underflow".into());
    }
    let base = if dead { 0 } else { h - np as i32 };
    ctrl.push(Ctrl {
        kind,
        start,
        br_if_not: None,
        else_br: None,
        patches: Vec::new(),
        has_else: false,
        base,
        n_params: np,
        n_results: nr,
        dead_on_entry: dead,
    });
    Ok(())
}

fn emit_jump(code: &mut Vec<CuOpC>, ctrl: &mut [Ctrl], idx: usize) {
    if ctrl[idx].kind == CtrlKind::Loop {
        let start = ctrl[idx].start;
        emit(code, OP_BR, 0, start);
    } else {
        let pc = emit(code, OP_BR, 0, 0);
        ctrl[idx].patches.push(pc);
    }
}

fn lower_operators(
    ops: Vec<Operator<'_>>,
    n_params: u16,
    n_results: u16,
    mut n_locals: u16,
    code: &mut Vec<CuOpC>,
    consts: &mut Vec<u64>,
    func_types: &[(Vec<ValType>, Vec<ValType>)],
    func_typeidx: &[u32],
    n_imports: u32,
    host_fn_ids: &[u32],
) -> Result<FuncMetaC, String> {
    let code_off = code.len() as u32;
    let scratch = if ops.iter().any(|op| matches!(op, Operator::BrTable { .. })) {
        n_locals = n_locals.saturating_add(1);
        Some(n_params + n_locals - 1)
    } else {
        None
    };
    let mut h: i32 = 0;
    let mut dead = false;
    let mut ctrl: Vec<Ctrl> = vec![Ctrl {
        kind: CtrlKind::Func,
        start: code_off,
        br_if_not: None,
        else_br: None,
        patches: Vec::new(),
        has_else: false,
        base: 0,
        n_params: 0,
        n_results,
        dead_on_entry: false,
    }];

    for op in ops {
        match op {
            Operator::Nop => {}
            Operator::Unreachable => {
                emit(code, OP_UNREACHABLE, 0, 0);
                dead = true;
            }
            Operator::I64Const { value } => {
                let idx = intern(consts, value as u64);
                emit(code, OP_I64_CONST, 0, idx);
                consume(&mut h, dead, 0, 1)?;
            }
            Operator::I32Const { value } => {
                let idx = intern(consts, value as u32 as u64);
                emit(code, OP_I64_CONST, 0, idx);
                consume(&mut h, dead, 0, 1)?;
            }
            Operator::Drop => {
                emit(code, OP_DROP, 0, 0);
                consume(&mut h, dead, 1, 0)?;
            }
            Operator::Select => {
                emit(code, OP_SELECT, 0, 0);
                consume(&mut h, dead, 3, 1)?;
            }
            Operator::TypedSelect { .. } => {
                emit(code, OP_SELECT, 0, 0);
                consume(&mut h, dead, 3, 1)?;
            }
            Operator::I32Eqz => {
                emit(code, OP_I32_EQZ, 0, 0);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I32Eq => {
                emit(code, OP_I32_EQ, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32Ne => {
                emit(code, OP_I32_NE, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32LtS => {
                emit(code, OP_I32_LT_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32LtU => {
                emit(code, OP_I32_LT_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32LeS => {
                emit(code, OP_I32_LE_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32LeU => {
                emit(code, OP_I32_LE_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32GtS => {
                emit(code, OP_I32_GT_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32GtU => {
                emit(code, OP_I32_GT_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32GeS => {
                emit(code, OP_I32_GE_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32GeU => {
                emit(code, OP_I32_GE_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32Add => {
                emit(code, OP_I32_ADD, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32Sub => {
                emit(code, OP_I32_SUB, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32Mul => {
                emit(code, OP_I32_MUL, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32And => {
                emit(code, OP_I32_AND, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32Or => {
                emit(code, OP_I32_OR, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32Xor => {
                emit(code, OP_I32_XOR, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32DivS => {
                emit(code, OP_I32_DIV_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32DivU => {
                emit(code, OP_I32_DIV_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32RemS => {
                emit(code, OP_I32_REM_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32RemU => {
                emit(code, OP_I32_REM_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32Shl => {
                emit(code, OP_I32_SHL, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32ShrS => {
                emit(code, OP_I32_SHR_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32ShrU => {
                emit(code, OP_I32_SHR_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I32WrapI64 => {
                emit(code, OP_I32_WRAP_I64, 0, 0);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64Ne => {
                emit(code, OP_I64_NE, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64LtU => {
                emit(code, OP_I64_LT_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64LeU => {
                emit(code, OP_I64_LE_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64GtS => {
                emit(code, OP_I64_GT_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64GtU => {
                emit(code, OP_I64_GT_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64GeS => {
                emit(code, OP_I64_GE_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64GeU => {
                emit(code, OP_I64_GE_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64Mul => {
                emit(code, OP_I64_MUL, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64And => {
                emit(code, OP_I64_AND, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64Or => {
                emit(code, OP_I64_OR, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64Xor => {
                emit(code, OP_I64_XOR, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64DivS => {
                emit(code, OP_I64_DIV_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64DivU => {
                emit(code, OP_I64_DIV_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64RemS => {
                emit(code, OP_I64_REM_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64RemU => {
                emit(code, OP_I64_REM_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64Shl => {
                emit(code, OP_I64_SHL, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64ShrS => {
                emit(code, OP_I64_SHR_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64ShrU => {
                emit(code, OP_I64_SHR_U, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64Extend32S => {
                emit(code, OP_I64_EXTEND_I32_S, 0, 0);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64ExtendI32S => {
                emit(code, OP_I64_EXTEND_I32_S, 0, 0);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64ExtendI32U => {
                emit(code, OP_I64_EXTEND_I32_U, 0, 0);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64MulWideS => {
                emit(code, OP_I64_MUL_WIDE_S, 0, 0);
                consume(&mut h, dead, 2, 2)?;
            }
            Operator::I64MulWideU => {
                emit(code, OP_I64_MUL_WIDE_U, 0, 0);
                consume(&mut h, dead, 2, 2)?;
            }
            Operator::I64Add128 => {
                emit(code, OP_I64_ADD128, 0, 0);
                consume(&mut h, dead, 4, 2)?;
            }
            Operator::I64Sub128 => {
                emit(code, OP_I64_SUB128, 0, 0);
                consume(&mut h, dead, 4, 2)?;
            }
            Operator::GlobalGet { global_index } => {
                emit(code, OP_GLOBAL_GET, 0, global_index);
                consume(&mut h, dead, 0, 1)?;
            }
            Operator::GlobalSet { global_index } => {
                emit(code, OP_GLOBAL_SET, 0, global_index);
                consume(&mut h, dead, 1, 0)?;
            }
            Operator::LocalGet { local_index } => {
                emit(code, OP_LOCAL_GET, local_index as u16, 0);
                consume(&mut h, dead, 0, 1)?;
            }
            Operator::LocalSet { local_index } => {
                emit(code, OP_LOCAL_SET, local_index as u16, 0);
                consume(&mut h, dead, 1, 0)?;
            }
            Operator::LocalTee { local_index } => {
                emit(code, OP_LOCAL_SET, local_index as u16, 0);
                emit(code, OP_LOCAL_GET, local_index as u16, 0);
            }
            Operator::I64Add => {
                emit(code, OP_I64_ADD, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64Sub => {
                emit(code, OP_I64_SUB, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64Eq => {
                emit(code, OP_I64_EQ, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64Eqz => {
                emit(code, OP_I64_EQZ, 0, 0);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64LeS => {
                emit(code, OP_I64_LE_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::I64LtS => {
                emit(code, OP_I64_LT_S, 0, 0);
                consume(&mut h, dead, 2, 1)?;
            }
            Operator::Block { blockty } => {
                let (np, nr) = block_arity(blockty, func_types)?;
                push_ctrl(
                    &mut ctrl,
                    CtrlKind::Block,
                    code.len() as u32,
                    np,
                    nr,
                    h,
                    dead,
                )?;
            }
            Operator::Loop { blockty } => {
                let (np, nr) = block_arity(blockty, func_types)?;
                push_ctrl(
                    &mut ctrl,
                    CtrlKind::Loop,
                    code.len() as u32,
                    np,
                    nr,
                    h,
                    dead,
                )?;
            }
            Operator::If { blockty } => {
                let (np, nr) = block_arity(blockty, func_types)?;
                consume(&mut h, dead, 1, 0)?;
                let pc = emit(code, OP_BR_IF_NOT, 0, 0);
                push_ctrl(
                    &mut ctrl,
                    CtrlKind::If,
                    code.len() as u32,
                    np,
                    nr,
                    h,
                    dead,
                )?;
                if let Some(c) = ctrl.last_mut() {
                    c.br_if_not = Some(pc);
                }
            }
            Operator::Else => {
                let (base, np, br_if_not) = {
                    let c = ctrl.last_mut().ok_or("else without if")?;
                    if c.kind != CtrlKind::If {
                        return Err("else not in if".into());
                    }
                    (c.base, c.n_params, c.br_if_not)
                };
                let br = emit(code, OP_BR, 0, 0);
                {
                    let c = ctrl.last_mut().unwrap();
                    c.else_br = Some(br);
                    c.has_else = true;
                }
                if let Some(if_pc) = br_if_not {
                    let dest = code.len() as u32;
                    patch_b(code, if_pc, dest);
                }
                h = base + np as i32;
                dead = false;
            }
            Operator::End => {
                let c = ctrl.pop().ok_or("unmatched end")?;
                let join_pc = if c.kind == CtrlKind::Func {
                    emit(code, OP_END_FUNC, n_results, 0)
                } else {
                    code.len() as u32
                };
                if c.kind == CtrlKind::If {
                    if c.has_else {
                        if let Some(br) = c.else_br {
                            patch_b(code, br, join_pc);
                        }
                    } else if let Some(if_pc) = c.br_if_not {
                        patch_b(code, if_pc, join_pc);
                    }
                }
                for p in c.patches {
                    patch_b(code, p, join_pc);
                }
                h = c.base + c.n_results as i32;
                dead = c.dead_on_entry;
            }
            Operator::Br { relative_depth } => {
                let idx = resolve_br(&ctrl, relative_depth)?;
                if !dead {
                    emit_unwind(code, h, &ctrl[idx], n_params, n_locals)?;
                }
                emit_jump(code, &mut ctrl, idx);
                dead = true;
            }
            Operator::BrIf { relative_depth } => {
                consume(&mut h, dead, 1, 0)?;
                let skip_placeholder = emit(code, OP_BR_IF_NOT, 0, 0);
                let idx = resolve_br(&ctrl, relative_depth)?;
                let saved_h = h;
                if !dead {
                    emit_unwind(code, h, &ctrl[idx], n_params, n_locals)?;
                }
                emit_jump(code, &mut ctrl, idx);
                h = saved_h;
                let dest = code.len() as u32;
                patch_b(code, skip_placeholder, dest);
            }
            Operator::BrTable { targets } => {
                let scratch = scratch.ok_or("br_table without scratch local")?;
                let default = targets.default();
                let mut depths: Vec<u32> = Vec::new();
                for t in targets.targets() {
                    depths.push(t.map_err(|e| format!("br_table: {e}"))?);
                }
                emit(code, OP_LOCAL_SET, scratch, 0);
                consume(&mut h, dead, 1, 0)?;
                for (i, depth) in depths.iter().copied().enumerate() {
                    let idxc = intern(consts, i as u32 as u64);
                    emit(code, OP_LOCAL_GET, scratch, 0);
                    consume(&mut h, dead, 0, 1)?;
                    emit(code, OP_I64_CONST, 0, idxc);
                    consume(&mut h, dead, 0, 1)?;
                    emit(code, OP_I32_EQ, 0, 0);
                    consume(&mut h, dead, 2, 1)?;
                    consume(&mut h, dead, 1, 0)?;
                    let skip_placeholder = emit(code, OP_BR_IF_NOT, 0, 0);
                    let idx = resolve_br(&ctrl, depth)?;
                    let saved_h = h;
                    if !dead {
                        emit_unwind(code, h, &ctrl[idx], n_params, n_locals)?;
                    }
                    emit_jump(code, &mut ctrl, idx);
                    h = saved_h;
                    let dest = code.len() as u32;
                    patch_b(code, skip_placeholder, dest);
                }
                let idx = resolve_br(&ctrl, default)?;
                if !dead {
                    emit_unwind(code, h, &ctrl[idx], n_params, n_locals)?;
                }
                emit_jump(code, &mut ctrl, idx);
                dead = true;
            }
            Operator::Return => {
                emit(code, OP_RETURN, n_results, 0);
                dead = true;
            }
            Operator::Call { function_index } => {
                let (np, nr) = func_arity(function_index, func_types, func_typeidx)?;
                if function_index < n_imports {
                    let fn_id = host_fn_ids
                        .get(function_index as usize)
                        .copied()
                        .ok_or_else(|| "host fn_id oob".to_string())?;
                    let packed = np | (nr << 8);
                    emit(code, OP_CALL_HOST, packed, fn_id);
                } else {
                    emit(code, OP_CALL, 0, function_index);
                }
                consume(&mut h, dead, np as i32, nr as i32)?;
            }
            Operator::ReturnCall { function_index } => {
                if function_index < n_imports {
                    return Err("tail call to host import not supported".into());
                }
                emit(code, OP_RETURN_CALL, 0, function_index);
                dead = true;
            }
            Operator::CallIndirect {
                type_index,
                table_index,
            } => {
                if table_index != 0 {
                    return Err("multi-table not supported".into());
                }
                let t = type_index as usize;
                if t >= func_types.len() {
                    return Err("call_indirect type oob".into());
                }
                let np = func_types[t].0.len() as i32;
                let nr = func_types[t].1.len() as i32;
                emit(code, OP_CALL_INDIRECT, 0, type_index);
                consume(&mut h, dead, 1 + np, nr)?;
            }
            Operator::I32Load { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(4, false, false), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64Load { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(8, false, true), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I32Load8S { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(1, true, false), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I32Load8U { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(1, false, false), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I32Load16S { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(2, true, false), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I32Load16U { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(2, false, false), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64Load8S { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(1, true, true), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64Load8U { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(1, false, true), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64Load16S { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(2, true, true), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64Load16U { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(2, false, true), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64Load32S { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(4, true, true), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64Load32U { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(4, false, true), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I32Store { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_STORE, 4, off);
                consume(&mut h, dead, 2, 0)?;
            }
            Operator::I64Store { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_STORE, 8, off);
                consume(&mut h, dead, 2, 0)?;
            }
            Operator::I32Store8 { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_STORE, 1, off);
                consume(&mut h, dead, 2, 0)?;
            }
            Operator::I32Store16 { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_STORE, 2, off);
                consume(&mut h, dead, 2, 0)?;
            }
            Operator::I64Store8 { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_STORE, 1, off);
                consume(&mut h, dead, 2, 0)?;
            }
            Operator::I64Store16 { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_STORE, 2, off);
                consume(&mut h, dead, 2, 0)?;
            }
            Operator::I64Store32 { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_STORE, 4, off);
                consume(&mut h, dead, 2, 0)?;
            }
            Operator::MemorySize { mem } => {
                if mem != 0 {
                    return Err("multi-memory not supported".into());
                }
                emit(code, OP_MEMORY_SIZE, 0, 0);
                consume(&mut h, dead, 0, 1)?;
            }
            Operator::MemoryGrow { mem } => {
                if mem != 0 {
                    return Err("multi-memory not supported".into());
                }
                emit(code, OP_MEMORY_GROW, 0, 0);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::MemoryCopy { dst_mem, src_mem } => {
                if dst_mem != 0 || src_mem != 0 {
                    return Err("multi-memory not supported".into());
                }
                emit(code, OP_MEMORY_COPY, 0, 0);
                consume(&mut h, dead, 3, 0)?;
            }
            Operator::MemoryFill { mem } => {
                if mem != 0 {
                    return Err("multi-memory not supported".into());
                }
                emit(code, OP_MEMORY_FILL, 0, 0);
                consume(&mut h, dead, 3, 0)?;
            }
            Operator::MemoryInit { data_index, mem } => {
                if mem != 0 {
                    return Err("multi-memory not supported".into());
                }
                emit(code, OP_MEMORY_INIT, data_index as u16, 0);
                consume(&mut h, dead, 3, 0)?;
            }
            Operator::DataDrop { data_index } => {
                emit(code, OP_DATA_DROP, data_index as u16, 0);
            }
            Operator::I32Clz => {
                emit(code, OP_CLZ, 32, 0);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::I64Clz => {
                emit(code, OP_CLZ, 64, 0);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::F32Const { value } => {
                let idx = intern(consts, value.bits() as u64);
                emit(code, OP_I64_CONST, 0, idx);
                consume(&mut h, dead, 0, 1)?;
            }
            Operator::F64Const { value } => {
                let idx = intern(consts, value.bits());
                emit(code, OP_I64_CONST, 0, idx);
                consume(&mut h, dead, 0, 1)?;
            }
            Operator::F32Load { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(4, false, false), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::F64Load { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_LOAD, load_flags(8, false, true), off);
                consume(&mut h, dead, 1, 1)?;
            }
            Operator::F32Store { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_STORE, 4, off);
                consume(&mut h, dead, 2, 0)?;
            }
            Operator::F64Store { memarg } => {
                let off = check_memarg(memarg)?;
                emit(code, OP_STORE, 8, off);
                consume(&mut h, dead, 2, 0)?;
            }
            Operator::I32ReinterpretF32
            | Operator::F32ReinterpretI32
            | Operator::I64ReinterpretF64
            | Operator::F64ReinterpretI64 => {
                consume(&mut h, dead, 1, 1)?;
            }
            other => {
                return Err(format!("unsupported wasm opcode: {:?}", other));
            }
        }
    }

    if !ctrl.is_empty() {
        return Err("unclosed control blocks".into());
    }

    let code_len = (code.len() as u32) - code_off;
    Ok(FuncMetaC {
        code_off,
        code_len,
        n_params,
        n_results,
        n_locals,
        max_stack: 0,
    })
}

pub fn translate_wasm(bytes: &[u8]) -> Result<HostModule, String> {
    let parser = Parser::new(0);
    let mut func_types: Vec<(Vec<ValType>, Vec<ValType>)> = Vec::new();
    let mut func_typeidx: Vec<u32> = Vec::new();
    let mut exports: Vec<(String, u32)> = Vec::new();
    let mut code: Vec<CuOpC> = Vec::new();
    let mut consts: Vec<u64> = Vec::new();
    let mut funcs: Vec<FuncMetaC> = Vec::new();
    let mut globals: Vec<u64> = Vec::new();
    let mut mem_min: u64 = 0;
    let mut mem_max_pages: Option<u64> = None;
    let mut has_memory = false;
    let mut table: Vec<u32> = Vec::new();
    let mut pending_data: Vec<(bool, u32, Vec<u8>)> = Vec::new(); // (active, offset, bytes) offset unused if !active
    let mut pending_elem: Vec<(u32, Vec<u32>)> = Vec::new(); // (table_off, func indices)
    let mut host_fn_ids: Vec<u32> = Vec::new();
    let mut host_import_mod: Vec<String> = Vec::new();
    let mut host_import_name: Vec<String> = Vec::new();
    let mut host_import_env: Vec<String> = Vec::new();

    for payload in parser.parse_all(bytes) {
        let payload = payload.map_err(|e| format!("parse: {e}"))?;
        match payload {
            Payload::TypeSection(reader) => {
                for rec in reader {
                    let rec = rec.map_err(|e| format!("type: {e}"))?;
                    for ty in rec.into_types() {
                        match &ty.composite_type.inner {
                            CompositeInnerType::Func(ft) => {
                                let params = ft.params().to_vec();
                                let results = ft.results().to_vec();
                                only_int(&params)?;
                                only_int(&results)?;
                                func_types.push((params, results));
                            }
                            _ => return Err("non-func type in type section".into()),
                        }
                    }
                }
            }
            Payload::FunctionSection(reader) => {
                for idx in reader {
                    func_typeidx.push(idx.map_err(|e| format!("funcsec: {e}"))?);
                }
            }
            Payload::ImportSection(reader) => {
                for imp in reader {
                    let imp = imp.map_err(|e| format!("import: {e}"))?;
                    match imp.ty {
                        TypeRef::Func(type_idx) => {
                            let t = type_idx as usize;
                            if t >= func_types.len() {
                                return Err(format!("import type oob: {}::{}", imp.module, imp.name));
                            }
                            only_int(&func_types[t].0)?;
                            only_int(&func_types[t].1)?;
                            let mod_s = imp.module.to_string();
                            let fn_s = imp.name.to_string();
                            let fn_id = env_fn_id::lookup(&mod_s, &fn_s)?;
                            let env_label = env_fn_id::label(fn_id)
                                .unwrap_or("?")
                                .to_string();
                            func_typeidx.push(type_idx);
                            host_fn_ids.push(fn_id);
                            host_import_mod.push(mod_s);
                            host_import_name.push(fn_s);
                            host_import_env.push(env_label);
                            let (ref params, ref results) = func_types[t];
                            funcs.push(FuncMetaC {
                                code_off: 0,
                                code_len: 0,
                                n_params: params.len() as u16,
                                n_results: results.len() as u16,
                                n_locals: 0,
                                max_stack: 0,
                            });
                        }
                        other => {
                            return Err(format!(
                                "unsupported import {:?}: {}::{}",
                                other, imp.module, imp.name
                            ));
                        }
                    }
                }
            }
            Payload::MemorySection(reader) => {
                for mt in reader {
                    let mt = mt.map_err(|e| format!("memory: {e}"))?;
                    if has_memory {
                        return Err("multiple memories not supported".into());
                    }
                    if mt.memory64 {
                        return Err("memory64 not supported".into());
                    }
                    if mt.shared {
                        return Err("shared memory not supported".into());
                    }
                    if mt.page_size_log2.is_some() {
                        return Err("custom page size not supported".into());
                    }
                    mem_min = mt.initial;
                    mem_max_pages = mt.maximum;
                    has_memory = true;
                }
            }
            Payload::TableSection(reader) => {
                for t in reader {
                    let t = t.map_err(|e| format!("table: {e}"))?;
                    if !table.is_empty() {
                        return Err("multiple tables not supported".into());
                    }
                    if t.ty.table64 {
                        return Err("table64 not supported".into());
                    }
                    if !t.ty.element_type.is_func_ref() {
                        return Err(format!("unsupported table elem type {:?}", t.ty.element_type));
                    }
                    if t.ty.initial > 1024 {
                        return Err("table too large".into());
                    }
                    table = vec![TABLE_NULL; t.ty.initial as usize];
                }
            }
            Payload::ElementSection(reader) => {
                for el in reader {
                    let el = el.map_err(|e| format!("elem: {e}"))?;
                    let off = match el.kind {
                        ElementKind::Active {
                            table_index,
                            offset_expr,
                        } => {
                            if table_index.unwrap_or(0) != 0 {
                                return Err("elem to non-zero table".into());
                            }
                            eval_i32_expr(offset_expr.get_operators_reader())?
                        }
                        ElementKind::Passive | ElementKind::Declared => {
                            return Err("passive/declared elem not supported".into());
                        }
                    };
                    let mut idxs = Vec::new();
                    match el.items {
                        ElementItems::Functions(r) => {
                            for i in r {
                                idxs.push(i.map_err(|e| format!("elem func: {e}"))?);
                            }
                        }
                        ElementItems::Expressions(_, r) => {
                            for expr in r {
                                let expr = expr.map_err(|e| format!("elem expr: {e}"))?;
                                let mut fidx: Option<u32> = None;
                                for op in expr.get_operators_reader() {
                                    let op = op.map_err(|e| format!("elem op: {e}"))?;
                                    match op {
                                        Operator::RefFunc { function_index } => {
                                            fidx = Some(function_index)
                                        }
                                        Operator::RefNull { .. } => fidx = Some(TABLE_NULL),
                                        Operator::End => {}
                                        other => {
                                            return Err(format!("unsupported elem expr {other:?}"))
                                        }
                                    }
                                }
                                idxs.push(fidx.ok_or("empty elem expr")?);
                            }
                        }
                    }
                    pending_elem.push((off, idxs));
                }
            }
            Payload::DataSection(reader) => {
                for d in reader {
                    let d = d.map_err(|e| format!("data: {e}"))?;
                    match d.kind {
                        DataKind::Active {
                            memory_index,
                            offset_expr,
                        } => {
                            if memory_index != 0 {
                                return Err("data to non-zero memory".into());
                            }
                            let off = eval_i32_expr(offset_expr.get_operators_reader())?;
                            pending_data.push((true, off, d.data.to_vec()));
                        }
                        DataKind::Passive => {
                            pending_data.push((false, 0, d.data.to_vec()));
                        }
                    }
                }
            }
            Payload::DataCountSection { .. } => {}
            Payload::GlobalSection(reader) => {
                for g in reader {
                    let g = g.map_err(|e| format!("global: {e}"))?;
                    if g.ty.content_type != ValType::I32
                        && g.ty.content_type != ValType::I64
                        && g.ty.content_type != ValType::F32
                        && g.ty.content_type != ValType::F64
                    {
                        return Err(format!("unsupported global type {:?}", g.ty.content_type));
                    }
                    let mut val: Option<u64> = None;
                    for op in g.init_expr.get_operators_reader() {
                        let op = op.map_err(|e| format!("global init: {e}"))?;
                        match op {
                            Operator::I32Const { value } => val = Some(value as u32 as u64),
                            Operator::I64Const { value } => val = Some(value as u64),
                            Operator::F32Const { value } => val = Some(value.bits() as u64),
                            Operator::F64Const { value } => val = Some(value.bits()),
                            Operator::End => {}
                            other => return Err(format!("unsupported global init {other:?}")),
                        }
                    }
                    globals.push(val.ok_or_else(|| "empty global init".to_string())?);
                }
            }
            Payload::ExportSection(reader) => {
                for ex in reader {
                    let ex = ex.map_err(|e| format!("export: {e}"))?;
                    if ex.kind == ExternalKind::Func {
                        exports.push((ex.name.to_string(), ex.index));
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                let fi = funcs.len();
                if fi >= func_typeidx.len() {
                    return Err("code/function section mismatch".into());
                }
                let tidx = func_typeidx[fi] as usize;
                if tidx >= func_types.len() {
                    return Err("type index oob".into());
                }
                let (ref params, ref results) = func_types[tidx];
                let n_params = params.len() as u16;
                let n_results = results.len() as u16;
                let mut n_locals: u32 = 0;
                let locals_reader = body
                    .get_locals_reader()
                    .map_err(|e| format!("locals: {e}"))?;
                for loc in locals_reader {
                    let (cnt, ty) = loc.map_err(|e| format!("local: {e}"))?;
                    if ty != ValType::I64
                        && ty != ValType::I32
                        && ty != ValType::F32
                        && ty != ValType::F64
                    {
                        return Err(format!("non-int local {:?}", ty));
                    }
                    n_locals = n_locals
                        .checked_add(cnt)
                        .ok_or_else(|| "too many locals".to_string())?;
                }
                if n_locals > u16::MAX as u32 {
                    return Err("too many locals".into());
                }
                let ops_reader = body
                    .get_operators_reader()
                    .map_err(|e| format!("ops: {e}"))?;
                let mut ops = Vec::new();
                for op in ops_reader {
                    ops.push(op.map_err(|e| format!("op: {e}"))?);
                }
                let meta = match lower_operators(
                    ops,
                    n_params,
                    n_results,
                    n_locals as u16,
                    &mut code,
                    &mut consts,
                    &func_types,
                    &func_typeidx,
                    host_fn_ids.len() as u32,
                    &host_fn_ids,
                ) {
                    Ok(m) => m,
                    Err(_) => {
                        let code_off = code.len() as u32;
                        emit(&mut code, OP_UNREACHABLE, 0, 0);
                        emit(&mut code, OP_END_FUNC, n_results, 0);
                        FuncMetaC {
                            code_off,
                            code_len: 2,
                            n_params,
                            n_results,
                            n_locals: 0,
                            max_stack: 0,
                        }
                    }
                };
                funcs.push(meta);
            }
            Payload::StartSection { .. } => return Err("start section not supported".into()),
            _ => {}
        }
    }

    let n_host_imports = host_fn_ids.len() as u32;

    let mut type_fp = Vec::new();
    for (p, r) in &func_types {
        type_fp.push(type_fingerprint(p, r)?);
    }

    let cap_pages = mem_max_pages
        .unwrap_or(CUWASM_MEM_MAX_PAGES)
        .min(CUWASM_MEM_MAX_PAGES);
    if has_memory && mem_min > cap_pages {
        return Err("memory too large for host".into());
    }
    let mem_max = if has_memory {
        (cap_pages as u32).saturating_mul(WASM_PAGE)
    } else {
        0
    };
    let mem_size = if has_memory {
        (mem_min as u32).saturating_mul(WASM_PAGE)
    } else {
        0
    };
    let mut memory = vec![0u8; mem_max as usize];

    let mut data_blob = Vec::new();
    let mut data_off = Vec::new();
    let mut data_len = Vec::new();
    let mut data_live = Vec::new();
    for (active, off, bytes) in pending_data {
        let start = data_blob.len() as u32;
        data_blob.extend_from_slice(&bytes);
        data_off.push(start);
        data_len.push(bytes.len() as u32);
        if active {
            let end = off as u64 + bytes.len() as u64;
            if end > mem_size as u64 {
                return Err("data segment out of bounds".into());
            }
            if !bytes.is_empty() {
                let o = off as usize;
                memory[o..o + bytes.len()].copy_from_slice(&bytes);
            }
            data_live.push(0);
        } else {
            data_live.push(1);
        }
    }

    for (off, idxs) in pending_elem {
        for (i, fi) in idxs.into_iter().enumerate() {
            let slot = off as usize + i;
            if slot >= table.len() {
                return Err("elem segment out of bounds".into());
            }
            table[slot] = fi;
        }
    }

    if funcs.is_empty() {
        return Err("no functions".into());
    }

    Ok(HostModule {
        code,
        consts,
        funcs,
        exports,
        globals,
        memory,
        mem_size,
        mem_max,
        data_blob,
        data_off,
        data_len,
        data_live,
        table,
        func_typeidx,
        type_fp,
        n_host_imports,
        host_fn_id: host_fn_ids,
        host_import_mod,
        host_import_name,
        host_import_env,
    })
}

fn vec_to_raw<T>(mut v: Vec<T>) -> (*mut T, u32) {
    v.shrink_to_fit();
    let n = v.len() as u32;
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    (p, n)
}

unsafe fn free_vec<T>(p: *mut T, n: u32) {
    if !p.is_null() {
        let _ = Vec::from_raw_parts(p, n as usize, n as usize);
    }
}

fn cstring_err(msg: &str) -> *mut c_char {
    CString::new(msg.replace('\0', "")).unwrap().into_raw()
}

/// Returns 0 on success, -1 on error (err string set).
#[no_mangle]
pub unsafe extern "C" fn cuwasm_translate_wasm(
    data: *const u8,
    len: usize,
    out: *mut TranslateOut,
) -> c_int {
    if out.is_null() {
        return -1;
    }
    *out = TranslateOut::default();
    if data.is_null() && len != 0 {
        (*out).err = cstring_err("null data");
        return -1;
    }
    let bytes = if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(data, len)
    };
    match translate_wasm(bytes) {
        Ok(m) => {
            let (code, n_code) = vec_to_raw(m.code);
            let (consts, n_consts) = vec_to_raw(m.consts);
            let (funcs, n_funcs) = vec_to_raw(m.funcs);
            let mut names: Vec<*mut c_char> = Vec::new();
            let mut idxs: Vec<u32> = Vec::new();
            for (name, idx) in m.exports {
                names.push(CString::new(name).unwrap().into_raw());
                idxs.push(idx);
            }
            let (export_names, n_exports1) = vec_to_raw(names);
            let (export_idxs, n_exports2) = vec_to_raw(idxs);
            debug_assert_eq!(n_exports1, n_exports2);
            let (globals, n_globals) = vec_to_raw(m.globals);
            let mem_size = m.mem_size;
            let mem_max = m.mem_max;
            let (memory, mem_max_chk) = vec_to_raw(m.memory);
            debug_assert_eq!(mem_max, mem_max_chk);
            let data_blob_len_expect = m.data_blob.len() as u32;
            let (data_blob, data_blob_len) = vec_to_raw(m.data_blob);
            debug_assert_eq!(data_blob_len_expect, data_blob_len);
            let n_data = m.data_live.len() as u32;
            let (data_off, n1) = vec_to_raw(m.data_off);
            let (data_len, n2) = vec_to_raw(m.data_len);
            let (data_live, n3) = vec_to_raw(m.data_live);
            debug_assert_eq!(n_data, n1);
            debug_assert_eq!(n_data, n2);
            debug_assert_eq!(n_data, n3);
            let (table, table_len) = vec_to_raw(m.table);
            let (func_typeidx, n_tidx) = vec_to_raw(m.func_typeidx);
            debug_assert_eq!(n_tidx, n_funcs);
            let (type_fp, n_types) = vec_to_raw(m.type_fp);
            let n_host_imports = m.n_host_imports;
            let (host_fn_id, n_host1) = vec_to_raw(m.host_fn_id);
            debug_assert_eq!(n_host_imports, n_host1);
            let mut himods: Vec<*mut c_char> = Vec::new();
            let mut hinames: Vec<*mut c_char> = Vec::new();
            let mut hienvs: Vec<*mut c_char> = Vec::new();
            for i in 0..n_host_imports as usize {
                himods.push(CString::new(m.host_import_mod[i].clone()).unwrap().into_raw());
                hinames.push(CString::new(m.host_import_name[i].clone()).unwrap().into_raw());
                hienvs.push(CString::new(m.host_import_env[i].clone()).unwrap().into_raw());
            }
            let (host_import_mod, n_host2) = vec_to_raw(himods);
            let (host_import_name, n_host3) = vec_to_raw(hinames);
            let (host_import_env, n_host4) = vec_to_raw(hienvs);
            debug_assert_eq!(n_host_imports, n_host2);
            debug_assert_eq!(n_host_imports, n_host3);
            debug_assert_eq!(n_host_imports, n_host4);
            (*out).code = code;
            (*out).n_code = n_code;
            (*out).consts = consts;
            (*out).n_consts = n_consts;
            (*out).funcs = funcs;
            (*out).n_funcs = n_funcs;
            (*out).export_names = export_names;
            (*out).export_idxs = export_idxs;
            (*out).n_exports = n_exports1;
            (*out).globals = globals;
            (*out).n_globals = n_globals;
            (*out).memory = memory;
            (*out).mem_size = mem_size;
            (*out).mem_max = mem_max;
            (*out).data_blob = data_blob;
            (*out).data_blob_len = data_blob_len;
            (*out).data_off = data_off;
            (*out).data_len = data_len;
            (*out).data_live = data_live;
            (*out).n_data = n_data;
            (*out).table = table;
            (*out).table_len = table_len;
            (*out).func_typeidx = func_typeidx;
            (*out).type_fp = type_fp;
            (*out).n_types = n_types;
            (*out).n_host_imports = n_host_imports;
            (*out).host_fn_id = host_fn_id;
            (*out).host_import_mod = host_import_mod;
            (*out).host_import_name = host_import_name;
            (*out).host_import_env = host_import_env;
            0
        }
        Err(e) => {
            (*out).err = cstring_err(&e);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn cuwasm_translate_free(out: *mut TranslateOut) {
    if out.is_null() {
        return;
    }
    let o = &mut *out;
    free_vec(o.code, o.n_code);
    free_vec(o.consts, o.n_consts);
    free_vec(o.funcs, o.n_funcs);
    if !o.export_names.is_null() {
        let names = Vec::from_raw_parts(
            o.export_names,
            o.n_exports as usize,
            o.n_exports as usize,
        );
        for p in names {
            if !p.is_null() {
                let _ = CString::from_raw(p);
            }
        }
    }
    free_vec(o.export_idxs, o.n_exports);
    free_vec(o.globals, o.n_globals);
    free_vec(o.memory, o.mem_max);
    free_vec(o.data_blob, o.data_blob_len);
    free_vec(o.data_off, o.n_data);
    free_vec(o.data_len, o.n_data);
    free_vec(o.data_live, o.n_data);
    free_vec(o.table, o.table_len);
    free_vec(o.func_typeidx, o.n_funcs);
    free_vec(o.type_fp, o.n_types);
    free_vec(o.host_fn_id, o.n_host_imports);
    for names in [o.host_import_mod, o.host_import_name, o.host_import_env] {
        if !names.is_null() {
            let v = Vec::from_raw_parts(
                names,
                o.n_host_imports as usize,
                o.n_host_imports as usize,
            );
            for p in v {
                if !p.is_null() {
                    let _ = CString::from_raw(p);
                }
            }
        }
    }
    if !o.err.is_null() {
        let _ = CString::from_raw(o.err);
    }
    *o = TranslateOut::default();
}

#[allow(dead_code)]
fn _keep_cstr(_: &CStr) {}
