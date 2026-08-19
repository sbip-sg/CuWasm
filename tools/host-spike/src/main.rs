//! T21: embed `soroban-env-host`, register a contract WASM, call one Env method
//! *outside* `Host::call`.

use soroban_env_host::{Env, Host};

fn main() {
    let host = Host::test_host_with_recording_footprint();
    host.set_test_ledger_info_with_current_test_protocol();

    let wasm = include_bytes!("../../../contracts/wasm/soroban_hello_world_contract.wasm");
    let addr = host.register_test_contract_wasm(wasm);

    let obj = host.obj_from_u64(42).expect("obj_from_u64");
    let back = host.obj_to_u64(obj).expect("obj_to_u64");
    assert_eq!(back, 42, "round-trip obj_from_u64/obj_to_u64");

    println!(
        "{{\"status\":\"ok\",\"contract\":{:?},\"obj_from_u64\":42,\"obj_to_u64\":{}}}",
        addr, back
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_spike() {
        let host = Host::test_host_with_recording_footprint();
        host.set_test_ledger_info_with_current_test_protocol();
        let wasm = include_bytes!("../../../contracts/wasm/soroban_hello_world_contract.wasm");
        let _addr = host.register_test_contract_wasm(wasm);
        let obj = host.obj_from_u64(7).unwrap();
        assert_eq!(host.obj_to_u64(obj).unwrap(), 7);
    }
}
