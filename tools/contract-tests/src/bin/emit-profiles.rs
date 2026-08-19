//! Emit host-call and opcode run profiles (FR-27 / FR-28) into docs/.

use cuwasm_contract_tests::{
    run_cuwasm_profile, RunProfileOut, CUWASM_CAPI_OP_COUNT,
};
use serde::Serialize;
use soroban_env_common::xdr::ScAddress;
use soroban_env_common::{Env, EnvBase, Symbol, Val};
use soroban_env_host::testutils::{generate_account_id, generate_bytes_array};
use soroban_env_host::Host;
use std::collections::BTreeMap;
use std::path::PathBuf;

const HELLO_WASM: &[u8] = include_bytes!("../../../../contracts/wasm/soroban_hello_world_contract.wasm");
const INCREMENT_WASM: &[u8] = include_bytes!("../../../../contracts/wasm/soroban_increment_contract.wasm");
const TOKEN_WASM: &[u8] = include_bytes!("../../../../contracts/wasm/soroban_token_contract.wasm");

fn opcode_names() -> &'static [&'static str] {
    &[
        "unreachable", "i64.const", "local.get", "local.set", "i64.add", "i64.sub", "i64.eq",
        "i64.eqz", "i64.le_s", "i64.lt_s", "br", "br_if_not", "call", "return_call", "return",
        "end", "drop", "select", "i32.eqz", "i32.eq", "i32.ne", "i32.lt_s", "i32.lt_u",
        "i32.le_s", "i32.le_u", "i32.gt_s", "i32.gt_u", "i32.ge_s", "i32.ge_u", "i32.add",
        "i32.sub", "i32.mul", "i32.and", "i32.or", "i32.xor", "i32.div_s", "i32.div_u",
        "i32.rem_s", "i32.rem_u", "i32.shl", "i32.shr_s", "i32.shr_u", "i32.wrap_i64",
        "i64.ne", "i64.lt_u", "i64.le_u", "i64.gt_s", "i64.gt_u", "i64.ge_s", "i64.ge_u",
        "i64.mul", "i64.and", "i64.or", "i64.xor", "i64.div_s", "i64.div_u", "i64.rem_s",
        "i64.rem_u", "i64.shl", "i64.shr_s", "i64.shr_u", "i64.extend_i32_s",
        "i64.extend_i32_u", "global.get", "global.set", "unwind", "i64.mul_wide_s",
        "i64.mul_wide_u", "i64.add128", "i64.sub128", "load", "store", "memory.size",
        "memory.grow", "memory.copy", "memory.fill", "memory.init", "data.drop", "call_host",
        "call_indirect", "i32.clz",
    ]
}

#[derive(Serialize)]
struct HostCallEntry {
    name: String,
    arg_count: u16,
    count: u64,
}

#[derive(Serialize)]
struct OpcodeEntry {
    op: String,
    count: u64,
}

#[derive(Serialize)]
struct ScenarioProfile {
    export: String,
    host_calls: Vec<HostCallEntry>,
    host_calls_total: u64,
    opcodes: Vec<OpcodeEntry>,
    opcodes_total: u64,
    unsupported_opcodes: Vec<OpcodeEntry>,
}

#[derive(Serialize)]
struct ContractProfile {
    contract: String,
    scenarios: Vec<ScenarioProfile>,
}

