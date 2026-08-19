//! Opcode / import / section inventory for a WASM module.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;
use wasmparser::{Parser, Payload};

fn op_name(s: &str) -> String {
    s.split('{').next().unwrap_or(s).trim().to_string()
}

fn profile(path: &Path) {
    let bytes = fs::read(path).expect("read wasm");
    let mut imports: Vec<(String, String, String)> = Vec::new();
    let mut exports: Vec<(String, String)> = Vec::new();
    let mut ops: BTreeMap<String, u64> = BTreeMap::new();
    let mut sections: BTreeMap<String, u32> = BTreeMap::new();
    let mut n_funcs = 0u32;
    let mut mem = String::from("none");
    let mut n_globals = 0u32;
    let mut n_tables = 0u32;
    let mut n_data = 0u32;
    let mut n_elem = 0u32;
    let mut customs: Vec<String> = Vec::new();
    let mut has_start = false;
    let mut float_ops = 0u64;
    let mut simd_ops = 0u64;

    for payload in Parser::new(0).parse_all(&bytes) {
        let payload = payload.expect("parse");
        match payload {
            Payload::Version { .. } => {}
            Payload::TypeSection(_) => *sections.entry("type".into()).or_default() += 1,
            Payload::ImportSection(r) => {
                *sections.entry("import".into()).or_default() += 1;
                for imp in r {
                    let imp = imp.expect("import");
                    let kind = format!("{:?}", imp.ty);
                    imports.push((imp.module.to_string(), imp.name.to_string(), kind));
                }
            }
            Payload::FunctionSection(r) => {
                *sections.entry("function".into()).or_default() += 1;
                n_funcs = r.count() as u32;
            }
            Payload::TableSection(r) => {
                *sections.entry("table".into()).or_default() += 1;
                n_tables = r.count() as u32;
            }
            Payload::MemorySection(r) => {
                *sections.entry("memory".into()).or_default() += 1;
                for m in r {
                    let m = m.expect("mem");
                    mem = format!(
                        "min={} max={:?} shared={} memory64={}",
                        m.initial, m.maximum, m.shared, m.memory64
                    );
                }
            }
            Payload::GlobalSection(r) => {
                *sections.entry("global".into()).or_default() += 1;
                n_globals = r.count() as u32;
            }
            Payload::ExportSection(r) => {
                *sections.entry("export".into()).or_default() += 1;
                for ex in r {
                    let ex = ex.expect("export");
                    exports.push((ex.name.to_string(), format!("{:?}", ex.kind)));
                }
            }
            Payload::StartSection { .. } => {
                has_start = true;
                *sections.entry("start".into()).or_default() += 1;
            }
            Payload::ElementSection(r) => {
                *sections.entry("element".into()).or_default() += 1;
                n_elem = r.count() as u32;
            }
            Payload::DataSection(r) => {
                *sections.entry("data".into()).or_default() += 1;
                n_data = r.count() as u32;
            }
            Payload::DataCountSection { .. } => {
                *sections.entry("datacount".into()).or_default() += 1;
            }
            Payload::CodeSectionEntry(body) => {
                *sections.entry("code".into()).or_default() += 1;
                for op in body.get_operators_reader().expect("ops") {
                    let op = op.expect("op");
                    let dbg = format!("{op:?}");
                    let name = op_name(&dbg);
                    *ops.entry(name.clone()).or_default() += 1;
                    let l = name.to_ascii_lowercase();
                    if l.contains("f32") || l.contains("f64") {
                        float_ops += 1;
                    }
                    if l.contains("v128") || l.contains("i8x") || l.contains("i16x") {
                        simd_ops += 1;
                    }
                }
            }
            Payload::CustomSection(c) => {
                *sections.entry("custom".into()).or_default() += 1;
                customs.push(c.name().to_string());
            }
            _ => {}
        }
    }

    println!("file: {}", path.display());
    println!("bytes: {}", bytes.len());
    println!("memory: {mem}");
    println!("funcs: {n_funcs}  globals: {n_globals}  tables: {n_tables}  data: {n_data}  elem: {n_elem}  start: {has_start}");
    println!("float_ops: {float_ops}  simd_ops: {simd_ops}");
    println!("customs: {customs:?}");
    println!("imports ({}):", imports.len());
    for (m, n, k) in &imports {
        println!("  {m}::{n}  {k}");
    }
    println!("exports ({}):", exports.len());
    for (n, k) in &exports {
        println!("  {n}  {k}");
    }
    let mut ranked: Vec<_> = ops.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("opcodes ({} unique, {} total):", ranked.len(), ranked.iter().map(|(_, c)| c).sum::<u64>());
    for (n, c) in ranked {
        println!("  {c:6}  {n}");
    }
    println!();
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: wasm-profile <a.wasm>...");
        std::process::exit(2);
    }
    for a in args {
        profile(Path::new(&a));
    }
}
