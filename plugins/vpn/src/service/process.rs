use goodwin_error::{Context, Result, ensure};
use std::{
    fs,
    io::Read,
    os::{
        fd::AsRawFd,
        unix::{fs::PermissionsExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use crate::types::Launch;

pub(super) struct Pipes {
    pub input: ChildStdin,
    pub output: ChildStdout,
    pub socket: PathBuf,
}

pub(super) struct Process(pub Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn spawn(args: &[&std::ffi::OsStr], socket: PathBuf) -> Result<(Process, Pipes)> {
    let helper = std::env::current_exe()?.with_file_name("vpn-fetch");
    if fs::metadata(&helper)?.permissions().mode() & 0o777 != 0o700 {
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))?;
    }
    let mut command = Command::new(helper);
    command
        .args(args)
        .env_remove("GOODWIN_VPN_PASSWORD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let parent = std::process::id();
    // SAFETY: only async-signal-safe calls run after fork. The main event loop
    // owns every spawn because Linux ties the death signal to the spawning thread.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() as u32 != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
    let mut child = Process(command.spawn()?);
    let input = child.0.stdin.take().context("Missing helper input")?;
    let output = child.0.stdout.take().context("Missing helper output")?;
    Ok((
        child,
        Pipes {
            input,
            output,
            socket,
        },
    ))
}

pub(super) struct Worker {
    process: Option<Process>,
    directory: tempfile::TempDir,
    name: &'static str,
}
impl Worker {
    pub fn new(socket: &Path, name: &'static str) -> Result<(Self, Pipes)> {
        let directory = tempfile::Builder::new()
            .prefix("w")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(socket.parent().context("Missing socket directory")?)?;
        let path = directory.path().join("proxy");
        let (child, pipes) = spawn(&[name.as_ref(), path.as_os_str()], path.clone())?;
        Ok((
            Self {
                process: Some(child),
                directory,
                name,
            },
            pipes,
        ))
    }
    pub fn socket(&self) -> PathBuf {
        self.directory.path().join("proxy")
    }
    pub fn alive(&mut self) -> bool {
        self.process
            .as_mut()
            .is_some_and(|p| matches!(p.0.try_wait(), Ok(None)))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.process.take();
        goodwin_vpn::cache::discard_partial(self.name);
    }
}

fn status_byte(output: &mut ChildStdout, timeout_ms: i32) -> Result<u8> {
    let mut fd = libc::pollfd {
        fd: output.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll borrows a single initialized descriptor for this call.
    ensure!(
        unsafe { libc::poll(&mut fd, 1, timeout_ms) } == 1,
        "VPN startup timed out"
    );
    let mut byte = [0];
    output.read_exact(&mut byte)?;
    Ok(byte[0])
}

pub(super) fn handshake(mut pipes: Pipes, launch: &Launch) -> super::JobResult<PathBuf> {
    if status_byte(&mut pipes.output, 105_000).ok() != Some(0) {
        return Err("vpn_engine_download_failed");
    }
    serde_json::to_writer(&mut pipes.input, launch).map_err(|_| "vpn_engine_failed")?;
    drop(pipes.input);
    if status_byte(&mut pipes.output, 5000).ok() != Some(1) {
        return Err("vpn_engine_failed");
    }
    Ok(pipes.socket)
}
