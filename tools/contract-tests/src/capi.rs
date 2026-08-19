use std::os::raw::{c_int, c_void};

pub const CUWASM_CAPI_OP_COUNT: usize = 81;

#[repr(C)]
pub struct CuwasmModule {
    _private: [u8; 0],
}

#[repr(C)]
pub struct CuwasmMailbox {
    pub fn_id: u32,
    pub n_args: u16,
    pub n_results: u16,
    pub args: [u64; 16],
    pub results: [u64; 1],
}

pub type CuwasmHostFn =
    extern "C" fn(ctx: *mut c_void, mb: *mut CuwasmMailbox, err: *mut u8, err_cap: usize) -> c_int;

#[repr(C)]
pub struct CuwasmRunResult {
    pub status: u16,
    pub results: [u64; 8],
    pub n_results: u32,
    pub error: [u8; 256],
}

#[repr(C)]
pub struct CuwasmRunProfile {
    pub opcode_counts: [u64; CUWASM_CAPI_OP_COUNT],
    pub unsupported_opcode_counts: [u64; CUWASM_CAPI_OP_COUNT],
    pub total_ops: u64,
}

extern "C" {
    pub fn cuwasm_module_load(
        wasm: *const u8,
        len: usize,
        err: *mut u8,
        err_cap: usize,
    ) -> *mut CuwasmModule;
    pub fn cuwasm_module_free(m: *mut CuwasmModule);
    pub fn cuwasm_module_export_index(m: *mut CuwasmModule, name: *const u8) -> c_int;
    pub fn cuwasm_module_memory(m: *mut CuwasmModule) -> *mut u8;
    pub fn cuwasm_module_memory_size(m: *mut CuwasmModule) -> u32;
    pub fn cuwasm_module_run(
        m: *mut CuwasmModule,
        func_idx: u32,
        args: *const u64,
        n_args: u32,
        max_steps: u64,
        host: CuwasmHostFn,
        ctx: *mut c_void,
        out: *mut CuwasmRunResult,
        profile: *mut CuwasmRunProfile,
    ) -> c_int;
}
