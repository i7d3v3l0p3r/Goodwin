use std::{ffi::OsString, path::PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::{
    package::Source,
    station::{self, Address, Model},
    wake_catalog::valid_model_id,
};

pub(crate) const USAGE: &str = "\
Использование
  goodwin-installer install <адрес[:порт]> [--package <путь>] [--adb <путь>] [--model keep|<id>]
  goodwin-installer --help
  goodwin-installer --version

Поддерживаются IP-адреса и имена Станций. Порт ADB по умолчанию 5555.
  --package  Локальный архив или папка со сборкой. По умолчанию свежий релиз с GitHub.
  --adb      Путь к adb. По умолчанию рядом с установщиком или в PATH.
  --model    ID из каталога Goodwin-wakeword, например egor-micro.
             По умолчанию keep, при первой установке goodwin-standard.

Ход установки выводится в stderr, адрес панели при успехе в stdout.
";

pub(crate) enum Action {
    Gui,
    Help,
    Version,
    Install(InstallOptions),
}

pub(crate) struct InstallOptions {
    address: Address,
    adb: Option<PathBuf>,
    source: Source,
    model: Model,
}

pub(crate) fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Action> {
    let Some(mode) = args.next() else {
        return Ok(Action::Gui);
    };
    if matches!(mode.to_str(), Some("--help" | "-h" | "--version" | "-V")) {
        ensure!(
            args.next().is_none(),
            "После {mode:?} аргументы не ожидаются"
        );
        return Ok(match mode.to_str() {
            Some("--version" | "-V") => Action::Version,
            _ => Action::Help,
        });
    }
    ensure!(mode == "install", "Неизвестная команда {mode:?}");
    let address = args.next().context("Укажите адрес Станции после install")?;
    if address == "--help" || address == "-h" {
        ensure!(args.next().is_none(), "После --help аргументы не ожидаются");
        return Ok(Action::Help);
    }
    let address = Address::parse(address.to_str().context("Некорректная кодировка адреса")?)?;
    let mut adb = None;
    let mut package = None;
    let mut model = None;
    while let Some(option) = args.next() {
        match option.to_str() {
            Some("--adb") => {
                ensure!(adb.is_none(), "Параметр --adb указан повторно");
                adb = Some(PathBuf::from(value(&mut args, "--adb")?));
            }
            Some("--package") => {
                ensure!(package.is_none(), "Параметр --package указан повторно");
                package = Some(PathBuf::from(value(&mut args, "--package")?));
            }
            Some("--model") => {
                ensure!(model.is_none(), "Параметр --model указан повторно");
                model = Some(match value(&mut args, "--model")?.to_str() {
                    Some("keep") => Model::Keep,
                    Some(id) if valid_model_id(id) => Model::Id(id.into()),
                    _ => bail!("Для --model используйте keep или ID модели из каталога"),
                });
            }
            _ => bail!("Неизвестный параметр {option:?}"),
        }
    }
    Ok(Action::Install(InstallOptions {
        address,
        adb,
        source: package.map_or(Source::Latest, Source::Local),
        model: model.unwrap_or_default(),
    }))
}

fn value(args: &mut impl Iterator<Item = OsString>, option: &str) -> Result<OsString> {
    args.next()
        .filter(|value| !value.is_empty() && !value.as_encoded_bytes().starts_with(b"--"))
        .with_context(|| format!("Укажите значение для {option}"))
}

pub(crate) fn install(options: InstallOptions) -> Result<()> {
    let adb = options.adb.unwrap_or_else(station::default_adb);
    let installed = station::install(
        &adb,
        options.address,
        options.source,
        options.model,
        None,
        |line| {
            eprintln!("{line}");
        },
    )?;
    let action = if installed.updated {
        "Goodwin обновлён"
    } else {
        "Goodwin установлен"
    };
    eprintln!("{action}. Пакет {}", installed.source);
    println!("{}", installed.panel);
    Ok(())
}

#[cfg(test)]
mod tests;
