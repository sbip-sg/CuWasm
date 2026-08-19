//! WASM → CuOp lowering. Decode with wasmparser; emit fixed-width ops.

use libc::c_char;
use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::ptr;
use wasmparser::{CompositeInnerType, ExternalKind, Operator, Parser, Payload, ValType};

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
        if *t != ValType::I64 && *t != ValType::I32 {
            return Err(format!("unsupported valtype {:?}", t));
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

fn lower_operators(
    ops: Vec<Operator<'_>>,
    n_params: u16,
    n_results: u16,
    n_locals: u16,
    code: &mut Vec<CuOpC>,
    consts: &mut Vec<u64>,
) -> Result<FuncMetaC, String> {
    let code_off = code.len() as u32;
    let mut ctrl: Vec<Ctrl> = vec![Ctrl {
        kind: CtrlKind::Func,
        start: code_off,
        br_if_not: None,
        else_br: None,
        patches: Vec::new(),
        has_else: false,
    }];

    for op in ops {
        match op {
            Operator::Nop => {}
            Operator::Unreachable => {
                emit(code, OP_UNREACHABLE, 0, 0);
            }
            Operator::I64Const { value } => {
                let idx = intern(consts, value as u64);
                emit(code, OP_I64_CONST, 0, idx);
            }
            Operator::I32Const { value } => {
                let idx = intern(consts, value as u32 as u64);
                emit(code, OP_I64_CONST, 0, idx);
            }
            Operator::Drop => {
                emit(code, OP_DROP, 0, 0);
            }
            Operator::Select => {
                emit(code, OP_SELECT, 0, 0);
            }
            Operator::TypedSelect { .. } => {
                emit(code, OP_SELECT, 0, 0);
            }
            Operator::I32Eqz => {
                emit(code, OP_I32_EQZ, 0, 0);
            }
            Operator::I32Eq => {
                emit(code, OP_I32_EQ, 0, 0);
            }
            Operator::I32Ne => {
                emit(code, OP_I32_NE, 0, 0);
            }
            Operator::I32LtS => {
                emit(code, OP_I32_LT_S, 0, 0);
            }
            Operator::I32LtU => {
                emit(code, OP_I32_LT_U, 0, 0);
            }
            Operator::I32LeS => {
                emit(code, OP_I32_LE_S, 0, 0);
            }
            Operator::I32LeU => {
                emit(code, OP_I32_LE_U, 0, 0);
            }
            Operator::I32GtS => {
                emit(code, OP_I32_GT_S, 0, 0);
            }
            Operator::I32GtU => {
                emit(code, OP_I32_GT_U, 0, 0);
            }
            Operator::I32GeS => {
                emit(code, OP_I32_GE_S, 0, 0);
            }
            Operator::I32GeU => {
                emit(code, OP_I32_GE_U, 0, 0);
            }
            Operator::I32Add => {
                emit(code, OP_I32_ADD, 0, 0);
            }
            Operator::I32Sub => {
                emit(code, OP_I32_SUB, 0, 0);
            }
            Operator::I32Mul => {
                emit(code, OP_I32_MUL, 0, 0);
            }
            Operator::I32And => {
                emit(code, OP_I32_AND, 0, 0);
            }
            Operator::I32Or => {
                emit(code, OP_I32_OR, 0, 0);
            }
            Operator::I32Xor => {
                emit(code, OP_I32_XOR, 0, 0);
            }
            Operator::I32DivS => {
                emit(code, OP_I32_DIV_S, 0, 0);
            }
            Operator::I32DivU => {
                emit(code, OP_I32_DIV_U, 0, 0);
            }
            Operator::I32RemS => {
                emit(code, OP_I32_REM_S, 0, 0);
            }
            Operator::I32RemU => {
                emit(code, OP_I32_REM_U, 0, 0);
            }
            Operator::I32Shl => {
                emit(code, OP_I32_SHL, 0, 0);
            }
            Operator::I32ShrS => {
                emit(code, OP_I32_SHR_S, 0, 0);
            }
            Operator::I32ShrU => {
                emit(code, OP_I32_SHR_U, 0, 0);
            }
            Operator::I32WrapI64 => {
                emit(code, OP_I32_WRAP_I64, 0, 0);
            }
            Operator::I64Ne => {
                emit(code, OP_I64_NE, 0, 0);
            }
            Operator::I64LtU => {
                emit(code, OP_I64_LT_U, 0, 0);
            }
            Operator::I64LeU => {
                emit(code, OP_I64_LE_U, 0, 0);
            }
            Operator::I64GtS => {
                emit(code, OP_I64_GT_S, 0, 0);
            }
            Operator::I64GtU => {
                emit(code, OP_I64_GT_U, 0, 0);
            }
            Operator::I64GeS => {
                emit(code, OP_I64_GE_S, 0, 0);
            }
            Operator::I64GeU => {
                emit(code, OP_I64_GE_U, 0, 0);
            }
            Operator::I64Mul => {
                emit(code, OP_I64_MUL, 0, 0);
            }
            Operator::I64And => {
                emit(code, OP_I64_AND, 0, 0);
            }
            Operator::I64Or => {
                emit(code, OP_I64_OR, 0, 0);
            }
            Operator::I64Xor => {
                emit(code, OP_I64_XOR, 0, 0);
            }
            Operator::I64DivS => {
                emit(code, OP_I64_DIV_S, 0, 0);
            }
            Operator::I64DivU => {
                emit(code, OP_I64_DIV_U, 0, 0);
            }
            Operator::I64RemS => {
                emit(code, OP_I64_REM_S, 0, 0);
            }
            Operator::I64RemU => {
                emit(code, OP_I64_REM_U, 0, 0);
            }
            Operator::I64Shl => {
                emit(code, OP_I64_SHL, 0, 0);
            }
            Operator::I64ShrS => {
                emit(code, OP_I64_SHR_S, 0, 0);
            }
            Operator::I64ShrU => {
                emit(code, OP_I64_SHR_U, 0, 0);
            }
            Operator::I64Extend32S => {
                emit(code, OP_I64_EXTEND_I32_S, 0, 0);
            }
            Operator::I64ExtendI32S => {
                emit(code, OP_I64_EXTEND_I32_S, 0, 0);
            }
            Operator::I64ExtendI32U => {
                emit(code, OP_I64_EXTEND_I32_U, 0, 0);
            }
            Operator::LocalGet { local_index } => {
                emit(code, OP_LOCAL_GET, local_index as u16, 0);
            }
            Operator::LocalSet { local_index } => {
                emit(code, OP_LOCAL_SET, local_index as u16, 0);
            }
            Operator::LocalTee { local_index } => {
                emit(code, OP_LOCAL_SET, local_index as u16, 0);
                emit(code, OP_LOCAL_GET, local_index as u16, 0);
            }
            Operator::I64Add => {
                emit(code, OP_I64_ADD, 0, 0);
            }
            Operator::I64Sub => {
                emit(code, OP_I64_SUB, 0, 0);
            }
            Operator::I64Eq => {
                emit(code, OP_I64_EQ, 0, 0);
            }
            Operator::I64Eqz => {
                emit(code, OP_I64_EQZ, 0, 0);
            }
            Operator::I64LeS => {
                emit(code, OP_I64_LE_S, 0, 0);
            }
            Operator::I64LtS => {
                emit(code, OP_I64_LT_S, 0, 0);
            }
            Operator::Block { blockty: _ } => {
                ctrl.push(Ctrl {
                    kind: CtrlKind::Block,
                    start: code.len() as u32,
                    br_if_not: None,
                    else_br: None,
                    patches: Vec::new(),
                    has_else: false,
                });
            }
            Operator::Loop { blockty: _ } => {
                ctrl.push(Ctrl {
                    kind: CtrlKind::Loop,
                    start: code.len() as u32,
                    br_if_not: None,
                    else_br: None,
                    patches: Vec::new(),
                    has_else: false,
                });
            }
            Operator::If { blockty: _ } => {
                let pc = emit(code, OP_BR_IF_NOT, 0, 0);
                ctrl.push(Ctrl {
                    kind: CtrlKind::If,
                    start: code.len() as u32,
                    br_if_not: Some(pc),
                    else_br: None,
                    patches: Vec::new(),
                    has_else: false,
                });
            }
            Operator::Else => {
                let c = ctrl.last_mut().ok_or("else without if")?;
                if c.kind != CtrlKind::If {
                    return Err("else not in if".into());
                }
                let br = emit(code, OP_BR, 0, 0);
                c.else_br = Some(br);
                c.has_else = true;
                if let Some(if_pc) = c.br_if_not {
                    let dest = code.len() as u32;
                    patch_b(code, if_pc, dest);
                }
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
            }
            Operator::Br { relative_depth } => {
                let idx = resolve_br(&ctrl, relative_depth)?;
                let kind = ctrl[idx].kind;
                if kind == CtrlKind::Loop {
                    let start = ctrl[idx].start;
                    emit(code, OP_BR, 0, start);
                } else {
                    let pc = emit(code, OP_BR, 0, 0);
                    ctrl[idx].patches.push(pc);
                }
            }
            Operator::BrIf { relative_depth } => {
                // jump if != 0. Encode as: br_if_not skip; br target; skip:
                let skip_placeholder = emit(code, OP_BR_IF_NOT, 0, 0);
                let idx = resolve_br(&ctrl, relative_depth)?;
                let kind = ctrl[idx].kind;
                if kind == CtrlKind::Loop {
                    let start = ctrl[idx].start;
                    emit(code, OP_BR, 0, start);
                } else {
                    let pc = emit(code, OP_BR, 0, 0);
                    ctrl[idx].patches.push(pc);
                }
                let dest = code.len() as u32;
                patch_b(code, skip_placeholder, dest);
            }
            Operator::Return => {
                emit(code, OP_RETURN, n_results, 0);
            }
            Operator::Call { function_index } => {
                emit(code, OP_CALL, 0, function_index);
            }
            Operator::ReturnCall { function_index } => {
                emit(code, OP_RETURN_CALL, 0, function_index);
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
    let mut saw_memory = false;
    let mut saw_global = false;
    let mut saw_import = false;

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
                    let _ = imp.map_err(|e| format!("import: {e}"))?;
                    saw_import = true;
                }
            }
            Payload::MemorySection(_) => saw_memory = true,
            Payload::GlobalSection(_) => saw_global = true,
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
                    if ty != ValType::I64 && ty != ValType::I32 {
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
                let meta = lower_operators(
                    ops,
                    n_params,
                    n_results,
                    n_locals as u16,
                    &mut code,
                    &mut consts,
                )?;
                funcs.push(meta);
            }
            Payload::StartSection { .. } => return Err("start section not supported".into()),
            Payload::TableSection(_)
            | Payload::ElementSection(_)
            | Payload::DataSection(_)
            | Payload::DataCountSection { .. } => {
                return Err("table/element/data not supported in stage 1".into());
            }
            _ => {}
        }
    }

    if saw_import {
        return Err("imports not supported in stage 1".into());
    }
    if saw_memory {
        return Err("memory not supported in stage 1".into());
    }
    if saw_global {
        return Err("globals not supported in stage 1".into());
    }
    if funcs.is_empty() {
        return Err("no functions".into());
    }

    let _ = (saw_memory, saw_global);
    Ok(HostModule {
        code,
        consts,
        funcs,
        exports,
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
            (*out).code = code;
            (*out).n_code = n_code;
            (*out).consts = consts;
            (*out).n_consts = n_consts;
            (*out).funcs = funcs;
            (*out).n_funcs = n_funcs;
            (*out).export_names = export_names;
            (*out).export_idxs = export_idxs;
            (*out).n_exports = n_exports1;
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
    if !o.err.is_null() {
        let _ = CString::from_raw(o.err);
    }
    *o = TranslateOut::default();
}

#[allow(dead_code)]
fn _keep_cstr(_: &CStr) {}
