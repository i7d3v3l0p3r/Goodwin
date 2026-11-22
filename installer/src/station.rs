use std::{
    ffi::OsStr,
    fs,
    io::{BufRead, BufReader},
    net::{IpAddr, Ipv6Addr},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail, ensure};

use crate::package::{self, Source};

const INSTALL: &str = include_str!("../install-station.sh");
const INIT: &str = include_str!("../../scripts/station-goodwin-init.sh");
const SUCCESS: &str = "GOODWIN_INSTALL_OK";
const INSTALL_EXIT: &str = "GOODWIN_INSTALL_EXIT ";
const DEFAULT_ADB_PORT: u16 = 5555;
const PREFLIGHT: &str = r#"
if [ "$(id -u)" != 0 ]; then
    echo ROOT_REQUIRED
elif [ "$(uname -m)" != aarch64 ] || [ ! -x /etc/init.d/S95data-services ]; then
    echo UNSUPPORTED_STATION
else
    echo GOODWIN_STATION_READY
fi
"#;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Model {
    #[default]
    Keep,
    Standard,
    Micro,
}

impl Model {
    fn arg(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Standard => "standard",
            Self::Micro => "micro",
        }
    }
}

pub(crate) struct Address {
    host: String,
    port: u16,
}

impl Address {
    pub(crate) fn from_ip(ip: IpAddr) -> Self {
        let host = match ip {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        };
        Self {
            host,
            port: DEFAULT_ADB_PORT,
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        let (host, port) = if let Some((host, suffix)) = value
            .strip_prefix('[')
            .and_then(|value| value.split_once(']'))
        {
            host.parse::<Ipv6Addr>()
                .context("Некорректный IPv6-адрес Станции")?;
            let port = if suffix.is_empty() {
                None
            } else {
                Some(
                    suffix
                        .strip_prefix(':')
                        .context("Укажите порт после двоеточия")?,
                )
            };
            (format!("[{host}]"), port)
        } else {
            let (host, port) = value
                .split_once(':')
                .map_or((value, None), |(host, port)| (host, Some(port)));
            ensure!(
                !host.is_empty()
                    && host.len() <= 253
                    && host.split('.').all(|part| {
                        !part.is_empty()
                            && part.len() <= 63
                            && !part.starts_with('-')
                            && !part.ends_with('-')
                            && part
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    }),
                "Укажите IP-адрес или имя Станции, например 192.168.1.42"
            );
            (host.to_owned(), port)
        };
        let port = port
            .map(str::parse::<u16>)
            .transpose()
            .context("Некорректный порт ADB")?
            .unwrap_or(DEFAULT_ADB_PORT);
        ensure!(port > 0, "Некорректный порт ADB");
        Ok(Self { host, port })
    }

    pub(crate) fn serial(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    fn panel(&self) -> String {
        format!("http://{}:8787/", self.host)
    }
}

pub(crate) struct Installed {
    pub(crate) source: String,
    pub(crate) panel: String,
    pub(crate) updated: bool,
}

pub(crate) fn install(
    adb: &Path,
    address: Address,
    source: Source,
    model: Model,
    log: impl Fn(String) + Sync,
) -> Result<Installed> {
    let serial = address.serial();
    let adb = Adb {
        executable: adb,
        serial: &serial,
    };
    log(format!("Подключаемся к {serial}…"));
    let connect = adb.run(None, 25, ["connect", &serial])?;
    if !connect.is_empty() {
        log(connect);
    }
    ensure!(
        adb.on_station(15, ["get-state"])?.trim() == "device",
        "Станция недоступна через ADB"
    );
    match adb.on_station(15, ["shell", PREFLIGHT])?.trim() {
        "ROOT_REQUIRED" => {
            bail!("Для установки на подготовленной Станции нужен ADB с правами root")
        }
        "UNSUPPORTED_STATION" => {
            bail!("Ожидается подготовленная Станция Мини 3 с поддержкой /data/init.d")
        }
        "GOODWIN_STATION_READY" => {}
        output => bail!("Не удалось проверить Станцию: {output}"),
    }

    let work = tempfile::Builder::new().prefix("goodwin-").tempdir()?;
    let source = package::prepare(source, work.path(), &log)?;
    for (name, script) in [("install-station.sh", INSTALL), ("init.sh", INIT)] {
        fs::write(work.path().join(name), script.replace("\r\n", "\n"))?;
    }

    log("Отправляем пакет на Станцию…".into());
    let remote = adb.on_station(
        15,
        [
            "shell",
            "umask 077; mkdir -p /data/opt && mktemp -d /data/opt/.goodwin-upload.XXXXXX",
        ],
    )?;
    let suffix = remote
        .strip_prefix("/data/opt/.goodwin-upload.")
        .unwrap_or_default();
    ensure!(
        suffix.len() == 6 && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "Не удалось создать временную папку на Станции"
    );

    let install = transfer_and_install(&adb, work.path(), &remote, model, &log);
    if adb.on_station(10, [
        "shell",
        &format!("rm -f {remote}/payload.tar.gz {remote}/install-station.sh {remote}/init.sh; rmdir {remote}"),
    ]).is_err() {
        log(format!("На Станции осталась временная папка {remote}"));
    }
    Ok(Installed {
        source,
        panel: address.panel(),
        updated: install?,
    })
}

fn transfer_and_install(
    adb: &Adb<'_>,
    work: &Path,
    remote: &str,
    model: Model,
    log: &(impl Fn(String) + Sync),
) -> Result<bool> {
    for name in ["payload.tar.gz", "install-station.sh", "init.sh"] {
        let local = work.join(name);
        let destination = format!("{remote}/{name}");
        adb.on_station(
            180,
            [
                OsStr::new("push"),
                local.as_os_str(),
                OsStr::new(&destination),
            ],
        )?;
    }
    log("Устанавливаем Goodwin и проверяем запуск…".into());
    // Legacy ADB shells can exit successfully without returning the remote status.
    let script = format!(
        "sh {remote}/install-station.sh {remote} {}; status=$?; printf '\\n{INSTALL_EXIT}%s\\n' \"$status\"",
        model.arg(),
    );
    let report = |line: String| {
        if !line.starts_with(SUCCESS) && !line.starts_with(INSTALL_EXIT) {
            log(line);
        }
    };
    let result = adb.run(Some(&report), 360, ["-s", adb.serial, "shell", &script])?;
    install_result(&result)
}

fn install_result(output: &str) -> Result<bool> {
    let mut lines = output.lines().map(str::trim);
    let status = lines
        .next_back()
        .and_then(|line| line.strip_prefix(INSTALL_EXIT))
        .and_then(|value| value.parse::<u8>().ok())
        .context("ADB не вернул код завершения установки на Станции")?;
    ensure!(
        status == 0,
        "Скрипт установки на Станции завершился с кодом {status}"
    );
    match lines.rfind(|line| line.starts_with(SUCCESS)) {
        Some("GOODWIN_INSTALL_OK fresh") => Ok(false),
        Some("GOODWIN_INSTALL_OK update") => Ok(true),
        _ => bail!("Станция не подтвердила запуск Goodwin"),
    }
}

struct Adb<'a> {
    executable: &'a Path,
    serial: &'a str,
}

impl Adb<'_> {
    fn on_station(
        &self,
        seconds: u64,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<String> {
        let args = [OsStr::new("-s"), OsStr::new(self.serial)]
            .into_iter()
            .map(OsStr::to_os_string)
            .chain(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self.run(None, seconds, args)
    }

    fn run(
        &self,
        log: Option<&(dyn Fn(String) + Sync)>,
        seconds: u64,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<String> {
        let mut command = Command::new(self.executable);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().context(
            "ADB не найден или не запускается. Установите Android SDK Platform-Tools или выберите файл ADB в окне установщика",
        )?;
        let stdout = child.stdout.take().context("Нет вывода ADB")?;
        let stderr = child.stderr.take().context("Нет вывода ошибок ADB")?;
        let start = Instant::now();
        let (status, output, errors) = thread::scope(|scope| -> Result<_> {
            let output = scope.spawn(|| read_output(stdout, log));
            let errors = scope.spawn(|| read_output(stderr, log));
            let status: Result<ExitStatus> = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) if start.elapsed() < Duration::from_secs(seconds) => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    result => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(match result {
                            Err(error) => error.into(),
                            _ => anyhow!("Станция или ADB не ответили вовремя"),
                        });
                    }
                }
            };
            let output = output
                .join()
                .map_err(|_| anyhow!("Не удалось прочитать вывод ADB"))??;
            let errors = errors
                .join()
                .map_err(|_| anyhow!("Не удалось прочитать ошибки ADB"))??;
            Ok((status?, output, errors))
        })?;
        if !status.success() {
            let detail = if errors.is_empty() { &output } else { &errors };
            bail!("ADB завершил команду с ошибкой ({status})\n{detail}");
        }
        Ok(output)
    }
}

