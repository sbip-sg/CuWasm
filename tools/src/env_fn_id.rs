//! Stable fn_id table from docs/soroban-env.json (flattened module/function order).

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct EnvRoot {
    modules: Vec<EnvMod>,
}

#[derive(Deserialize)]
struct EnvMod {
    export: String,
    functions: Vec<EnvFn>,
}

#[derive(Deserialize)]
struct EnvFn {
    export: String,
    name: String,
}

struct Table {
    by_import: HashMap<(String, String), u32>,
    labels: Vec<String>,
}

fn build() -> Table {
    let json = include_str!("../../docs/soroban-env.json");
    let root: EnvRoot = serde_json::from_str(json).expect("soroban-env.json");
    let mut by_import = HashMap::new();
    let mut labels = Vec::new();
    let mut id = 0u32;
    for m in root.modules {
        for f in m.functions {
            by_import.insert((m.export.clone(), f.export.clone()), id);
            labels.push(format!("{}::{} ({})", m.export, f.export, f.name));
            id += 1;
        }
    }
    Table { by_import, labels }
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(build)
}

pub fn lookup(mod_export: &str, fn_export: &str) -> Result<u32, String> {
    table()
        .by_import
        .get(&(mod_export.to_string(), fn_export.to_string()))
        .copied()
        .ok_or_else(|| format!("unknown host import {mod_export}::{fn_export}"))
}

pub fn label(fn_id: u32) -> Option<&'static str> {
    table().labels.get(fn_id as usize).map(|s| s.as_str())
}

pub fn n_functions() -> u32 {
    table().labels.len() as u32
}
