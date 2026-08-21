use crate::capi::CuwasmMailbox;
use crate::env_ids;
use soroban_env_common::{
    AddressObject, Env, EnvBase, MapObject, Object, StorageType, Tag, TryFromVal, U32Val, Val,
    VecObject,
};
use soroban_env_common::xdr::{Limits, ScVal, WriteXdr};
use soroban_env_host::Host;
use std::os::raw::{c_int, c_void};

/// One host call with decoded storage key/value bytes for GPU planning.
#[derive(Clone, Debug)]
pub struct HostCallTrace {
    pub seq: u32,
    pub fn_id: u32,
    pub name: String,
    pub n_args: u16,
    pub raw_args: Vec<u64>,
    pub raw_result: u64,
    /// ScVal Debug for arg0 when it is a Val (keys, objects).
    pub arg0_scval: Option<String>,
    pub arg0_xdr_hex: Option<String>,
    pub arg0_sha256_hex: Option<String>,
    pub arg1_scval: Option<String>,
    pub arg1_xdr_hex: Option<String>,
    pub result_scval: Option<String>,
    pub result_xdr_hex: Option<String>,
    /// For vec_new: guest-relative element payloads read from linear memory.
    pub vec_elem_raw: Option<Vec<u64>>,
}

pub struct DispatchCtx {
    pub host: Host,
    pub mem: *mut u8,
    pub mem_size: u32,
    /// Mirrors ContractVM's relative object table for guest-visible handles.
    pub relative_objects: Vec<Val>,
    /// Ordered host calls for profiling (fn_name, arg_count).
    pub host_calls: Vec<(String, u16)>,
    /// Detailed per-call trace (always collected; cheap compared to host work).
    pub traces: Vec<HostCallTrace>,
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

    fn read_slice_descriptors(&self, pos: u32, len: u32) -> Result<Vec<(u32, u32)>, String> {
        let nbytes = len as u64 * 8;
        let end = pos as u64 + nbytes;
        if end > self.mem_size as u64 {
            return Err("guest slice descriptor oob".into());
        }
        let mut out = Vec::with_capacity(len as usize);
        for i in 0..len {
            let off = pos as usize + (i as usize * 8);
            let mut buf = [0u8; 8];
            unsafe {
                std::ptr::copy_nonoverlapping(self.mem.add(off), buf.as_mut_ptr(), 8);
            }
            let ptr = u32::from_le_bytes(buf[0..4].try_into().unwrap());
            let slen = u32::from_le_bytes(buf[4..8].try_into().unwrap());
            out.push((ptr, slen));
        }
        Ok(out)
    }