fn merge_profiles(profiles: &[RunProfileOut]) -> (Vec<HostCallEntry>, Vec<OpcodeEntry>, Vec<OpcodeEntry>, u64, u64) {
    let mut host_map: BTreeMap<(String, u16), u64> = BTreeMap::new();
    let mut op_map = [0u64; CUWASM_CAPI_OP_COUNT];
    let mut uns_map = [0u64; CUWASM_CAPI_OP_COUNT];
    let mut host_total = 0u64;
    let mut op_total = 0u64;
    for p in profiles {
        for (name, n) in &p.host_calls {
            *host_map.entry((name.clone(), *n)).or_insert(0) += 1;
            host_total += 1;
        }
        for i in 0..CUWASM_CAPI_OP_COUNT {
            op_map[i] += p.opcode_counts[i];
            uns_map[i] += p.unsupported_opcode_counts[i];
        }
        op_total += p.total_ops;
    }
    let names = opcode_names();
    let host_calls: Vec<_> = host_map
        .into_iter()
        .map(|((name, arg_count), count)| HostCallEntry {
            name,
            arg_count,
            count,
        })
        .collect();
    let mut opcodes: Vec<_> = op_map
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .map(|(i, c)| OpcodeEntry {
            op: names.get(i).copied().unwrap_or("?").to_string(),
            count: *c,
        })
        .collect();
    opcodes.sort_by(|a, b| b.count.cmp(&a.count));
    let unsupported_opcodes: Vec<_> = uns_map
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .map(|(i, c)| OpcodeEntry {
            op: names.get(i).copied().unwrap_or("?").to_string(),
            count: *c,
        })
        .collect();
    (host_calls, opcodes, unsupported_opcodes, host_total, op_total)
}

fn scenario(export: &str, profiles: &[RunProfileOut]) -> ScenarioProfile {
    let (host_calls, opcodes, unsupported_opcodes, host_calls_total, opcodes_total) =
        merge_profiles(profiles);
    ScenarioProfile {
        export: export.to_string(),
        host_calls,
        host_calls_total,
        opcodes,
        opcodes_total,
        unsupported_opcodes,
    }
}

fn contract_hash(host: &Host, id: soroban_env_common::AddressObject) -> soroban_env_common::xdr::Hash {
    match host.scaddress_from_address(id).expect("scaddress") {
        ScAddress::Contract(h) => h,
        _ => panic!("expected contract address"),
    }
}

fn register_token(host: &Host) -> soroban_env_common::AddressObject {
    let admin = generate_account_id(host);
    let prev_auth = host.snapshot_auth_manager().expect("snapshot auth");
    host.switch_to_recording_auth_inherited_from_snapshot(&prev_auth)
        .expect("recording auth");
    let wasm_hash = host
        .upload_wasm(host.bytes_new_from_slice(TOKEN_WASM).expect("bytes"))
        .expect("upload");
    host.set_source_account(admin.clone()).expect("source");
    let deployer = host.source_account_address().expect("addr").expect("deployer");
    let salt = host
        .bytes_new_from_slice(&generate_bytes_array(host))
        .expect("salt");
    let name = host.string_new_from_slice(b"TestToken").expect("name");
    let symbol = host.string_new_from_slice(b"TT").expect("symbol");
    let ctor = host
        .vec_new_from_slice(&[
            deployer.into(),
            7u32.into(),
            name.into(),
            symbol.into(),
        ])
        .expect("ctor");
    let id = host
        .create_contract_with_constructor(deployer, wasm_hash, salt, ctor)
        .expect("create");
    host.set_auth_manager(prev_auth).expect("restore auth");
    id
}

fn account_address(host: &Host, account: soroban_env_common::xdr::AccountId) -> soroban_env_common::AddressObject {
    host.set_source_account(account).expect("set_source_account");
    host.source_account_address()
        .expect("source_account_address")
        .expect("account address")
}

fn i128_val(host: &Host, v: i128) -> Val {
    host.obj_from_i128_pieces((v >> 64) as i64, v as u64)
        .expect("i128")
        .into()
}

fn profile_hello(host: &Host, hash: soroban_env_common::xdr::Hash) -> RunProfileOut {
    let arg: Val = host.string_new_from_slice(b"World").expect("str").into();
    let mut pr = None;
    host.with_test_contract_frame(hash, Symbol::try_from_small_str("hello").unwrap(), || {
        let (v, p) = run_cuwasm_profile(host, HELLO_WASM, "hello", &[arg]).expect("hello profile");
        pr = Some(p);
        Ok(v)
    })
    .expect("hello frame");
    pr.unwrap()
}

