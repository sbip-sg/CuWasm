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

    fn run_cuwasm(host: &Host, wasm: &[u8], export: &str, arg: Val) -> Result<Val, soroban_env_host::HostError> {
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
        let relative_arg = ctx.to_relative(arg).expect("to_relative arg");
        let args = [relative_arg.get_payload()];
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
                args.as_ptr(),
                1,
                10_000_000,
                host_dispatch,
                &mut ctx as *mut DispatchCtx as *mut std::os::raw::c_void,
                &mut out,
            )
        };
        unsafe { cuwasm_module_free(module) };
        assert_eq!(rc, 0, "cuwasm run status={} err={}", out.status, String::from_utf8_lossy(&out.error));
        assert_eq!(out.n_results, 1);
        let got_relative = Val::from_payload(out.results[0]);
        Ok(ctx.to_absolute(got_relative).expect("to_absolute result"))
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

        let ScAddress::Contract(contract_hash) = host.scaddress_from_address(hello_id)? else {
            panic!("expected contract address");
        };
        let got = host.with_test_contract_frame(
            contract_hash,
            Symbol::try_from_small_str("hello")?,
            || run_cuwasm(&host, HELLO_WASM, "hello", cuwasm_arg),
        )?;

        assert_eq!(host.compare(&reference, &got)?, Ordering::Equal);
        Ok(())
    }
}
