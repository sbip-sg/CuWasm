use crate::capi::CuwasmMailbox;
use crate::env_ids;
use soroban_env_common::{Env, EnvBase, Object, StorageType, Tag, U32Val, Val};
use soroban_env_host::Host;
use std::os::raw::{c_int, c_void};

pub struct DispatchCtx {
    pub host: Host,
    pub mem: *mut u8,
    pub mem_size: u32,
    /// Mirrors ContractVM's relative object table for guest-visible handles.
    pub relative_objects: Vec<Val>,
}

impl DispatchCtx {
    fn read_bytes(&self, pos: u32, len: u32) -> Result<Vec<u8>, String> {
        let end = pos as u64 + len as u64;
        if end > self.mem_size as u64 {
            return Err("guest memory oob".into());
        }
        let slice = unsafe { std::slice::from_raw_parts(self.mem.add(pos as usize), len as usize) };
        Ok(slice.to_vec())
    }

    fn read_vals(&self, pos: u32, len: u32) -> Result<Vec<Val>, String> {
        let nbytes = len as u64 * 8;
        let end = pos as u64 + nbytes;
        if end > self.mem_size as u64 {
            return Err("guest val slice oob".into());
        }
        let mut out = Vec::with_capacity(len as usize);
        for i in 0..len {
            let off = pos as usize + (i as usize * 8);
            let mut buf = [0u8; 8];
            unsafe {
                std::ptr::copy_nonoverlapping(self.mem.add(off), buf.as_mut_ptr(), 8);
            }
            let val = Val::from_payload(u64::from_le_bytes(buf));
            out.push(self.to_absolute(val)?);
        }
        Ok(out)
    }

    fn is_relative_handle(handle: u32) -> bool {
        handle & 1 == 0
    }

    fn handle_to_index(handle: u32) -> usize {
        (handle as usize) >> 1
    }

    fn index_to_relative_handle(index: usize) -> Result<u32, String> {
        let smaller = u32::try_from(index).map_err(|_| "relative object index overflow")?;
        smaller
            .checked_shl(1)
            .ok_or_else(|| "relative object handle overflow".to_string())
    }

    pub fn to_relative(&mut self, val: Val) -> Result<Val, String> {
        if Object::try_from(val).is_err() {
            return Ok(val);
        }
        let tag = val.get_tag();
        let index = self.relative_objects.len();
        self.relative_objects.push(val);
        let handle = Self::index_to_relative_handle(index)?;
        Ok(Object::from_handle_and_tag(handle, tag).into())
    }

    pub fn to_absolute(&self, val: Val) -> Result<Val, String> {
        if let Ok(obj) = Object::try_from(val) {
            let handle = obj.get_handle();
            if Self::is_relative_handle(handle) {
                let index = Self::handle_to_index(handle);
                let abs = self
                    .relative_objects
                    .get(index)
                    .copied()
                    .ok_or_else(|| format!("relative object index {index} out of range"))?;
                if abs.get_tag() != val.get_tag() {
                    return Err("relative object tag mismatch".into());
                }
                return Ok(abs);
            }
        }
        Ok(val)
    }

    fn decode_u32(raw: u64) -> Result<u32, String> {
        let val = Val::from_payload(raw);
        if val.get_tag() != Tag::U32Val {
            return Err(format!("expected U32Val, got {:?}", val.get_tag()));
        }
        const MINOR_BITS: u32 = 24;
        const MINOR_MASK: u64 = (1u64 << MINOR_BITS) - 1;
        let body = raw >> 8;
        Ok(((body & !MINOR_MASK) >> MINOR_BITS) as u32)
    }

    fn decode_val(&self, raw: u64) -> Result<Val, String> {
        self.to_absolute(Val::from_payload(raw))
    }

    fn decode_storage_type(raw: u64) -> Result<StorageType, String> {
        match raw {
            0 => Ok(StorageType::Temporary),
            1 => Ok(StorageType::Persistent),
            2 => Ok(StorageType::Instance),
            _ => Err(format!("bad StorageType {raw}")),
        }
    }

