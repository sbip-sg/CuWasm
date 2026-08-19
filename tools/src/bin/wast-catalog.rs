use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use wast::core::{WastArgCore, WastRetCore};
use wast::parser::ParseBuffer;
use wast::{Wast, WastArg, WastDirective, WastExecute, WastRet};

#[derive(Serialize)]
struct Case {
    kind: String,
    file: String,
    export: String,
    wasm: String,
    skip: String,
    args: Vec<String>,
    arg_ty: Vec<String>,
    expected: Vec<String>,
    exp_ty: Vec<String>,
}

fn walk_wast(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn rec(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                rec(&p, out);
            } else if p.extension().and_then(|s| s.to_str()) == Some("wast") {
                out.push(p);
            }
        }
    }
    rec(root, &mut out);
    out.sort();
    out
}

fn arg_core(a: &WastArg<'_>) -> Result<(u64, &'static str), String> {
    match a {
        WastArg::Core(WastArgCore::I32(v)) => Ok((*v as u32 as u64, "i32")),
        WastArg::Core(WastArgCore::I64(v)) => Ok((*v as u64, "i64")),
        _ => Err("non-int arg".into()),
    }
}

fn ret_core(r: &WastRet<'_>) -> Result<(u64, &'static str), String> {
    match r {
        WastRet::Core(WastRetCore::I32(v)) => Ok((*v as u32 as u64, "i32")),
        WastRet::Core(WastRetCore::I64(v)) => Ok((*v as u64, "i64")),
        _ => Err("non-int result".into()),
    }
}

