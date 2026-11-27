use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transport {
    Tcp,
    WebSocket,
    HttpUpgrade,
    Http2,
    Grpc,
    Xhttp,
}

#[derive(Serialize, Deserialize)]
pub struct Reality {
    pub key: [u8; 32],
    pub short_id: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub struct Profile {
    pub server: String,
    pub port: u16,
    pub uuid: String,
    pub tls_name: Option<String>,
    pub fingerprint: Option<String>,
    pub alpn: Vec<String>,
    pub reality: Option<Reality>,
    pub vision: bool,
    pub encryption: Option<String>,
    pub transport: Transport,
    pub authority: String,
    pub path: String,
}

impl Profile {
    pub fn worker(&self) -> &'static str {
        if self.reality.is_some()
            || self.vision
            || self.fingerprint.is_some()
            || self.encryption.is_some()
        {
            "vless-xtls"
        } else if matches!(
            self.transport,
            Transport::Http2 | Transport::Grpc | Transport::Xhttp
        ) {
            "vless-http2"
        } else {
            "vless-stream"
        }
    }
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(all(test, not(target_os = "linux")), allow(dead_code))]
pub struct Launch {
    pub profile: Profile,
    pub password: String,
}
