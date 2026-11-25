use std::{
    ffi::{CStr, c_char, c_void},
    fs::File,
    io::Read,
    path::Path,
    ptr::{self, NonNull},
    slice,
    sync::Mutex,
};

use crate::plugin_wasm::{
    FRAME_LIMIT, HostServices, INPUT_LIMIT, MAX_SIGNALS, META_LIMIT, MODULE_LIMIT,
};
use goodwin_error::{Context, Result, ensure};

const PAGE: usize = 64 * 1024;
const INSTRUCTION_LIMIT: i32 = 32_000_000;
const HOST_INSTRUCTION_LIMIT: i32 = 1_000_000_000;
const BUFFER_INSTRUCTION_LIMIT: i32 = 1_000_000;
type Handle = *mut c_void;

unsafe extern "C" {
    fn goodwin_wamr_init() -> bool;
    fn goodwin_wamr_load(data: *mut u8, size: u32, err: *mut c_char, cap: u32) -> Handle;
    fn goodwin_wamr_instantiate(
        module: Handle,
        stack: u32,
        pages: u32,
        err: *mut c_char,
        cap: u32,
    ) -> Handle;
    fn goodwin_wamr_lookup(instance: Handle, name: *const c_char, params: u32) -> Handle;
    fn wasm_runtime_is_underlying_binary_freeable(module: Handle) -> bool;
    fn wasm_runtime_unload(module: Handle);
    fn wasm_runtime_deinstantiate(instance: Handle);
    fn wasm_runtime_create_exec_env(instance: Handle, stack: u32) -> Handle;
    fn wasm_runtime_destroy_exec_env(env: Handle);
    fn wasm_runtime_set_user_data(env: Handle, data: Handle);
    fn wasm_runtime_get_user_data(env: Handle) -> Handle;
    fn wasm_runtime_get_module_inst(env: Handle) -> Handle;
    fn wasm_runtime_clear_exception(instance: Handle);
    fn wasm_runtime_get_exception(instance: Handle) -> *const c_char;
    fn wasm_runtime_set_instruction_count_limit(env: Handle, remaining: i32);
    fn wasm_runtime_call_wasm(env: Handle, function: Handle, count: u32, args: *mut u32) -> bool;
    fn wasm_runtime_validate_app_addr(instance: Handle, offset: u64, size: u64) -> bool;
    fn wasm_runtime_addr_app_to_native(instance: Handle, offset: u64) -> Handle;
    #[cfg(test)]
    fn wasm_runtime_get_default_memory(instance: Handle) -> Handle;
    #[cfg(test)]
    fn wasm_memory_get_cur_page_count(memory: Handle) -> u64;
    #[cfg(test)]
    fn wasm_memory_get_bytes_per_page(memory: Handle) -> u64;
}

static RUNTIME: Mutex<bool> = Mutex::new(false);

struct Module {
    raw: NonNull<c_void>,
    _source: Option<Vec<u8>>,
}

impl Drop for Module {
    fn drop(&mut self) {
        // The source bytes outlive unload when the interpreter needs them.
        unsafe { wasm_runtime_unload(self.raw.as_ptr()) };
    }
}

struct Instance {
    raw: NonNull<c_void>,
    _module: Module,
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe { wasm_runtime_deinstantiate(self.raw.as_ptr()) };
    }
}

pub(crate) struct Guest<H> {
    instance: Instance,
    env: NonNull<c_void>,
    host: Option<H>,
    input: Option<NonNull<c_void>>,
    buffer: NonNull<c_void>,
    dispatch: NonNull<c_void>,
}

// SAFETY: Both platforms serialize calls to each guest. WAMR's native
// callbacks are synchronous and the interpreter is built without Wasm threads.
unsafe impl<H: Send> Send for Guest<H> {}

impl<H> Drop for Guest<H> {
    fn drop(&mut self) {
        unsafe { wasm_runtime_destroy_exec_env(self.env.as_ptr()) };
    }
}

