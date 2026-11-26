use std::{
    ffi::OsStr,
    fs,
    io::{BufRead, BufReader},
    net::Ipv6Addr,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use goodwin_error::{Context, Error, Result, bail, ensure, report};

use crate::{
    package::{self, Source},
    wake_catalog::{Catalog, INT8_ONLY_MARKER, Runtime, tflite_marker},
};

const INSTALL: &str = include_str!("../install-station.sh");
const INIT: &str = include_str!("../../scripts/station-goodwin-init.sh");
const SUCCESS: &str = "GOODWIN_INSTALL_OK";
const INSTALL_EXIT: &str = "GOODWIN_INSTALL_EXIT ";
const INSTALL_VERSION: &str = "GOODWIN_INSTALL_VERSION ";
const INSTALL_ERROR: &str = "GOODWIN_INSTALL_ERROR ";
const DEFAULT_ADB_PORT: u16 = 5555;

fn packaged_wake_runtime(binary: &[u8]) -> (&'static [Runtime], &'static str) {
    let has_marker = |marker: &[u8]| binary.windows(marker.len()).any(|bytes| bytes == marker);
    let int8_only = has_marker(INT8_ONLY_MARKER);
    let runtimes = Runtime::available(!int8_only, int8_only || has_marker(tflite_marker()));
    let default = if int8_only {
        "goodwin-micro-int8"
    } else {
        "goodwin-standard"
    };
    (runtimes, default)
}
const PREFLIGHT: &str = r#"
if [ "$(id -u)" != 0 ]; then
    echo ROOT_REQUIRED
elif [ "$(uname -m)" != aarch64 ] || [ ! -x /etc/init.d/S95data-services ]; then
    echo UNSUPPORTED_STATION
elif [ -e /data/opt/.goodwin-install-lock ]; then
    echo INSTALL_LOCKED
else
    echo GOODWIN_STATION_READY
fi
"#;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Model {
    #[default]
    Keep,
    Id(String),
}

#[derive(Clone)]
pub(crate) struct Address {
    host: String,
    port: u16,
}

impl Address {
    #[cfg(feature = "gui")]
    pub(crate) fn from_ip(ip: std::net::IpAddr) -> Self {
        use std::net::IpAddr;

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
    #[cfg(feature = "gui")]
    pub(crate) version: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallIssue {
    Adb,
    Connection,
    Root,
    Unsupported,
    LocalPackage,
    Release,
    Model,
    Locked,
    Layout,
    RolledBack,
    RecoveryFailed,
    Unconfirmed,
}

impl std::fmt::Display for InstallIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Adb => "ADB не найден или не запускается",
            Self::Connection => "Станция недоступна через ADB",
            Self::Root => "Для установки на подготовленной Станции нужен ADB с правами root",
            Self::Unsupported => {
                "Ожидается подготовленная Станция Мини 3 с поддержкой /data/init.d"
            }
            Self::LocalPackage => "Не удалось подготовить локальную сборку",
            Self::Release => "Не удалось скачать или проверить релиз Goodwin",
            Self::Model => "Не удалось подготовить или загрузить модель активации",
            Self::Locked => "Другая установка ещё идёт или требует восстановления",
            Self::Layout => "Состояние установленного Goodwin не соответствует ожидаемому",
            Self::RolledBack => "Установка прервалась. Предыдущее состояние восстановлено",
            Self::RecoveryFailed => "Автоматическое восстановление не завершилось",
            Self::Unconfirmed => "Станция не подтвердила результат установки",
        })
    }
}

impl std::error::Error for InstallIssue {}

#[derive(Debug)]
struct InstallError {
    issue: InstallIssue,
    cause: Error,
}

impl InstallError {
    fn new(issue: InstallIssue, cause: impl Into<Error>) -> Self {
        Self {
            issue,
            cause: cause.into(),
        }
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.issue, formatter)
    }
}

impl core::error::Error for InstallError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(self.cause.as_error())
    }
}

