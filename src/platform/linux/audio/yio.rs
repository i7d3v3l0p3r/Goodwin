use std::{env, net::SocketAddr, sync::Arc};

use anyhow::{Context, Result, ensure};
use tokio::task::JoinHandle;

use crate::app::App;

mod microphone;
mod protocol;

#[cfg(test)]
pub(crate) use microphone::run_microphone;
#[cfg(test)]
pub(crate) use protocol::{Framing, write_samples};

pub(crate) struct Capture {
    endpoint: SocketAddr,
}

impl Capture {
    pub(crate) fn from_env(default: &str) -> Result<Self> {
        let endpoint: SocketAddr = env::var("GOODWIN_MICROPHONE")
            .unwrap_or_else(|_| default.into())
            .parse()
            .context("Invalid microphone address")?;
        ensure!(
            endpoint.ip().is_loopback(),
            "The microphone address must be loopback"
        );
        Ok(Self { endpoint })
    }

    pub(crate) fn start(&self, app: Arc<App>) -> JoinHandle<()> {
        tokio::spawn(microphone::run_microphone(app, self.endpoint))
    }
}
