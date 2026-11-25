use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
pub mod guest;

#[derive(Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", try_from = "RequestWire")]
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

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Settings,
    HttpStart,
    HttpRead,
    HttpClose,
    Chat,
    Speech,
    List,
    Read,
    Write,
    Rename,
    Remove,
}

#[derive(Deserialize)]
struct RequestWire {
    operation: Operation,
    request: Option<HttpRequest>,
    length: Option<usize>,
    path: Option<String>,
    offset: Option<u64>,
    #[serde(default)]
    optional: bool,
    replace: Option<bool>,
    from: Option<String>,
    to: Option<String>,
}

impl TryFrom<RequestWire> for Request {
    type Error = &'static str;

    fn try_from(wire: RequestWire) -> Result<Self, Self::Error> {
        Ok(match wire.operation {
            Operation::Settings => Self::Settings,
            Operation::HttpStart => Self::HttpStart {
                request: wire.request.ok_or("Missing HTTP request")?,
            },
            Operation::HttpRead => Self::HttpRead {
                length: wire.length.ok_or("Missing HTTP read length")?,
            },
            Operation::HttpClose => Self::HttpClose,
            Operation::Chat => Self::Chat,
            Operation::Speech => Self::Speech,
            Operation::List => Self::List {
                path: wire.path.ok_or("Missing storage path")?,
            },
            Operation::Read => Self::Read {
                path: wire.path.ok_or("Missing storage path")?,
                offset: wire.offset.ok_or("Missing storage offset")?,
                length: wire.length.ok_or("Missing storage read length")?,
                optional: wire.optional,
            },
            Operation::Write => Self::Write {
                path: wire.path.ok_or("Missing storage path")?,
                replace: wire.replace.ok_or("Missing storage replace flag")?,
            },
            Operation::Rename => Self::Rename {
                from: wire.from.ok_or("Missing source path")?,
                to: wire.to.ok_or("Missing destination path")?,
            },
            Operation::Remove => Self::Remove {
                path: wire.path.ok_or("Missing storage path")?,
            },
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_codec;

    #[test]
    fn reads_the_existing_storage_wire_format_with_operation_last() {
        let wire = br#"{"length":85,"offset":18446744073709551615,"path":"state.json","operation":"read"}"#;
        let request: Request = json_codec::from_slice(wire).unwrap();
        assert!(matches!(
            &request,
            Request::Read { path, offset: u64::MAX, length: 85, optional: false }
                if path == "state.json"
        ));
        assert!(matches!(
            json_codec::from_slice::<Request>(&json_codec::to_vec(&request).unwrap()).unwrap(),
            Request::Read { .. }
        ));
    }

    #[test]
    fn reads_existing_http_requests_with_externally_tagged_bodies() {
        let wire = br#"{"operation":"http_start","request":{"path":["audio","transcriptions"],"get":false,"query":[],"accept":null,"timeout_ms":2000,"body":{"Wave":{"fields":[["model","speech"]]}}}}"#;
        let Request::HttpStart { request } = json_codec::from_slice(wire).unwrap() else {
            panic!("expected HTTP request");
        };
        assert!(matches!(request.body, HttpBody::Wave { fields } if fields[0].0 == "model"));
    }
}
