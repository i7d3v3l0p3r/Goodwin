use std::{
    ffi::{CStr, CString},
    io::{self, Read},
    ptr::{self, NonNull},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use esp_idf_svc::{handle::RawHandle, http::server::EspHttpServer, sys};
use goodwin_error::{Result, ensure};

use super::{Panel, State, firmware, jobs::Problem, status_reason};
use crate::panel_core::{CONTENT_SECURITY_POLICY, same_origin, session_token};

const UPLOAD_TIMEOUT: Duration = Duration::from_secs(180);

pub(in crate::platform::espidf) struct Server {
    pub(super) http: EspHttpServer<'static>,
    uploads: Box<Uploads>,
}

impl Server {
    pub(super) fn new(http: EspHttpServer<'static>, panel: Arc<Panel>) -> Result<Self> {
        let server = Self {
            http,
            uploads: Box::new(Uploads {
                panel,
                closing: Arc::new(AtomicBool::new(false)),
                worker: Arc::new(Mutex::new(None)),
            }),
        };
        let user_ctx = ptr::from_ref(server.uploads.as_ref()).cast_mut().cast();
        let register = |uri: &CStr, handler: unsafe extern "C" fn(*mut sys::httpd_req_t) -> i32| {
            let route = sys::httpd_uri_t {
                uri: uri.as_ptr(),
                method: sys::http_method_HTTP_POST,
                handler: Some(handler),
                user_ctx,
                ..Default::default()
            };
            // SAFETY: ESP-IDF copies the descriptor. The boxed context is stable
            // and outlives the server, including callbacks during shutdown.
            let code = unsafe { sys::httpd_register_uri_handler(server.http.handle(), &route) };
            ensure!(code == sys::ESP_OK, "Register upload route {uri:?}: {code}");
            Ok::<_, goodwin_error::Error>(())
        };
        register(c"/api/firmware", receive_firmware)?;
        #[cfg(feature = "wasm-plugins")]
        register(c"/api/plugin-packages", receive_package)?;
        Ok(server)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let worker = {
            let mut worker = self.uploads.worker.lock().expect("upload worker");
            self.uploads.closing.store(true, Ordering::Release);
            worker.take()
        };
        // The server and socket database must outlive all asynchronous requests.
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

struct Uploads {
    panel: Arc<Panel>,
    closing: Arc<AtomicBool>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
}

#[derive(Clone, Copy)]
enum Route {
    Firmware,
    #[cfg(feature = "wasm-plugins")]
    Package,
}

extern "C" fn receive_firmware(req: *mut sys::httpd_req_t) -> i32 {
    receive(req, Route::Firmware)
}

#[cfg(feature = "wasm-plugins")]
extern "C" fn receive_package(req: *mut sys::httpd_req_t) -> i32 {
    receive(req, Route::Package)
}

fn receive(req: *mut sys::httpd_req_t, route: Route) -> i32 {
    let Some(req) = NonNull::new(req) else {
        return sys::ESP_FAIL;
    };
    // SAFETY: These callbacks are registered only with our boxed Uploads as
    // their context, and the original request is valid throughout the callback.
    let uploads = unsafe { &*req.as_ref().user_ctx.cast::<Uploads>() };
    match uploads.start(req, route) {
        Ok(()) => sys::ESP_OK,
        Err((status, code)) => {
            send_problem(req, status, code, true);
            // Returning failure closes the connection without making HTTPD
            // drain a rejected request's body on its own task.
            sys::ESP_FAIL
        }
    }
}

impl Uploads {
    fn start(&self, req: NonNull<sys::httpd_req_t>, route: Route) -> Result<(), Problem> {
        let origin = header(req, c"Origin")?;
        let host = header(req, c"Host")?;
        let site = header(req, c"Sec-Fetch-Site")?;
        if !same_origin(origin.as_deref(), host.as_deref(), site.as_deref()) {
            return Err((403, "origin"));
        }
        let cookie = header(req, c"Cookie")?;
        let access = self.panel.access(cookie.as_deref().and_then(session_token));
        if !access.authorized {
            return Err((401, "login_required"));
        }
        if !access.protected {
            let code = match route {
                Route::Firmware => "core_update_password_required",
                #[cfg(feature = "wasm-plugins")]
                Route::Package => "plugin_password_required",
            };
            return Err((403, code));
        }

        // SAFETY: The parser has already validated Content-Length. It belongs
        // to the original request, which is still exclusively on HTTPD's task.
        let size = unsafe { req.as_ref().content_len };
        let mut worker = self.worker.lock().expect("upload worker");
        if self.closing.load(Ordering::Acquire) || worker.is_some() {
            return Err((409, "busy"));
        }
        let job = match route {
            Route::Firmware => {
                if header(req, c"Content-Type")?.as_deref() != Some("application/octet-stream")
                    || !(4096..=firmware::capacity()).contains(&size)
                {
                    return Err((400, "invalid_firmware"));
                }
                Job::Firmware(FirmwareUpload::reserve(&self.panel)?)
            }
            #[cfg(feature = "wasm-plugins")]
            Route::Package => {
                use crate::platform::espidf::plugins::ARCHIVE_LIMIT;

                if size > ARCHIVE_LIMIT {
                    return Err((413, "plugin_archive_too_large"));
                }
                let plugins = self.panel.plugins.get().ok_or((404, "not_found"))?;
                Job::Package(Arc::clone(plugins))
            }
        };
        let stack = match route {
            Route::Firmware => 16 * 1024,
            #[cfg(feature = "wasm-plugins")]
            Route::Package => 28 * 1024,
        };
        let (send, receive) = mpsc::sync_channel(1);
        let active = Arc::clone(&self.worker);
        let task = thread::Builder::new()
            .name("goodwin-upload".into())
            .stack_size(stack)
            .spawn(move || {
                if let Ok(upload) = receive.recv() {
                    job.run(upload);
                    #[cfg(feature = "esp-emulator")]
                    {
                        // SAFETY: Null asks FreeRTOS for the current task's watermark.
                        let free = unsafe { sys::uxTaskGetStackHighWaterMark(ptr::null_mut()) };
                        log::info!("Upload task stack headroom: {free} bytes");
                    }
                }
                // ESP-IDF retains the stack of a joinable thread after it
                // exits. Detach ourselves once the async request is gone.
                // If server shutdown already took the handle, it joins us.
                let mut worker = active.lock().expect("upload worker");
                drop(worker.take());
            })
            .map_err(|error| {
                // SAFETY: Read-only heap telemetry for task-capable internal memory.
                let largest = unsafe {
                    sys::heap_caps_get_largest_free_block(sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT)
                };
                log::warn!("Start {stack}-byte upload task: {error}; largest internal block: {largest} bytes");
                (503, "busy")
            })?;
        *worker = Some(task);
        let upload = Upload::begin(req, size, Arc::clone(&self.closing))?;
        if let Err(error) = send.send(upload) {
            // The original request has been handed off. Complete the copy on
            // this exceptional path instead of responding on the original.
            let mut upload = error.0;
            upload.problem(503, "busy");
        }
        Ok(())
    }
}

fn header(req: NonNull<sys::httpd_req_t>, name: &CStr) -> Result<Option<String>, Problem> {
    // SAFETY: The original request is live on the HTTP task and no response
    // has been sent. The string name is NUL-terminated.
    let len = unsafe { sys::httpd_req_get_hdr_value_len(req.as_ptr(), name.as_ptr()) };
    if len == 0 {
        return Ok(None);
    }
    let mut bytes = vec![0; len + 1];
    // SAFETY: ESP-IDF writes at most the provided buffer size, including NUL.
    let code = unsafe {
        sys::httpd_req_get_hdr_value_str(
            req.as_ptr(),
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if code != sys::ESP_OK {
        return Err((400, "invalid_request"));
    }
    bytes.pop();
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| (400, "invalid_request"))
}

struct Upload {
    req: NonNull<sys::httpd_req_t>,
    remaining: usize,
    deadline: Instant,
    closing: Arc<AtomicBool>,
    close_socket: bool,
}

// SAFETY: httpd_req_async_handler_begin creates an owned request copy and
// reserves the socket for that copy. Only this owner accesses it. Server::drop
// joins its worker before stopping HTTPD and freeing the socket database.
unsafe impl Send for Upload {}

impl Upload {
    fn begin(
        original: NonNull<sys::httpd_req_t>,
        remaining: usize,
        closing: Arc<AtomicBool>,
    ) -> Result<Self, Problem> {
        let mut copy = ptr::null_mut();
        // SAFETY: Called only inside the original request's callback. ESP-IDF
        // hands off the unread body to the copy on success.
        let code = unsafe { sys::httpd_req_async_handler_begin(original.as_ptr(), &mut copy) };
        if code != sys::ESP_OK {
            return Err((503, "busy"));
        }
        Ok(Self {
            req: NonNull::new(copy).expect("ESP-IDF async request"),
            remaining,
            deadline: Instant::now() + UPLOAD_TIMEOUT,
            closing,
            close_socket: false,
        })
    }

    fn problem(&mut self, status: u16, code: &str) {
        self.close_socket |= !send_problem(self.req, status, code, self.remaining != 0);
    }

    fn respond(&mut self, status: u16, mime: &CStr, body: &[u8]) {
        self.close_socket |= !respond(self.req, status, mime, body, self.remaining != 0);
    }
}

impl Read for Upload {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = self.remaining.min(buf.len());
        if len == 0 {
            return Ok(0);
        }
        if self.closing.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        // SAFETY: The async copy owns the socket. The mutable buffer is valid
        // for len bytes and ESP-IDF does not retain its address.
        let read = unsafe { sys::httpd_req_recv(self.req.as_ptr(), buf.as_mut_ptr().cast(), len) };
        match read {
            0 => Err(io::ErrorKind::UnexpectedEof.into()),
            n if n < 0 => Err(io::Error::other(format!("Read upload socket: {n}"))),
            n => {
                self.remaining -= n as usize;
                Ok(n as usize)
            }
        }
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        // SAFETY: We still own the async copy and its socket. Shut down the
        // socket before releasing ownership if its unread body cannot be used
        // for a subsequent request. Only HTTPD closes the descriptor itself.
        unsafe {
            if self.remaining != 0 || self.close_socket {
                let fd = sys::httpd_req_to_sockfd(self.req.as_ptr());
                if fd >= 0 {
                    sys::lwip_shutdown(fd, sys::SHUT_RDWR as i32);
                }
            }
            let _ = sys::httpd_req_async_handler_complete(self.req.as_ptr());
        }
    }
}

fn send_problem(req: NonNull<sys::httpd_req_t>, status: u16, code: &str, close: bool) -> bool {
    let body = goodwin_json::to_vec(&serde_json::json!({"error": code})).unwrap_or_default();
    respond(req, status, c"application/json", &body, close)
}

fn respond(
    req: NonNull<sys::httpd_req_t>,
    status: u16,
    mime: &CStr,
    body: &[u8],
    close: bool,
) -> bool {
    let status = CString::new(format!("{status} {}", status_reason(status))).expect("HTTP status");
    let policy = CString::new(CONTENT_SECURITY_POLICY).expect("content security policy");
    let req = req.as_ptr();
    // SAFETY: The request is still live on its owning task. HTTPD borrows
    // these NUL-terminated headers until httpd_resp_send, before they drop.
    unsafe {
        sys::httpd_resp_set_status(req, status.as_ptr()) == sys::ESP_OK
            && sys::httpd_resp_set_type(req, mime.as_ptr()) == sys::ESP_OK
            && sys::httpd_resp_set_hdr(req, c"Cache-Control".as_ptr(), c"no-store".as_ptr())
                == sys::ESP_OK
            && sys::httpd_resp_set_hdr(req, c"Content-Security-Policy".as_ptr(), policy.as_ptr())
                == sys::ESP_OK
            && sys::httpd_resp_set_hdr(req, c"X-Content-Type-Options".as_ptr(), c"nosniff".as_ptr())
                == sys::ESP_OK
            && (!close
                || sys::httpd_resp_set_hdr(req, c"Connection".as_ptr(), c"close".as_ptr())
                    == sys::ESP_OK)
            && sys::httpd_resp_send(req, body.as_ptr().cast(), body.len() as isize) == sys::ESP_OK
    }
}

enum Job {
    Firmware(FirmwareUpload),
    #[cfg(feature = "wasm-plugins")]
    Package(Arc<crate::platform::espidf::plugins::Plugins>),
}

impl Job {
    fn run(self, mut body: Upload) {
        match self {
            Self::Firmware(mut update) => {
                let size = body.remaining;
                match firmware::install(|block| Ok(body.read(block)?), size) {
                    Ok(true) => {
                        update.staged = true;
                        body.respond(202, c"application/json", br#"{"restarting":true}"#);
                        drop(body);
                        update.panel.reboot.store(true, Ordering::Release);
                    }
                    Ok(false) => body.problem(400, "invalid_firmware"),
                    Err(error) => {
                        log::warn!("Install firmware: {error:#}");
                        body.problem(500, "firmware_update_failed");
                    }
                }
            }
            #[cfg(feature = "wasm-plugins")]
            Self::Package(plugins) => {
                match super::super::memory::external(|| plugins.install(&mut body)) {
                    Ok(()) => body.respond(204, c"text/plain", &[]),
                    Err(error) => body.problem(error.status(), error.code()),
                }
            }
        }
    }
}

struct FirmwareUpload {
    panel: Arc<Panel>,
    staged: bool,
}

impl FirmwareUpload {
    fn reserve(panel: &Arc<Panel>) -> Result<Self, Problem> {
        let state = State::lock(&panel.state);
        if state.busy()
            || panel.wake_model_busy()
            || !panel.wifi_connected()
            || panel.firmware_busy.swap(true, Ordering::AcqRel)
        {
            return Err((409, "busy"));
        }
        Ok(Self {
            panel: Arc::clone(panel),
            staged: false,
        })
    }
}

impl Drop for FirmwareUpload {
    fn drop(&mut self) {
        if !self.staged {
            self.panel.firmware_busy.store(false, Ordering::Release);
        }
    }
}