    fn read_symbol_keys(&self, pos: u32, len: u32) -> Result<Vec<String>, String> {
        let mut keys = Vec::with_capacity(len as usize);
        for (ptr, slen) in self.read_slice_descriptors(pos, len)? {
            let bytes = self.read_bytes(ptr, slen)?;
            keys.push(
                String::from_utf8(bytes)
                    .map_err(|e| format!("symbol key utf8: {e}"))?,
            );
        }
        Ok(keys)
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

    fn write_vals(&mut self, pos: u32, vals: &[Val]) -> Result<(), String> {
        let nbytes = vals.len() as u64 * 8;
        let end = pos as u64 + nbytes;
        if end > self.mem_size as u64 {
            return Err("guest val write oob".into());
        }
        for (i, &v) in vals.iter().enumerate() {
            let rel = self.to_relative(v)?;
            let payload = rel.get_payload().to_le_bytes();
            let off = pos as usize + i * 8;
            unsafe {
                std::ptr::copy_nonoverlapping(payload.as_ptr(), self.mem.add(off), 8);
            }
        }
        Ok(())
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

    fn finish_raw(&self, mb: &mut CuwasmMailbox, raw: u64) {
        mb.n_results = 1;
        mb.results[0] = raw;
    }

    fn hex_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn val_to_scval(&self, val: Val) -> Result<ScVal, String> {
        ScVal::try_from_val(&self.host, &val).map_err(|e| format!("ScVal convert: {e:?}"))
    }

    fn scval_xdr(sc: &ScVal) -> Result<Vec<u8>, String> {
        sc.to_xdr(Limits::none()).map_err(|e| format!("xdr: {e}"))
    }

    /// SHA-256 of XDR bytes, computed by the host (not a GPU-side hash).
    fn host_sha256(&self, bytes: &[u8]) -> Result<[u8; 32], String> {
        let obj = self
            .host
            .bytes_new_from_slice(bytes)
            .map_err(|e| format!("bytes_new_from_slice: {e:?}"))?;
        let hash_obj = self
            .host
            .compute_hash_sha256(obj)
            .map_err(|e| format!("compute_hash_sha256: {e:?}"))?;
        let mut out = [0u8; 32];
        self.host
            .bytes_copy_to_slice(hash_obj, 0u32.into(), &mut out)
            .map_err(|e| format!("bytes_copy_to_slice: {e:?}"))?;
        Ok(out)
    }

    fn describe_val(&self, raw: u64) -> (Option<String>, Option<String>, Option<String>) {
        let Ok(abs) = self.decode_val(raw) else {
            return (None, None, None);
        };
        let Ok(sc) = self.val_to_scval(abs) else {
            return (Some(format!("payload=0x{raw:016x}")), None, None);
        };
        let dbg = format!("{sc:?}");
        let xdr = DispatchCtx::scval_xdr(&sc).ok();
        let sha = match &xdr {
            Some(bytes) => self.host_sha256(bytes).ok().map(|h| DispatchCtx::hex_bytes(&h)),
            None => None,
        };
        (Some(dbg), xdr.map(|b| DispatchCtx::hex_bytes(&b)), sha)
    }

    fn record_trace(&mut self, mb: &CuwasmMailbox, name: &str) {
        let mut t = HostCallTrace {
            seq: self.traces.len() as u32,
            fn_id: mb.fn_id,
            name: name.to_string(),
            n_args: mb.n_args,
            raw_args: mb.args[..mb.n_args as usize].to_vec(),
            raw_result: mb.results[0],
            arg0_scval: None,
            arg0_xdr_hex: None,
            arg0_sha256_hex: None,
            arg1_scval: None,
            arg1_xdr_hex: None,
            result_scval: None,
            result_xdr_hex: None,
            vec_elem_raw: None,
        };
        match name {
            "has_contract_data" | "get_contract_data" | "extend_contract_data_ttl" => {
                let (d, x, h) = self.describe_val(mb.args[0]);
                t.arg0_scval = d;
                t.arg0_xdr_hex = x;
                t.arg0_sha256_hex = h;
            }
            "put_contract_data" => {
                let (d0, x0, h0) = self.describe_val(mb.args[0]);
                t.arg0_scval = d0;
                t.arg0_xdr_hex = x0;
                t.arg0_sha256_hex = h0;
                let (d1, x1, _) = self.describe_val(mb.args[1]);
                t.arg1_scval = d1;
                t.arg1_xdr_hex = x1;
            }
            "vec_new_from_linear_memory" => {
                if let (Ok(pos), Ok(len)) = (Self::decode_u32(mb.args[0]), Self::decode_u32(mb.args[1]))
                {
                    if let Ok(vals) = self.read_vals_raw(pos, len) {
                        t.vec_elem_raw = Some(vals);
                    }
                }
                let (d, x, h) = self.describe_val(mb.results[0]);
                t.result_scval = d;
                t.result_xdr_hex = x;
                t.arg0_sha256_hex = h;
            }
            "obj_from_i128_pieces" | "string_new_from_linear_memory" => {
                let (d, x, _) = self.describe_val(mb.results[0]);
                t.result_scval = d;
                t.result_xdr_hex = x;
            }
            _ => {}
        }
        if matches!(name, "get_contract_data") {
            let (d, x, _) = self.describe_val(mb.results[0]);
            t.result_scval = d;
            t.result_xdr_hex = x;
        }
        self.traces.push(t);
    }

    fn read_vals_raw(&self, pos: u32, len: u32) -> Result<Vec<u64>, String> {
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
            out.push(u64::from_le_bytes(buf));
        }
        Ok(out)
    }

    pub fn dispatch(&mut self, mb: &mut CuwasmMailbox) -> Result<(), String> {
        let name = env_ids::name(mb.fn_id).unwrap_or("?");
        self.host_calls.push((name.to_string(), mb.n_args));
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
            "symbol_new_from_linear_memory" => {
                let pos = Self::decode_u32(mb.args[0])?;
                let len = Self::decode_u32(mb.args[1])?;
                let bytes = self.read_bytes(pos, len)?;
                Some(
                    self.host
                        .symbol_new_from_slice(&bytes)
                        .map_err(|e| format!("symbol_new_from_slice: {e:?}"))?
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
            "map_new_from_linear_memory" => {
                let keys_pos = Self::decode_u32(mb.args[0])?;
                let vals_pos = Self::decode_u32(mb.args[1])?;
                let len = Self::decode_u32(mb.args[2])?;
                let key_strs = self.read_symbol_keys(keys_pos, len)?;
                let key_refs: Vec<&str> = key_strs.iter().map(|s| s.as_str()).collect();
                let vals = self.read_vals(vals_pos, len)?;
                Some(
                    self.host
                        .map_new_from_slices(&key_refs, &vals)
                        .map_err(|e| format!("map_new_from_slices: {e:?}"))?
                        .into(),
                )
            }
            "map_unpack_to_linear_memory" => {
                let map: MapObject = self.decode_val(mb.args[0])?.try_into().map_err(|_| {
                    format!("map_unpack: arg0 not MapObject")
                })?;
                let keys_pos = Self::decode_u32(mb.args[1])?;
                let vals_pos = Self::decode_u32(mb.args[2])?;
                let len = Self::decode_u32(mb.args[3])?;
                let key_strs = self.read_symbol_keys(keys_pos, len)?;
                let key_refs: Vec<&str> = key_strs.iter().map(|s| s.as_str()).collect();
                let mut vals = vec![Val::VOID.into(); len as usize];
                self.host
                    .map_unpack_to_slice(map, &key_refs, &mut vals)
                    .map_err(|e| format!("map_unpack_to_slice: {e:?}"))?;
                self.write_vals(vals_pos, &vals)?;
                return self.finish(mb, None);
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
            "extend_contract_data_ttl" => {
                let k = self.decode_val(mb.args[0])?;
                let t = Self::decode_storage_type(mb.args[1])?;
                let threshold: U32Val = Self::decode_u32(mb.args[2])?.into();
                let extend_to: U32Val = Self::decode_u32(mb.args[3])?.into();
                self.host
                    .extend_contract_data_ttl(k, t, threshold, extend_to)
                    .map_err(|e| format!("extend_contract_data_ttl: {e:?}"))?;
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
            "contract_event" => {
                let topics: VecObject = self.decode_val(mb.args[0])?.try_into().map_err(|_| {
                    format!("contract_event: topics not VecObject")
                })?;
                let data = self.decode_val(mb.args[1])?;
                self.host
                    .contract_event(topics, data)
                    .map_err(|e| format!("contract_event: {e:?}"))?;
                None
            }
            "get_ledger_sequence" => Some(
                self.host
                    .get_ledger_sequence()
                    .map_err(|e| format!("get_ledger_sequence: {e:?}"))?
                    .into(),
            ),
            "obj_from_i128_pieces" => {
                let hi = mb.args[0] as i64;
                let lo = mb.args[1];
                Some(
                    self.host
                        .obj_from_i128_pieces(hi, lo)
                        .map_err(|e| format!("obj_from_i128_pieces: {e:?}"))?
                        .into(),
                )
            }
            "obj_to_i128_lo64" => {
                let obj = self.decode_val(mb.args[0])?;
                let lo = self
                    .host
                    .obj_to_i128_lo64(obj.try_into().map_err(|_| "not I128Object")?)
                    .map_err(|e| format!("obj_to_i128_lo64: {e:?}"))?;
                self.finish_raw(mb, lo);
                self.record_trace(mb, name);
                return Ok(());
            }
            "obj_to_i128_hi64" => {
                let obj = self.decode_val(mb.args[0])?;
                let hi = self
                    .host
                    .obj_to_i128_hi64(obj.try_into().map_err(|_| "not I128Object")?)
                    .map_err(|e| format!("obj_to_i128_hi64: {e:?}"))?;
                self.finish_raw(mb, hi as u64);
                self.record_trace(mb, name);
                return Ok(());
            }
            "require_auth" => {
                let addr: AddressObject = self
                    .decode_val(mb.args[0])?
                    .try_into()
                    .map_err(|_| "require_auth: not AddressObject".to_string())?;
                self.host
                    .require_auth(addr)
                    .map_err(|e| format!("require_auth: {e:?}"))?;
                None
            }
            other => {
                let key = env_ids::import_key(mb.fn_id).unwrap_or("?");
                return Err(format!("unimplemented host function {key} ({other})"));
            }
        };
        let r = self.finish(mb, res);
        if r.is_ok() {
            self.record_trace(mb, name);
        }
        r
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