struct ActiveCall<'a> {
    host: Option<&'a mut dyn HostServices>,
    output: Vec<u8>,
    signals: u32,
}

impl<H: HostServices> Guest<H> {
    pub(crate) fn open(path: &Path, host: Option<H>, memory_limit: usize) -> Result<Self> {
        let mut source = Vec::new();
        File::open(path)?
            .take((MODULE_LIMIT + 1) as u64)
            .read_to_end(&mut source)?;
        Self::load_owned(source, host, memory_limit)
    }

    #[cfg(test)]
    pub(crate) fn load(source: &[u8], host: Option<H>, memory_limit: usize) -> Result<Self> {
        Self::load_owned(source.to_vec(), host, memory_limit)
    }

    fn load_owned(mut source: Vec<u8>, host: Option<H>, memory_limit: usize) -> Result<Self> {
        ensure!(source.len() <= MODULE_LIMIT, "WASM module is too large");
        let pages = u32::try_from(memory_limit / PAGE)?;
        ensure!(pages > 0, "WASM memory limit is smaller than a page");
        let pages = super::validation::module(&source, host.is_some(), pages)?;
        init_runtime()?;
        let mut error = [0 as c_char; 256];
        let raw = unsafe {
            goodwin_wamr_load(
                source.as_mut_ptr(),
                source.len() as u32,
                error.as_mut_ptr(),
                error.len() as u32,
            )
        };
        let raw = NonNull::new(raw).with_context(|| message(&error))?;
        let keep_source = !unsafe { wasm_runtime_is_underlying_binary_freeable(raw.as_ptr()) };
        let module = Module {
            raw,
            _source: keep_source.then_some(source),
        };
        let stack = if host.is_some() { 32 * 1024 } else { 8 * 1024 };
        let raw = unsafe {
            goodwin_wamr_instantiate(
                module.raw.as_ptr(),
                stack,
                pages,
                error.as_mut_ptr(),
                error.len() as u32,
            )
        };
        let raw = NonNull::new(raw).with_context(|| message(&error))?;
        let instance = Instance {
            raw,
            _module: module,
        };
        let lookup = |name: &CStr, params| {
            let function =
                unsafe { goodwin_wamr_lookup(instance.raw.as_ptr(), name.as_ptr(), params) };
            NonNull::new(function)
                .with_context(|| format!("Missing or invalid WASM export {name:?}"))
        };
        let input = host
            .as_ref()
            .map(|_| lookup(c"goodwin_input", 1))
            .transpose()?;
        let buffer = lookup(c"goodwin_buffer", 0)?;
        let dispatch = lookup(c"goodwin_call", 1)?;
        let env =
            NonNull::new(unsafe { wasm_runtime_create_exec_env(instance.raw.as_ptr(), stack) })
                .context("Allocate WAMR execution environment")?;
        Ok(Self {
            instance,
            env,
            host,
            input,
            buffer,
            dispatch,
        })
    }

    pub(crate) fn host_mut(&mut self) -> Option<&mut H> {
        self.host.as_mut()
    }

    #[cfg(not(target_os = "espidf"))]
    pub(crate) fn call_as<T: serde::de::DeserializeOwned>(
        &mut self,
        frame: &[u8],
        response_limit: usize,
    ) -> Result<(T, u32)> {
        self.exchange_as(frame, response_limit, self.instruction_limit())
    }

    fn instruction_limit(&self) -> i32 {
        if self.host.is_some() {
            HOST_INSTRUCTION_LIMIT
        } else {
            INSTRUCTION_LIMIT
        }
    }

    #[cfg(target_os = "espidf")]
    pub(crate) fn call_with(
        &mut self,
        frame: &[u8],
        response_limit: usize,
        decode: &mut dyn FnMut(&[u8]) -> Result<()>,
    ) -> Result<u32> {
        self.exchange_with(frame, response_limit, self.instruction_limit(), decode)
    }

