use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
pub mod guest;

#[derive(Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Request {
    Settings,
    HttpStart {
        request: HttpRequest,
    },
    HttpRead {
        length: usize,
    },
    HttpClose,
    Chat,
    Speech,
    List {
        path: String,
    },
    Read {
        path: String,
        offset: u64,
        length: usize,
        #[serde(default)]
        optional: bool,
    },
    Write {
        path: String,
        replace: bool,
    },
    Rename {
        from: String,
        to: String,
    },
    Remove {
        path: String,
    },
}

#[derive(Deserialize, Serialize)]
pub struct HttpRequest {
    pub path: Vec<String>,
    pub get: bool,
    pub query: Vec<(String, String)>,
    pub accept: Option<String>,
    pub timeout_ms: u64,
    pub body: HttpBody,
}

#[derive(Deserialize, Serialize)]
pub enum HttpBody {
    Empty,
    Json,
    Wave { fields: Vec<(String, String)> },
}

#[derive(Deserialize, Serialize)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: String,
}
