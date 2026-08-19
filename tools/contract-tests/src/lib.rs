mod capi;
mod dispatch;
mod env_ids;

pub use capi::{CUWASM_CAPI_OP_COUNT, *};
pub use dispatch::{host_dispatch, DispatchCtx};

use capi::CuwasmRunProfile;
use soroban_env_common::Val;
use soroban_env_host::Host;
use std::ffi::CString;

pub struct RunProfileOut {
    pub host_calls: Vec<(String, u16)>,
    pub opcode_counts: [u64; capi::CUWASM_CAPI_OP_COUNT],
    pub unsupported_opcode_counts: [u64; capi::CUWASM_CAPI_OP_COUNT],
    pub total_ops: u64,
}

pub fn run_cuwasm_profile(
    host: &Host,
    wasm: &[u8],
    export: &str,
    args: &[Val],
) -> Result<(Val, RunProfileOut), soroban_env_host::HostError> {
    let mut err = [0u8; 256];
    let module = unsafe { cuwasm_module_load(wasm.as_ptr(), wasm.len(), err.as_mut_ptr(), err.len()) };
    if module.is_null() {
        panic!("translate: {}", String::from_utf8_lossy(&err));
    }
    let fi = unsafe {
        cuwasm_module_export_index(module, CString::new(export).unwrap().as_ptr() as *const u8)
    };
    if fi < 0 {
        panic!("missing export {export}");
    }
    let mem = unsafe { cuwasm_module_memory(module) };
    let mem_size = unsafe { cuwasm_module_memory_size(module) };
    let mut ctx = DispatchCtx {
        host: host.clone(),
        mem,
        mem_size,
        relative_objects: Vec::new(),
        host_calls: Vec::new(),
    };
    let mut rel_args = Vec::with_capacity(args.len());
    for &arg in args {
        rel_args.push(ctx.to_relative(arg).expect("to_relative arg"));
    }
    let payloads: Vec<u64> = rel_args.iter().map(|v| v.get_payload()).collect();
    let mut out = CuwasmRunResult {
        status: 0,
        results: [0; 8],
        n_results: 0,
        error: [0; 256],
    };
    let mut profile = CuwasmRunProfile {
        opcode_counts: [0; capi::CUWASM_CAPI_OP_COUNT],
        unsupported_opcode_counts: [0; capi::CUWASM_CAPI_OP_COUNT],
        total_ops: 0,
    };
    let rc = unsafe {
        cuwasm_module_run(
            module,
            fi as u32,
            if payloads.is_empty() {
                std::ptr::null()
            } else {
                payloads.as_ptr()
            },
            payloads.len() as u32,
            10_000_000,
            host_dispatch,
            &mut ctx as *mut DispatchCtx as *mut std::os::raw::c_void,
            &mut out,
            &mut profile,
        )
    };
    unsafe { cuwasm_module_free(module) };
    if rc != 0 {
        panic!(
            "cuwasm run status={} err={}",
            out.status,
            String::from_utf8_lossy(&out.error)
        );
    }
    let got = if out.n_results == 0 {
        ().into()
    } else {
        assert_eq!(out.n_results, 1);
        let got_relative = Val::from_payload(out.results[0]);
        ctx.to_absolute(got_relative).expect("to_absolute result")
    };
    Ok((
        got,
        RunProfileOut {
            host_calls: ctx.host_calls,
            opcode_counts: profile.opcode_counts,
            unsupported_opcode_counts: profile.unsupported_opcode_counts,
            total_ops: profile.total_ops,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_env_common::xdr::ScAddress;
    use soroban_env_common::{Compare, Env, EnvBase, Symbol, Val};
    use soroban_env_host::Host;
    use std::cmp::Ordering;

    const HELLO_WASM: &[u8] =
        include_bytes!("../../../contracts/wasm/soroban_hello_world_contract.wasm");
    const INCREMENT_WASM: &[u8] =
        include_bytes!("../../../contracts/wasm/soroban_increment_contract.wasm");
    const TOKEN_WASM: &[u8] =
        include_bytes!("../../../contracts/wasm/soroban_token_contract.wasm");

    fn run_cuwasm(
        host: &Host,
        wasm: &[u8],
        export: &str,
        args: &[Val],
    ) -> Result<Val, soroban_env_host::HostError> {
        Ok(run_cuwasm_profile(host, wasm, export, args)?.0)
    }

    fn contract_hash(host: &Host, id: soroban_env_common::AddressObject) -> soroban_env_common::xdr::Hash {
        match host.scaddress_from_address(id).expect("scaddress") {
            ScAddress::Contract(h) => h,
            _ => panic!("expected contract address"),
        }
    }

    fn register_token_contract(host: &Host) -> Result<soroban_env_common::AddressObject, soroban_env_host::HostError> {
        use soroban_env_host::testutils::{generate_account_id, generate_bytes_array};

        let admin = generate_account_id(host);
        let prev_auth = host.snapshot_auth_manager()?;
        host.switch_to_recording_auth_inherited_from_snapshot(&prev_auth)?;
        let wasm_hash = host.upload_wasm(host.bytes_new_from_slice(TOKEN_WASM)?)?;
        host.set_source_account(admin.clone())?;
        let deployer = host
            .source_account_address()?
            .expect("deployer address");
        let salt = host.bytes_new_from_slice(&generate_bytes_array(host))?;
        let name = host.string_new_from_slice(b"TestToken")?;
        let symbol = host.string_new_from_slice(b"TT")?;
        let ctor = host.vec_new_from_slice(&[
            deployer.into(),
            7u32.into(),
            name.into(),
            symbol.into(),
        ])?;
        let id = host.create_contract_with_constructor(deployer, wasm_hash, salt, ctor)?;
        host.set_auth_manager(prev_auth)?;
        Ok(id)
    }

    fn account_address(
        host: &Host,
        account: soroban_env_common::xdr::AccountId,
    ) -> soroban_env_common::AddressObject {
        host.set_source_account(account).expect("set_source_account");
        host.source_account_address()
            .expect("source_account_address")
            .expect("account address")
    }

    fn i128_val(host: &Host, v: i128) -> Result<Val, soroban_env_host::HostError> {
        Ok(host
            .obj_from_i128_pieces((v >> 64) as i64, v as u64)?
            .into())
    }

    #[test]
    fn test_hello_world() -> Result<(), soroban_env_host::HostError> {
        let host = Host::test_host_with_recording_footprint();
        host.set_test_ledger_info_with_current_test_protocol();
        let hello_id = host.register_test_contract_wasm(HELLO_WASM);
        let arg = host.string_new_from_slice(b"World")?;
        let reference = host.call(
            hello_id,
            Symbol::try_from_small_str("hello")?,
            host.vec_new_from_slice(&[arg.into()])?,
        )?;

        let cuwasm_arg: Val = host.string_new_from_slice(b"World")?.into();

        let got = host.with_test_contract_frame(
            contract_hash(&host, hello_id),
            Symbol::try_from_small_str("hello")?,
            || run_cuwasm(&host, HELLO_WASM, "hello", &[cuwasm_arg]),
        )?;

        assert_eq!(host.compare(&reference, &got)?, Ordering::Equal);
        Ok(())
    }

    #[test]
    fn test_increment() -> Result<(), soroban_env_host::HostError> {
        let host = Host::test_host_with_recording_footprint();
        host.set_test_ledger_info_with_current_test_protocol();
        let id = host.register_test_contract_wasm(INCREMENT_WASM);

        let reference = host.call(
            id,
            Symbol::try_from_small_str("increment")?,
            host.vec_new_from_slice(&[])?,
        )?;
        let reference2 = host.call(
            id,
            Symbol::try_from_small_str("increment")?,
            host.vec_new_from_slice(&[])?,
        )?;

        let host2 = Host::test_host_with_recording_footprint();
        host2.set_test_ledger_info_with_current_test_protocol();
        let id2 = host2.register_test_contract_wasm(INCREMENT_WASM);
        let hash2 = contract_hash(&host2, id2);

        let got = host2.with_test_contract_frame(
            hash2.clone(),
            Symbol::try_from_small_str("increment")?,
            || run_cuwasm(&host2, INCREMENT_WASM, "increment", &[]),
        )?;
        let got2 = host2.with_test_contract_frame(
            hash2,
            Symbol::try_from_small_str("increment")?,
            || run_cuwasm(&host2, INCREMENT_WASM, "increment", &[]),
        )?;

        assert_eq!(host.compare(&reference, &got)?, Ordering::Equal);
        assert_eq!(host.compare(&reference2, &got2)?, Ordering::Equal);
        Ok(())
    }

    #[test]
    fn test_token() -> Result<(), soroban_env_host::HostError> {
        let host = Host::test_host_with_recording_footprint();
        host.set_test_ledger_info_with_current_test_protocol();
        let token_id = register_token_contract(&host)?;

        let alice = account_address(&host, soroban_env_host::testutils::generate_account_id(&host));
        let bob = account_address(&host, soroban_env_host::testutils::generate_account_id(&host));
        let mint_amt = i128_val(&host, 1000)?;
        let xfer_amt = i128_val(&host, 400)?;

        host.switch_to_recording_auth(true)?;

        host.call(
            token_id,
            Symbol::try_from_small_str("mint")?,
            host.vec_new_from_slice(&[alice.into(), mint_amt])?,
        )?;
        let ref_bal = host.call(
            token_id,
            Symbol::try_from_small_str("balance")?,
            host.vec_new_from_slice(&[alice.into()])?,
        )?;

        host.call(
            token_id,
            Symbol::try_from_small_str("transfer")?,
            host.vec_new_from_slice(&[alice.into(), bob.into(), xfer_amt])?,
        )?;
        let ref_alice = host.call(
            token_id,
            Symbol::try_from_small_str("balance")?,
            host.vec_new_from_slice(&[alice.into()])?,
        )?;
        let ref_bob = host.call(
            token_id,
            Symbol::try_from_small_str("balance")?,
            host.vec_new_from_slice(&[bob.into()])?,
        )?;

        let host2 = Host::test_host_with_recording_footprint();
        host2.set_test_ledger_info_with_current_test_protocol();
        let token_id2 = register_token_contract(&host2)?;
        let hash2 = contract_hash(&host2, token_id2);
        let alice2 = account_address(
            &host2,
            soroban_env_host::testutils::generate_account_id(&host2),
        );
        let bob2 = account_address(
            &host2,
            soroban_env_host::testutils::generate_account_id(&host2),
        );
        let mint_amt2 = i128_val(&host2, 1000)?;
        let xfer_amt2 = i128_val(&host2, 400)?;
        host2.switch_to_recording_auth(true)?;

        host2.with_test_contract_frame(
            hash2.clone(),
            Symbol::try_from_small_str("mint")?,
            || run_cuwasm(&host2, TOKEN_WASM, "mint", &[alice2.into(), mint_amt2]),
        )?;
        let cu_bal = host2.with_test_contract_frame(
            hash2.clone(),
            Symbol::try_from_small_str("balance")?,
            || run_cuwasm(&host2, TOKEN_WASM, "balance", &[alice2.into()]),
        )?;

        host2.with_test_contract_frame(
            hash2.clone(),
            Symbol::try_from_small_str("transfer")?,
            || run_cuwasm(
                &host2,
                TOKEN_WASM,
                "transfer",
                &[alice2.into(), bob2.into(), xfer_amt2],
            ),
        )?;
        let cu_alice = host2.with_test_contract_frame(
            hash2.clone(),
            Symbol::try_from_small_str("balance")?,
            || run_cuwasm(&host2, TOKEN_WASM, "balance", &[alice2.into()]),
        )?;
        let cu_bob = host2.with_test_contract_frame(
            hash2,
            Symbol::try_from_small_str("balance")?,
            || run_cuwasm(&host2, TOKEN_WASM, "balance", &[bob2.into()]),
        )?;

        assert_eq!(host.compare(&ref_bal, &cu_bal)?, Ordering::Equal);
        assert_eq!(host.compare(&ref_alice, &cu_alice)?, Ordering::Equal);
        assert_eq!(host.compare(&ref_bob, &cu_bob)?, Ordering::Equal);
        Ok(())
    }
}
