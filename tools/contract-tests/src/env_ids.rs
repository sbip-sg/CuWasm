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
    by_id: Vec<(String, String)>,
}

fn build() -> Table {
    let json = include_str!("../../../docs/soroban-env.json");
    let root: EnvRoot = serde_json::from_str(json).expect("soroban-env.json");
    let mut by_import = HashMap::new();
    let mut by_id = Vec::new();
    let mut id = 0u32;
    for m in root.modules {
        for f in m.functions {
            by_import.insert((m.export.clone(), f.export.clone()), id);
            by_id.push((format!("{}::{}", m.export, f.export), f.name));
            id += 1;
        }
    }
    Table { by_import, by_id }
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(build)
}

pub fn name(fn_id: u32) -> Option<&'static str> {
    table().by_id.get(fn_id as usize).map(|(_, n)| n.as_str())
}

pub fn import_key(fn_id: u32) -> Option<&'static str> {
    table().by_id.get(fn_id as usize).map(|(k, _)| k.as_str())
}