fn profile_increment(host: &Host, hash: soroban_env_common::xdr::Hash) -> Vec<RunProfileOut> {
    let sym = Symbol::try_from_small_str("increment").unwrap();
    let p1 = {
        let mut pr = None;
        host.with_test_contract_frame(hash.clone(), sym, || {
            let (v, p) = run_cuwasm_profile(host, INCREMENT_WASM, "increment", &[]).expect("inc1");
            pr = Some(p);
            Ok(v)
        }).expect("inc1 frame");
        pr.unwrap()
    };
    let p2 = {
        let mut pr = None;
        host.with_test_contract_frame(hash, sym, || {
            let (v, p) = run_cuwasm_profile(host, INCREMENT_WASM, "increment", &[]).expect("inc2");
            pr = Some(p);
            Ok(v)
        }).expect("inc2 frame");
        pr.unwrap()
    };
    vec![p1, p2]
}

fn profile_token(host: &Host, hash: soroban_env_common::xdr::Hash) -> Vec<RunProfileOut> {
    host.switch_to_recording_auth(true).expect("recording auth");
    let alice = account_address(host, generate_account_id(host));
    let bob = account_address(host, generate_account_id(host));
    let mint_amt = i128_val(host, 1000);
    let xfer_amt = i128_val(host, 400);
    let mut out = Vec::new();
    macro_rules! run_capture {
        ($sym:expr, $export:expr, $args:expr) => {{
            let mut pr = None;
            host.with_test_contract_frame(
                hash.clone(),
                Symbol::try_from_small_str($sym).unwrap(),
                || {
                    let (v, p) = run_cuwasm_profile(host, TOKEN_WASM, $export, $args)
                        .expect(concat!($sym, " profile"));
                    pr = Some(p);
                    Ok(v)
                },
            )
            .expect(concat!($sym, " frame"));
            pr.unwrap()
        }};
    }
    out.push(run_capture!("mint", "mint", &[alice.into(), mint_amt]));
    out.push(run_capture!("balance", "balance", &[alice.into()]));
    out.push(run_capture!("transfer", "transfer", &[alice.into(), bob.into(), xfer_amt]));
    out.push(run_capture!("balance", "balance", &[bob.into()]));
    out
}

fn write_profile(path: PathBuf, doc: &ContractProfile) {
    let json = serde_json::to_string_pretty(doc).expect("json");
    std::fs::write(&path, json).expect("write profile");
    println!("wrote {}", path.display());
}

fn main() {
    assert_eq!(opcode_names().len(), CUWASM_CAPI_OP_COUNT, "opcode name table");
    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs");

    let host = Host::test_host_with_recording_footprint();
    host.set_test_ledger_info_with_current_test_protocol();
    let hello_id = host.register_test_contract_wasm(HELLO_WASM);
    let hello_hash = contract_hash(&host, hello_id);
    let hello_p = profile_hello(&host, hello_hash);
    write_profile(
        docs.join("hello_world-run-profile.json"),
        &ContractProfile {
            contract: "soroban_hello_world_contract.wasm".into(),
            scenarios: vec![scenario("hello", std::slice::from_ref(&hello_p))],
        },
    );

    let host = Host::test_host_with_recording_footprint();
    host.set_test_ledger_info_with_current_test_protocol();
    let inc_id = host.register_test_contract_wasm(INCREMENT_WASM);
    let inc_hash = contract_hash(&host, inc_id);
    let inc_ps = profile_increment(&host, inc_hash);
    write_profile(
        docs.join("increment-run-profile.json"),
        &ContractProfile {
            contract: "soroban_increment_contract.wasm".into(),
            scenarios: vec![scenario("increment", &inc_ps)],
        },
    );

    let host = Host::test_host_with_recording_footprint();
    host.set_test_ledger_info_with_current_test_protocol();
    let token_id = register_token(&host);
    let token_hash = contract_hash(&host, token_id);
    let token_ps = profile_token(&host, token_hash);
    write_profile(
        docs.join("token-run-profile.json"),
        &ContractProfile {
            contract: "soroban_token_contract.wasm".into(),
            scenarios: vec![scenario("mint/balance/transfer/balance", &token_ps)],
        },
    );
}
