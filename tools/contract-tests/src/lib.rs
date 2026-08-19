mod capi;
mod dispatch;
mod env_ids;

pub use capi::*;
pub use dispatch::{host_dispatch, DispatchCtx};

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_env_common::xdr::ScAddress;
    use soroban_env_common::{Compare, Env, EnvBase, Symbol, Val};
    use soroban_env_host::Host;
    use std::cmp::Ordering;
    use std::ffi::CString;

    const HELLO_WASM: &[u8] =
        include_bytes!("../../../contracts/wasm/soroban_hello_world_contract.wasm");
    const INCREMENT_WASM: &[u8] =
        include_bytes!("../../../contracts/wasm/soroban_increment_contract.wasm");

    fn run_cuwasm(
        host: &Host,
        wasm: &[u8],
        export: &str,
        args: &[Val],
    ) -> Result<Val, soroban_env_host::HostError> {
        let mut err = [0u8; 256];
        let module = unsafe { cuwasm_module_load(wasm.as_ptr(), wasm.len(), err.as_mut_ptr(), err.len()) };
        assert!(!module.is_null(), "translate: {}", String::from_utf8_lossy(&err));
        let fi = unsafe {
            cuwasm_module_export_index(module, CString::new(export).unwrap().as_ptr() as *const u8)
        };
        assert!(fi >= 0, "missing export {export}");
        let mem = unsafe { cuwasm_module_memory(module) };
        let mem_size = unsafe { cuwasm_module_memory_size(module) };
        let mut ctx = DispatchCtx {
            host: host.clone(),
            mem,
            mem_size,
            relative_objects: Vec::new(),
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
            )
        };
        unsafe { cuwasm_module_free(module) };
        assert_eq!(
            rc, 0,
            "cuwasm run status={} err={}",
            out.status,
            String::from_utf8_lossy(&out.error)
        );
        if out.n_results == 0 {
            return Ok(().into());
        }
        assert_eq!(out.n_results, 1);
        let got_relative = Val::from_payload(out.results[0]);
        Ok(ctx.to_absolute(got_relative).expect("to_absolute result"))
    }

    fn contract_hash(host: &Host, id: soroban_env_common::AddressObject) -> soroban_env_common::xdr::Hash {
        match host.scaddress_from_address(id).expect("scaddress") {
            ScAddress::Contract(h) => h,
            _ => panic!("expected contract address"),
        }
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
}
