use crate::capi::CuwasmMailbox;
use crate::env_ids;
use soroban_env_common::{EnvBase, Object, Tag, Val};
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

    pub fn dispatch(&mut self, mb: &mut CuwasmMailbox) -> Result<(), String> {
        let name = env_ids::name(mb.fn_id).unwrap_or("?");
        let res: Val = match name {
            "string_new_from_linear_memory" => {
                let pos = Self::decode_u32(mb.args[0])?;
                let len = Self::decode_u32(mb.args[1])?;
                let bytes = self.read_bytes(pos, len)?;
                self.host
                    .string_new_from_slice(&bytes)
                    .map_err(|e| format!("string_new_from_slice: {e:?}"))?
                    .into()
            }
            "vec_new_from_linear_memory" => {
                let pos = Self::decode_u32(mb.args[0])?;
                let len = Self::decode_u32(mb.args[1])?;
                let vals = self.read_vals(pos, len)?;
                self.host
                    .vec_new_from_slice(&vals)
                    .map_err(|e| format!("vec_new_from_slice: {e:?}"))?
                    .into()
            }
            other => {
                let key = env_ids::import_key(mb.fn_id).unwrap_or("?");
                return Err(format!("unimplemented host function {key} ({other})"));
            }
        };
        let rel = self.to_relative(res)?;
        mb.n_results = 1;
        mb.results[0] = rel.get_payload();
        Ok(())
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