    #[cfg(test)]
    pub(crate) fn memory_size(&self) -> usize {
        // SAFETY: Module admission requires one linear memory, including private memories.
        let memory = unsafe { wasm_runtime_get_default_memory(self.instance.raw.as_ptr()) };
        // WAMR can combine fixed-size memories into one larger internal page.
        unsafe {
            (wasm_memory_get_cur_page_count(memory) * wasm_memory_get_bytes_per_page(memory))
                as usize
        }
    }

    #[cfg(all(test, not(target_os = "espidf")))]
    pub(crate) fn exchange(
        &mut self,
        frame: &[u8],
        response_limit: usize,
        limit: i32,
    ) -> Result<super::Reply> {
        self.exchange_as(frame, response_limit, limit)
    }

    #[cfg(not(target_os = "espidf"))]
    fn exchange_as<T: serde::de::DeserializeOwned>(
        &mut self,
        frame: &[u8],
        response_limit: usize,
        limit: i32,
    ) -> Result<(T, u32)> {
        let mut reply = None;
        let signals = self.exchange_with(frame, response_limit, limit, &mut |bytes| {
            reply = Some(serde_json::from_slice(bytes).context("Decode WASM RPC")?);
            Ok(())
        })?;
        Ok((reply.context("Missing WASM RPC response")?, signals))
    }