fn read_output(input: impl std::io::Read, log: Option<&(dyn Fn(String) + Sync)>) -> Result<String> {
    let mut output = String::new();
    for line in BufReader::new(input).split(b'\n') {
        let line = line?;
        let line = String::from_utf8_lossy(&line);
        let line = line.trim_end_matches('\r');
        if let Some(log) = log.filter(|_| !line.is_empty()) {
            log(line.to_owned());
        }
        output.push_str(line);
        output.push('\n');
    }
    Ok(output.trim().to_owned())
}

pub(crate) fn default_adb() -> PathBuf {
    let executable = if cfg!(windows) { "adb.exe" } else { "adb" };
    let adjacent = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|path| path.join(executable)));
    adjacent
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(executable))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn station_address_uses_separate_adb_and_panel_ports() {
        let station = Address::parse("station.local").unwrap();
        assert_eq!(station.serial(), "station.local:5555");
        let station = Address::parse("[fd00::42]:4444").unwrap();
        assert_eq!(station.serial(), "[fd00::42]:4444");
        assert_eq!(station.panel(), "http://[fd00::42]:8787/");
        assert!(Address::parse("http://station.local").is_err());
        assert!(Address::parse("station.local:0").is_err());
    }

    #[test]
    fn installation_uses_the_remote_exit_status_on_legacy_adb() {
        assert!(!install_result("GOODWIN_INSTALL_OK fresh\r\nGOODWIN_INSTALL_EXIT 0\r\n").unwrap());
        assert!(install_result("GOODWIN_INSTALL_OK update\nGOODWIN_INSTALL_EXIT 0").unwrap());
        let error = install_result("Could not open the upload directory.\nGOODWIN_INSTALL_EXIT 2")
            .unwrap_err();
        assert!(error.to_string().ends_with("с кодом 2"));
        assert!(
            install_result("")
                .unwrap_err()
                .to_string()
                .contains("ADB не вернул код")
        );
        assert!(
            install_result("GOODWIN_INSTALL_EXIT 0")
                .unwrap_err()
                .to_string()
                .contains("Станция не подтвердила запуск")
        );
    }
}
