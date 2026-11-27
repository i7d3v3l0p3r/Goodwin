use std::{fs::Permissions, net::Ipv4Addr, os::unix::fs::PermissionsExt, path::PathBuf};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use goodwin_error::Result;
use tokio::{
    net::{TcpListener, UnixStream},
    task::JoinSet,
};

/// Keep the consumer-facing port reserved for this host's lifetime. A stopped
/// or replaced VPN can never turn an assigned client back into a direct client.
pub(super) struct Gateway {
    directory: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
    pub(super) url: String,
    pub(super) password: String,
}

impl Gateway {
    pub(super) async fn open() -> Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("goodwin-vpn-")
            .permissions(Permissions::from_mode(0o700))
            .tempdir()?;
        let socket = directory.path().join("proxy");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let password = URL_SAFE_NO_PAD.encode(crate::panel_core::random_bytes::<24>()?);
        let url = format!("socks5h://goodwin:{password}@{address}");
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    Some(_) = connections.join_next() => {}
                    incoming = listener.accept(), if connections.len() < 64 => {
                        let Ok((mut client, _)) = incoming else { break };
                        let socket = socket.clone();
                        connections.spawn(async move {
                            if let Ok(mut vpn) = UnixStream::connect(socket).await {
                                let _ = tokio::io::copy_bidirectional(&mut client, &mut vpn).await;
                            }
                        });
                    }
                }
            }
        });
        Ok(Self {
            directory,
            task,
            url,
            password,
        })
    }

    pub(super) fn socket(&self) -> PathBuf {
        self.directory.path().join("proxy")
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.task.abort();
    }
}
