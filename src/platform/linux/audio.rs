use std::{env, ffi::OsString, process::Stdio};

use tokio::process::Command;

use crate::model;

#[cfg(goodwin_model = "mini3")]
pub(crate) mod yio;

#[derive(Clone)]
pub(crate) struct Aplay {
    program: OsString,
    device: OsString,
}

impl Aplay {
    pub(crate) fn from_env() -> Self {
        Self {
            program: env::var_os("GOODWIN_APLAY").unwrap_or_else(|| "aplay".into()),
            device: env::var_os("GOODWIN_PLAYBACK_DEVICE")
                .unwrap_or_else(|| model::PLAYBACK_DEVICE.into()),
        }
    }

    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .arg("-q")
            .arg("-D")
            .arg(&self.device)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
    }
}