    fn finish(&mut self, mb: &mut CuwasmMailbox, res: Option<Val>) -> Result<(), String> {
        match res {
            None => {
                // Soroban wasm imports always have an i64 result slot, even for Void.
                mb.n_results = 1;
                let void_val: Val = ().into();
                mb.results[0] = void_val.get_payload();
            }
            Some(v) => {
                let out = if Object::try_from(v).is_ok() {
                    self.to_relative(v)?
                } else {
                    v
                };
                mb.n_results = 1;
                mb.results[0] = out.get_payload();
            }
        }
        Ok(())
    }

    pub fn dispatch(&mut self, mb: &mut CuwasmMailbox) -> Result<(), String> {
        let name = env_ids::name(mb.fn_id).unwrap_or("?");
        let res: Option<Val> = match name {
            "string_new_from_linear_memory" => {
                let pos = Self::decode_u32(mb.args[0])?;
                let len = Self::decode_u32(mb.args[1])?;
                let bytes = self.read_bytes(pos, len)?;
                Some(
                    self.host
                        .string_new_from_slice(&bytes)
                        .map_err(|e| format!("string_new_from_slice: {e:?}"))?
                        .into(),
                )
            }
            "vec_new_from_linear_memory" => {
                let pos = Self::decode_u32(mb.args[0])?;
                let len = Self::decode_u32(mb.args[1])?;
                let vals = self.read_vals(pos, len)?;
                Some(
                    self.host
                        .vec_new_from_slice(&vals)
                        .map_err(|e| format!("vec_new_from_slice: {e:?}"))?
                        .into(),
                )
            }
            "has_contract_data" => {
                let k = self.decode_val(mb.args[0])?;
                let t = Self::decode_storage_type(mb.args[1])?;
                Some(
                    self.host
                        .has_contract_data(k, t)
                        .map_err(|e| format!("has_contract_data: {e:?}"))?
                        .into(),
                )
            }
            "get_contract_data" => {
                let k = self.decode_val(mb.args[0])?;
                let t = Self::decode_storage_type(mb.args[1])?;
                Some(
                    self.host
                        .get_contract_data(k, t)
                        .map_err(|e| format!("get_contract_data: {e:?}"))?,
                )
            }
            "put_contract_data" => {
                let k = self.decode_val(mb.args[0])?;
                let v = self.decode_val(mb.args[1])?;
                let t = Self::decode_storage_type(mb.args[2])?;
                self.host
                    .put_contract_data(k, v, t)
                    .map_err(|e| format!("put_contract_data: {e:?}"))?;
                None
            }
            "extend_current_contract_instance_and_code_ttl" => {
                let threshold: U32Val = Self::decode_u32(mb.args[0])?.into();
                let extend_to: U32Val = Self::decode_u32(mb.args[1])?.into();
                self.host
                    .extend_current_contract_instance_and_code_ttl(threshold, extend_to)
                    .map_err(|e| format!("extend_current_contract_instance_and_code_ttl: {e:?}"))?;
                None
            }
            other => {
                let key = env_ids::import_key(mb.fn_id).unwrap_or("?");
                return Err(format!("unimplemented host function {key} ({other})"));
            }
        };
        self.finish(mb, res)
    }
}

pub extern "C" fn host_dispatch(
    ctx: *mut c_void,
    mb: *mut CuwasmMailbox,
    err: *mut u8,
    err_cap: usize,
) -> c_int {
    if ctx.is_null() || mb.is_null() {
        return 0;
    }
    let ctx = unsafe { &mut *(ctx as *mut DispatchCtx) };
    let mb = unsafe { &mut *mb };
    if let Err(msg) = ctx.dispatch(mb) {
        if !err.is_null() && err_cap > 0 {
            let bytes = msg.as_bytes();
            let n = bytes.len().min(err_cap - 1);
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), err, n);
                *err.add(n) = 0;
            }
        }
        return 0;
    }
    1
}