#[cfg(any(feature = "gui", test))]
pub(crate) fn install_issue(error: &Error) -> Option<InstallIssue> {
    error
        .find::<InstallError>()
        .map(|error| error.issue)
        .or_else(|| error.find::<InstallIssue>().copied())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InstallStage {
    #[default]
    Connect,
    Package,
    Transfer,
    Install,
}

fn connect(adb: &Adb<'_>, log: &impl Fn(String)) -> Result<()> {
    log(format!("Подключаемся к {}…", adb.serial));
    let output = adb.run(None, 25, ["connect", adb.serial])?;
    if !output.is_empty() {
        log(output);
    }
    ensure!(
        adb.on_station(15, ["get-state"])?.trim() == "device",
        InstallIssue::Connection
    );
    match adb.on_station(15, ["shell", PREFLIGHT])?.trim() {
        "ROOT_REQUIRED" => {
            bail!(InstallIssue::Root)
        }
        "UNSUPPORTED_STATION" => {
            bail!(InstallIssue::Unsupported)
        }
        "INSTALL_LOCKED" => bail!(InstallIssue::Locked),
        "GOODWIN_STATION_READY" => Ok(()),
        output => bail!("Не удалось проверить Станцию: {output}"),
    }
}

fn already_installed(adb: &Adb<'_>) -> Result<bool> {
    let existing = adb.on_station(
        15,
        [
            "shell",
            "if [ -f /data/opt/etc/goodwin/config.json ]; then echo update; else echo fresh; fi",
        ],
    )?;
    match existing.trim() {
        "fresh" => Ok(false),
        "update" => Ok(true),
        _ => bail!("Не удалось определить, установлен ли Goodwin на Станции"),
    }
}

#[cfg(feature = "gui")]
pub(crate) fn check(adb: &Path, address: &Address) -> Result<bool> {
    let serial = address.serial();
    let adb = Adb {
        executable: adb,
        serial: &serial,
    };
    connect(&adb, &|_| {})?;
    already_installed(&adb)
}

pub(crate) fn install(
    adb: &Path,
    address: Address,
    source: Source,
    model: Model,
    catalog: Option<&Catalog>,
    log: impl Fn(String) + Sync,
) -> Result<Installed> {
    install_with_progress(adb, address, source, model, catalog, log, |_| {})
}

pub(crate) fn install_with_progress(
    adb: &Path,
    address: Address,
    source: Source,
    model: Model,
    catalog: Option<&Catalog>,
    log: impl Fn(String) + Sync,
    phase: impl Fn(InstallStage),
) -> Result<Installed> {
    let serial = address.serial();
    let adb = Adb {
        executable: adb,
        serial: &serial,
    };
    phase(InstallStage::Connect);
    connect(&adb, &log)?;

    phase(InstallStage::Package);
    let work = tempfile::Builder::new().prefix("goodwin-").tempdir()?;
    let issue = if matches!(&source, Source::Local(_)) {
        InstallIssue::LocalPackage
    } else {
        InstallIssue::Release
    };
    let source = package::prepare(source, work.path(), &log)
        .map_err(|error| InstallError::new(issue, error))?;
    let (choice, model_file) = if matches!(model, Model::Id(_)) || !already_installed(&adb)? {
        let payload = work.path().join("payload");
        let binary = fs::read(payload.join("goodwin"))?;
        let (runtimes, default) = packaged_wake_runtime(&binary);
        drop(binary);
        let id = match &model {
            Model::Id(id) => id.as_str(),
            Model::Keep => default,
        };
        log(format!("Скачиваем модель активации {id}…"));
        let latest;
        let catalog = match catalog {
            Some(catalog) => catalog,
            None => {
                latest = Catalog::latest(runtimes)
                    .map_err(|error| InstallError::new(InstallIssue::Model, error))?;
                &latest
            }
        };
        let model = catalog
            .models
            .iter()
            .find(|model| model.selection.id == id)
            .context("В каталоге нет выбранной модели активации")
            .map_err(|error| InstallError::new(InstallIssue::Model, error))?;
        ensure!(
            runtimes.contains(&model.selection.runtime),
            InstallIssue::Model
        );
        let name = match model.selection.runtime {
            Runtime::MicroTflite => "wake-model.tflite",
            _ => "wake-model.onnx",
        };
        let selection = catalog
            .download(id, fs::File::create(payload.join(name))?)
            .map_err(|error| InstallError::new(InstallIssue::Model, error))?;
        goodwin_json::to_writer(
            fs::File::create(payload.join("wake-model.json"))?,
            selection,
        )?;
        ("catalog", Some(name))
    } else {
        ("keep", None)
    };
    let scratch_kib = package::build_payload(work.path(), model_file)?;
    for (name, script) in [("install-station.sh", INSTALL), ("init.sh", INIT)] {
        fs::write(work.path().join(name), script.replace("\r\n", "\n"))?;
    }

    phase(InstallStage::Transfer);
    // Keep the upload and extraction off the persistent volume when tmpfs has room.
    let tmp_available = adb.on_station(10, ["shell", "df -Pk /tmp"]).ok();
    let tmp_has_room = tmp_available
        .as_deref()
        .and_then(|output| {
            output
                .lines()
                .last()?
                .split_whitespace()
                .nth(3)?
                .parse::<u64>()
                .ok()
        })
        .is_some_and(|available| available >= scratch_kib + 1024);
    let upload_parent = if tmp_has_room { "/tmp" } else { "/data/opt" };
    let upload_prefix = format!("{upload_parent}/.goodwin-upload.");
    log(format!("Отправляем пакет на Станцию ({upload_parent})…"));
    let remote = adb.on_station(
        15,
        [
            "shell",
            &format!("umask 077; mkdir -p {upload_parent} && mktemp -d {upload_prefix}XXXXXX"),
        ],
    )?;
    let suffix = remote.strip_prefix(&upload_prefix).unwrap_or_default();
    ensure!(
        suffix.len() == 6 && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "Не удалось создать временную папку на Станции"
    );

    let install = transfer_and_install(&adb, work.path(), &remote, choice, &log, &phase);
    if adb.on_station(10, [
        "shell",
        &format!("rm -rf {remote}/payload; rm -f {remote}/payload.tar.gz {remote}/install-station.sh {remote}/init.sh; rmdir {remote}"),
    ]).is_err() {
        log(format!("На Станции осталась временная папка {remote}"));
    }
    let install = install?;
    Ok(Installed {
        source,
        panel: address.panel(),
        updated: install.updated,
        #[cfg(feature = "gui")]
        version: install.version,
    })
}

fn transfer_and_install(
    adb: &Adb<'_>,
    work: &Path,
    remote: &str,
    choice: &str,
    log: &(impl Fn(String) + Sync),
    phase: &impl Fn(InstallStage),
) -> Result<RemoteInstallation> {
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
    phase(InstallStage::Install);
    log("Устанавливаем Goodwin и проверяем запуск…".into());
    // Legacy ADB shells can exit successfully without returning the remote status.
    let script = format!(
        "sh {remote}/install-station.sh {remote} {choice}; status=$?; printf '\\n{INSTALL_EXIT}%s\\n' \"$status\"",
    );
    let report = |line: String| {
        if ![SUCCESS, INSTALL_EXIT, INSTALL_VERSION, INSTALL_ERROR]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            log(line);
        }
    };
    let result = adb
        .run(Some(&report), 360, ["-s", adb.serial, "shell", &script])
        .map_err(|error| InstallError::new(InstallIssue::Unconfirmed, error))?;
    install_result(&result)
}

