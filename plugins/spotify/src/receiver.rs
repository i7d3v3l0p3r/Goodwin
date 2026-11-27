use std::{
    ffi::CString,
    fs, io,
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    ptr,
};

use goodwin_error::Result;

const ROOT: &str = "/data/opt/soloist";
const LAUNCHER: &str = "/data/opt/soloist/goodwin-receiver";
const ENDPOINT: &str = "127.0.0.1:9090";

pub(crate) struct Receiver(Child);

impl Drop for Receiver {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(crate) fn start(address: &str) -> Result<Option<Receiver>> {
    let address = address.trim();
    if (!address.is_empty()
        && address != "ws://127.0.0.1:9090"
        && address != "ws://127.0.0.1:9090/")
        || !Path::new(LAUNCHER).try_exists()?
    {
        return Ok(None);
    }

    let cache = goodwin_plugin::data_dir()?.join("soloist-cache");
    fs::create_dir_all(&cache)?;
    let mountpoint = CString::new(cache.as_os_str().as_bytes())?;
    let parent = std::process::id() as libc::pid_t;

    let mut command = Command::new(LAUNCHER);
    command
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .args(["--device-name", "Goodwin Mini 3"])
        .args(["--data-dir", &format!("{ROOT}/data")])
        .arg("--cache-dir")
        .arg(&cache)
        .args(["--cache-size", "100", "--ws", ENDPOINT])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());

    // Use only system calls between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(io::Error::from_raw_os_error(libc::ESRCH));
            }
            // The tmpfs is private to Soloist and disappears when Soloist exits.
            if libc::unshare(libc::CLONE_NEWNS) == -1
                || libc::mount(
                    ptr::null(),
                    c"/".as_ptr(),
                    ptr::null(),
                    libc::MS_REC | libc::MS_PRIVATE,
                    ptr::null(),
                ) == -1
                || libc::mount(
                    c"tmpfs".as_ptr(),
                    mountpoint.as_ptr(),
                    c"tmpfs".as_ptr(),
                    libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
                    c"size=10m,mode=0700".as_ptr().cast(),
                ) == -1
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(Some(Receiver(command.spawn()?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_external_receiver_does_not_start_a_local_process() {
        assert!(start("ws://192.0.2.1:9090").unwrap().is_none());
    }

    #[test]
    fn dropping_the_receiver_stops_and_reaps_its_child() {
        let child = Command::new("sleep").arg("60").spawn().unwrap();
        let pid = child.id() as libc::pid_t;
        drop(Receiver(child));
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }
}