fn wasm_has_import(bytes: &[u8]) -> bool {
    if bytes.len() < 8 {
        return false;
    }
    let mut i = 8usize;
    while i + 1 < bytes.len() {
        let id = bytes[i];
        i += 1;
        let mut n = 0u32;
        let mut shift = 0;
        loop {
            if i >= bytes.len() {
                return false;
            }
            let b = bytes[i];
            i += 1;
            n |= ((b & 0x7f) as u32) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 28 {
                return false;
            }
        }
        if id == 2 && n > 0 {
            return true;
        }
        if i + n as usize > bytes.len() {
            return false;
        }
        i += n as usize;
    }
    false
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: wast-catalog <wast-root> <out-dir>");
        std::process::exit(2);
    }
    let root = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let wasm_dir = out.join("wasm");
    fs::create_dir_all(&wasm_dir).unwrap();

    let files = walk_wast(&root);
    let mut cases: Vec<Case> = Vec::new();
    let mut wasm_n = 0u32;
    let mut parse_fail = 0u32;

    for path in &files {
        let rel = path.strip_prefix(&root).unwrap_or(path).to_string_lossy().replace('\\', "/");
        let src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                cases.push(Case {
                    kind: "skip".into(),
                    file: rel,
                    export: String::new(),
                    wasm: String::new(),
                    skip: format!("read: {e}"),
                    args: vec![],
                    arg_ty: vec![],
                    expected: vec![],
                    exp_ty: vec![],
                });
                continue;
            }
        };
        let buf = match ParseBuffer::new(&src) {
            Ok(b) => b,
            Err(e) => {
                parse_fail += 1;
                cases.push(Case {
                    kind: "skip".into(),
                    file: rel,
                    export: String::new(),
                    wasm: String::new(),
                    skip: format!("parse: {e}"),
                    args: vec![],
                    arg_ty: vec![],
                    expected: vec![],
                    exp_ty: vec![],
                });
                continue;
            }
        };
        let wast: Wast = match wast::parser::parse(&buf) {
            Ok(w) => w,
            Err(e) => {
                parse_fail += 1;
                cases.push(Case {
                    kind: "skip".into(),
                    file: rel.clone(),
                    export: String::new(),
                    wasm: String::new(),
                    skip: format!("wast: {e}"),
                    args: vec![],
                    arg_ty: vec![],
                    expected: vec![],
                    exp_ty: vec![],
                });
                continue;
            }
        };

        let mut current_wasm: Option<String> = None;
        let mut current_skip: Option<String> = None;

        for dir in wast.directives {
            match dir {
                WastDirective::Module(mut m) | WastDirective::ModuleDefinition(mut m) => {
                    match m.encode() {
                        Ok(bytes) => {
                            if wasm_has_import(&bytes) {
                                current_wasm = None;
                                current_skip = Some("import".into());
                            } else {
                                wasm_n += 1;
                                let name = format!("{wasm_n:05}.wasm");
                                fs::write(wasm_dir.join(&name), &bytes).unwrap();
                                current_wasm = Some(format!("wasm/{name}"));
                                current_skip = None;
                            }
                        }
                        Err(e) => {
                            current_wasm = None;
                            current_skip = Some(format!("encode: {e}"));
                        }
                    }
                }
                WastDirective::Register { .. } => {}
                WastDirective::AssertReturn { exec, results, .. } => {
                    let WastExecute::Invoke(inv) = exec else {
                        cases.push(Case {
                            kind: "skip".into(),
                            file: rel.clone(),
                            export: String::new(),
                            wasm: String::new(),
                            skip: "non-invoke assert_return".into(),
                            args: vec![],
                            arg_ty: vec![],
                            expected: vec![],
                            exp_ty: vec![],
                        });
                        continue;
                    };
                    if let Some(reason) = &current_skip {
                        cases.push(Case {
                            kind: "skip".into(),
                            file: rel.clone(),
                            export: inv.name.to_string(),
                            wasm: String::new(),
                            skip: reason.clone(),
                            args: vec![],
                            arg_ty: vec![],
                            expected: vec![],
                            exp_ty: vec![],
                        });
                        continue;
                    }
                    let Some(wasm) = &current_wasm else {
                        cases.push(Case {
                            kind: "skip".into(),
                            file: rel.clone(),
                            export: inv.name.to_string(),
                            wasm: String::new(),
                            skip: "no module".into(),
                            args: vec![],
                            arg_ty: vec![],
                            expected: vec![],
                            exp_ty: vec![],
                        });
                        continue;
                    };
                    let mut args_v = Vec::new();
                    let mut arg_ty = Vec::new();
                    let mut bad: Option<String> = None;
                    for a in &inv.args {
                        match arg_core(a) {
                            Ok((bits, ty)) => {
                                args_v.push(bits.to_string());
                                arg_ty.push(ty.to_string());
                            }
                            Err(e) => bad = Some(e),
                        }
                    }
                    let mut exp_v = Vec::new();
                    let mut exp_ty = Vec::new();
                    for r in &results {
                        match ret_core(r) {
                            Ok((bits, ty)) => {
                                exp_v.push(bits.to_string());
                                exp_ty.push(ty.to_string());
                            }
                            Err(e) => bad = Some(e),
                        }
                    }
                    if let Some(e) = bad {
                        cases.push(Case {
                            kind: "skip".into(),
                            file: rel.clone(),
                            export: inv.name.to_string(),
                            wasm: wasm.clone(),
                            skip: e,
                            args: vec![],
                            arg_ty: vec![],
                            expected: vec![],
                            exp_ty: vec![],
                        });
                        continue;
                    }
                    cases.push(Case {
                        kind: "return".into(),
                        file: rel.clone(),
                        export: inv.name.to_string(),
                        wasm: wasm.clone(),
                        skip: String::new(),
                        args: args_v,
                        arg_ty,
                        expected: exp_v,
                        exp_ty,
                    });
                }
                WastDirective::AssertTrap { exec, .. } => {
                    let WastExecute::Invoke(inv) = exec else {
                        cases.push(Case {
                            kind: "skip".into(),
                            file: rel.clone(),
                            export: String::new(),
                            wasm: String::new(),
                            skip: "non-invoke assert_trap".into(),
                            args: vec![],
                            arg_ty: vec![],
                            expected: vec![],
                            exp_ty: vec![],
                        });
                        continue;
                    };
                    if let Some(reason) = &current_skip {
                        cases.push(Case {
                            kind: "skip".into(),
                            file: rel.clone(),
                            export: inv.name.to_string(),
                            wasm: String::new(),
                            skip: reason.clone(),
                            args: vec![],
                            arg_ty: vec![],
                            expected: vec![],
                            exp_ty: vec![],
                        });
                        continue;
                    }
                    let Some(wasm) = &current_wasm else {
                        cases.push(Case {
                            kind: "skip".into(),
                            file: rel.clone(),
                            export: inv.name.to_string(),
                            wasm: String::new(),
                            skip: "no module".into(),
                            args: vec![],
                            arg_ty: vec![],
                            expected: vec![],
                            exp_ty: vec![],
                        });
                        continue;
                    };
                    let mut args_v = Vec::new();
                    let mut arg_ty = Vec::new();
                    let mut bad: Option<String> = None;
                    for a in &inv.args {
                        match arg_core(a) {
                            Ok((bits, ty)) => {
                                args_v.push(bits.to_string());
                                arg_ty.push(ty.to_string());
                            }
                            Err(e) => bad = Some(e),
                        }
                    }
                    if let Some(e) = bad {
                        cases.push(Case {
                            kind: "skip".into(),
                            file: rel.clone(),
                            export: inv.name.to_string(),
                            wasm: wasm.clone(),
                            skip: e,
                            args: vec![],
                            arg_ty: vec![],
                            expected: vec![],
                            exp_ty: vec![],
                        });
                        continue;
                    }
                    cases.push(Case {
                        kind: "trap".into(),
                        file: rel.clone(),
                        export: inv.name.to_string(),
                        wasm: wasm.clone(),
                        skip: String::new(),
                        args: args_v,
                        arg_ty,
                        expected: vec![],
                        exp_ty: vec![],
                    });
                }
                WastDirective::AssertInvalid { .. }
                | WastDirective::AssertMalformed { .. }
                | WastDirective::AssertUnlinkable { .. }
                | WastDirective::AssertExhaustion { .. }
                | WastDirective::AssertException { .. }
                | WastDirective::AssertSuspension { .. } => {
                    cases.push(Case {
                        kind: "skip".into(),
                        file: rel.clone(),
                        export: String::new(),
                        wasm: String::new(),
                        skip: "assert_invalid/malformed".into(),
                        args: vec![],
                        arg_ty: vec![],
                        expected: vec![],
                        exp_ty: vec![],
                    });
                }
                _ => {}
            }
        }
    }

    let jsonl = out.join("catalog.jsonl");
    let mut body = String::new();
    for c in &cases {
        body.push_str(&serde_json::to_string(c).unwrap());
        body.push('\n');
    }
    fs::write(&jsonl, body).unwrap();
    eprintln!(
        "catalog: {} cases, {} wasm modules, {} parse-fail files, {} wast files -> {}",
        cases.len(),
        wasm_n,
        parse_fail,
        files.len(),
        jsonl.display()
    );
}