    #[inline(never)]
    fn exchange_with(
        &mut self,
        frame: &[u8],
        response_limit: usize,
        limit: i32,
        decode: &mut dyn FnMut(&[u8]) -> Result<()>,
    ) -> Result<u32> {
        ensure!(
            frame.len()
                <= if self.input.is_some() {
                    INPUT_LIMIT
                } else {
                    FRAME_LIMIT
                },
            "WASM plugin request is too large"
        );
        let mut state = ActiveCall {
            host: self.host.as_mut().map(|host| host as &mut dyn HostServices),
            output: Vec::new(),
            signals: 0,
        };
        let instance = self.instance.raw.as_ptr();
        let env = self.env.as_ptr();
        // WAMR meters each export call separately. Reserve a bounded portion of
        // the exchange budget for accessing the guest's RPC buffers.
        let auxiliary = (limit / 4).min(BUFFER_INSTRUCTION_LIMIT);
        ensure!(auxiliary > 0, "WASM instruction budget is too small");
        let dispatch_limit = limit - auxiliary * if self.input.is_some() { 2 } else { 1 };
        // The environment is exclusively borrowed. Clear the transient callback pointer on return.
        unsafe { wasm_runtime_set_user_data(env, (&mut state as *mut ActiveCall<'_>).cast()) };
        let result = (|| {
            let invoke = |function: NonNull<c_void>, param: Option<u32>, budget: i32| {
                let mut word = param.unwrap_or(0);
                let ok = unsafe {
                    wasm_runtime_clear_exception(instance);
                    wasm_runtime_set_instruction_count_limit(env, budget);
                    wasm_runtime_call_wasm(
                        env,
                        function.as_ptr(),
                        u32::from(param.is_some()),
                        &mut word,
                    )
                };
                if !ok {
                    let error = unsafe { wasm_runtime_get_exception(instance) };
                    let description = if error.is_null() {
                        "unknown error".into()
                    } else {
                        unsafe { CStr::from_ptr(error) }.to_string_lossy()
                    };
                    goodwin_error::bail!("WAMR call: {description}");
                }
                Ok(word)
            };
            let offset = match self.input {
                Some(input) => invoke(input, Some(frame.len() as u32), auxiliary)?,
                None => invoke(self.buffer, None, auxiliary)?,
            };
            let target = guest_pointer(instance, offset, frame.len())?;
            // The pointers have separately validated ranges in host and guest memory.
            unsafe { ptr::copy_nonoverlapping(frame.as_ptr(), target.as_ptr(), frame.len()) };
            let length = invoke(self.dispatch, Some(frame.len() as u32), dispatch_limit)? as i32;
            let length = usize::try_from(length).context("Invalid WASM response size")?;
            ensure!(
                length <= response_limit && (self.input.is_some() || length <= FRAME_LIMIT),
                "WASM plugin response is too large"
            );
            let offset = if self.input.is_some() {
                invoke(self.buffer, None, auxiliary)?
            } else {
                offset
            };
            let response = guest_pointer(instance, offset, length)?;
            // The validated guest response remains stable while it is decoded.
            decode(unsafe { slice::from_raw_parts(response.as_ptr(), length) })
        })();
        unsafe { wasm_runtime_set_user_data(env, ptr::null_mut()) };
        result?;
        Ok(state.signals)
    }
}

fn init_runtime() -> Result<()> {
    let mut ready = RUNTIME.lock().expect("WAMR initialization lock");
    if !*ready {
        // SAFETY: The lock serializes initialization and the native imports
        // and allocator stay available throughout the process lifetime.
        ensure!(unsafe { goodwin_wamr_init() }, "Initialize WAMR");
        *ready = true;
    }
    Ok(())
}

fn message(error: &[c_char]) -> String {
    // WAMR writes a nul-terminated error into this initialized buffer.
    unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn guest_pointer(instance: Handle, offset: u32, size: usize) -> Result<NonNull<u8>> {
    if size == 0 {
        return Ok(NonNull::dangling());
    }
    ensure!(
        unsafe { wasm_runtime_validate_app_addr(instance, u64::from(offset), size as u64) },
        "WASM host buffer is out of bounds"
    );
    NonNull::new(unsafe { wasm_runtime_addr_app_to_native(instance, u64::from(offset)) }.cast())
        .context("Missing WASM memory")
}

#[unsafe(no_mangle)]
extern "C" fn goodwin_wamr_host_call(
    env: Handle,
    meta: u32,
    meta_len: u32,
    input: u32,
    input_len: u32,
) -> i32 {
    let data = unsafe {
        wasm_runtime_get_user_data(env)
            .cast::<ActiveCall<'_>>()
            .as_mut()
    };
    let Some(data) = data else { return -1 };
    data.output.clear();
    let result = (|| {
        let host = data.host.as_mut().context("WASM host unavailable")?;
        ensure!(
            meta_len as usize <= META_LIMIT && input_len as usize <= INPUT_LIMIT,
            "WASM host operation is too large"
        );
        let instance = unsafe { wasm_runtime_get_module_inst(env) };
        let args = guest_pointer(instance, meta, meta_len as usize)?;
        let request = goodwin_json::from_slice(unsafe {
            slice::from_raw_parts(args.as_ptr(), meta_len as usize)
        })?;
        let input = guest_pointer(instance, input, input_len as usize)?;
        host.execute(request, unsafe {
            slice::from_raw_parts(input.as_ptr(), input_len as usize)
        })
    })();
    match result {
        Ok(output) => {
            data.output = output;
            i32::try_from(data.output.len()).unwrap_or(-1)
        }
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
extern "C" fn goodwin_wamr_host_result(env: Handle, offset: u32, size: u32) -> i32 {
    let data = unsafe {
        wasm_runtime_get_user_data(env)
            .cast::<ActiveCall<'_>>()
            .as_mut()
    };
    let Some(data) = data else { return -1 };
    let output = std::mem::take(&mut data.output);
    if output.len() != size as usize {
        return -1;
    }
    let instance = unsafe { wasm_runtime_get_module_inst(env) };
    let Ok(target) = guest_pointer(instance, offset, size as usize) else {
        return -1;
    };
    unsafe { ptr::copy_nonoverlapping(output.as_ptr(), target.as_ptr(), size as usize) };
    0
}

#[unsafe(no_mangle)]
extern "C" fn goodwin_wamr_signal(env: Handle) {
    if let Some(data) = unsafe {
        wasm_runtime_get_user_data(env)
            .cast::<ActiveCall<'_>>()
            .as_mut()
    } {
        data.signals = (data.signals + 1).min(MAX_SIGNALS);
    }
}