#[derive(Debug)]
struct RemoteInstallation {
    updated: bool,
    #[cfg(feature = "gui")]
    version: Option<String>,
}

fn install_result(output: &str) -> Result<RemoteInstallation> {
    let mut lines = output.lines().map(str::trim);
    let status = lines
        .next_back()
        .and_then(|line| line.strip_prefix(INSTALL_EXIT))
        .and_then(|value| value.parse::<u8>().ok())
        .context("ADB не вернул код завершения установки на Станции")
        .map_err(|error| InstallError::new(InstallIssue::Unconfirmed, error))?;
    if status != 0 {
        let error = report!("Скрипт установки на Станции завершился с кодом {status}");
        let issue = lines
            .rev()
            .find_map(|line| match line.strip_prefix(INSTALL_ERROR)? {
                "model" => Some(InstallIssue::Model),
                "locked" => Some(InstallIssue::Locked),
                "layout" => Some(InstallIssue::Layout),
                "rolled_back" => Some(InstallIssue::RolledBack),
                "recovery_failed" => Some(InstallIssue::RecoveryFailed),
                _ => None,
            });
        return Err(match issue {
            Some(issue) => InstallError::new(issue, error).into(),
            None => error,
        });
    }
    #[cfg(feature = "gui")]
    let version = lines
        .clone()
        .find_map(|line| line.strip_prefix(INSTALL_VERSION))
        .filter(|version| {
            !version.is_empty()
                && version.len() <= 128
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte))
        })
        .map(str::to_owned);
    let updated = match lines.rfind(|line| line.starts_with(SUCCESS)) {
        Some("GOODWIN_INSTALL_OK fresh") => false,
        Some("GOODWIN_INSTALL_OK update") => true,
        _ => bail!(InstallIssue::Unconfirmed),
    };
    Ok(RemoteInstallation {
        updated,
        #[cfg(feature = "gui")]
        version,
    })
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
        let mut child = command
            .spawn()
            .map_err(|error| InstallError::new(InstallIssue::Adb, error))?;
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
                            _ => report!("Станция или ADB не ответили вовремя"),
                        });
                    }
                }
            };
            let output = output
                .join()
                .map_err(|_| report!("Не удалось прочитать вывод ADB"))??;
            let errors = errors
                .join()
                .map_err(|_| report!("Не удалось прочитать ошибки ADB"))??;
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
    fn package_capabilities_select_a_supported_default_model() {
        let (runtimes, default) = packaged_wake_runtime(b"legacy");
        assert_eq!(runtimes, &[Runtime::Standard, Runtime::Micro]);
        assert_eq!(default, "goodwin-standard");
        let (runtimes, default) = packaged_wake_runtime(tflite_marker());
        assert_eq!(
            runtimes,
            &[Runtime::Standard, Runtime::Micro, Runtime::MicroTflite]
        );
        assert_eq!(default, "goodwin-standard");
        let compact = [INT8_ONLY_MARKER, tflite_marker()].concat();
        let (runtimes, default) = packaged_wake_runtime(&compact);
        assert_eq!(runtimes, &[Runtime::MicroTflite]);
        assert_eq!(default, "goodwin-micro-int8");
    }

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
        assert!(
            !install_result("GOODWIN_INSTALL_OK fresh\r\nGOODWIN_INSTALL_EXIT 0\r\n")
                .unwrap()
                .updated
        );
        assert!(
            install_result("GOODWIN_INSTALL_OK update\nGOODWIN_INSTALL_EXIT 0")
                .unwrap()
                .updated
        );
        let error = install_result("Could not open the upload directory.\nGOODWIN_INSTALL_EXIT 2")
            .unwrap_err();
        assert!(error.to_string().ends_with("с кодом 2"));
        assert_eq!(
            install_issue(&install_result("").unwrap_err()),
            Some(InstallIssue::Unconfirmed)
        );
        assert!(
            install_result("GOODWIN_INSTALL_EXIT 0")
                .unwrap_err()
                .to_string()
                .contains("Станция не подтвердила результат")
        );
    }

    #[test]
    fn installation_retains_the_remote_recovery_status() {
        let error = install_result("GOODWIN_INSTALL_ERROR recovery_failed\nGOODWIN_INSTALL_EXIT 1")
            .unwrap_err();
        assert_eq!(install_issue(&error), Some(InstallIssue::RecoveryFailed));
        let error = install_result("GOODWIN_INSTALL_ERROR rolled_back\nGOODWIN_INSTALL_EXIT 1")
            .unwrap_err();
        assert_eq!(install_issue(&error), Some(InstallIssue::RolledBack));
    }

    #[cfg(feature = "gui")]
    #[test]
    fn installation_reports_the_running_version_even_for_a_local_package() {
        let result = install_result("GOODWIN_INSTALL_VERSION 0.3.1-local+test\r\nGOODWIN_INSTALL_OK fresh\r\nGOODWIN_INSTALL_EXIT 0").unwrap();
        assert_eq!(result.version.as_deref(), Some("0.3.1-local+test"));
        let legacy = install_result("GOODWIN_INSTALL_OK update\nGOODWIN_INSTALL_EXIT 0").unwrap();
        assert!(legacy.version.is_none());
    }
}
