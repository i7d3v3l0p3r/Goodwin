use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};

const REPO: &str = "i7d3v3l0p3r/Goodwin";
const ARCHIVE: &str = "goodwin-mini3-aarch64.tar.gz";
const PREFIX: &str = "goodwin-mini3";
const SUMS: &str = "SHA256SUMS";
const PACKAGE_LIMIT: u64 = 128 * 1024 * 1024;
const METADATA_LIMIT: u64 = 64 * 1024;
const FILES: [&str; 5] = [
    "goodwin",
    "models/standard.onnx",
    "models/micro.onnx",
    "models/LICENSE",
    "models/NOTICE",
];

pub(crate) enum Source {
    Latest,
    Local(PathBuf),
}

pub(crate) fn prepare(source: Source, work: &Path, log: &impl Fn(String)) -> Result<(String, u64)> {
    let unpack = work.join("payload");
    fs::create_dir_all(unpack.join("models"))?;
    let label = match source {
        Source::Latest => {
            let archive = work.join(ARCHIVE);
            let tag = download_release(work, log)?;
            log("Проверяем пакет Goodwin…".into());
            unpack_archive(&archive, &unpack)?;
            tag
        }
        Source::Local(path) => {
            log("Проверяем локальную сборку…".into());
            ensure!(
                path.is_file() || path.is_dir(),
                "Локальная сборка недоступна: {}",
                path.display()
            );
            let archive = if path.is_file() {
                path.clone()
            } else {
                path.join(ARCHIVE)
            };
            if archive.is_file() {
                let manifest = archive.with_file_name(SUMS);
                if archive.file_name().is_some_and(|name| name == ARCHIVE) && manifest.is_file() {
                    verify(&archive, ARCHIVE, &read_checksums(&manifest)?)?;
                }
                unpack_archive(&archive, &unpack)?;
            } else {
                let nested = path.join(PREFIX);
                let files = if !path.join("goodwin").is_file() && nested.is_dir() {
                    &nested
                } else {
                    &path
                };
                copy_folder(files, &unpack)?;
            }
            path.display().to_string()
        }
    };
    let scratch_kib = build_payload(&unpack, &work.join("payload.tar.gz"))?;
    Ok((label, scratch_kib))
}

fn download_release(work: &Path, log: &impl Fn(String)) -> Result<String> {
    let client = Client::builder()
        .https_only(true)
        .user_agent("GoodwinInstaller")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(240))
        .build()?;
    let release = client
        .get(format!("https://github.com/{REPO}/releases/latest"))
        .timeout(Duration::from_secs(30))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .context("Не удалось получить последний релиз Goodwin с GitHub")?;
    let tag = release
        .url()
        .path()
        .strip_prefix(&format!("/{REPO}/releases/tag/"))
        .filter(|tag| release.url().host_str() == Some("github.com") && release_tag(tag))
        .context("GitHub не вернул тег последнего релиза Goodwin")?
        .to_owned();
    log(format!("Найден релиз {tag}. Скачиваем пакет…"));
    for (name, limit) in [(SUMS, METADATA_LIMIT), (ARCHIVE, PACKAGE_LIMIT)] {
        let response = client
            .get(format!(
                "https://github.com/{REPO}/releases/download/{tag}/{name}"
            ))
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(reqwest::Error::without_url)
            .with_context(|| format!("Не удалось скачать {name} из релиза {tag}"))?;
        copy_limited(response, &work.join(name), limit)?;
    }
    verify(
        &work.join(ARCHIVE),
        ARCHIVE,
        &read_checksums(&work.join(SUMS))?,
    )?;
    Ok(tag)
}

fn release_tag(tag: &str) -> bool {
    tag.strip_prefix('v').is_some_and(|version| {
        let mut parts = version.split('.');
        (&mut parts)
            .take(3)
            .filter(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .count()
            == 3
            && parts.next().is_none()
    })
}

fn unpack_archive(source: &Path, output: &Path) -> Result<()> {
    let input = File::open(source)?;
    ensure!(
        input.metadata()?.len() <= PACKAGE_LIMIT,
        "Пакет слишком большой"
    );
    let mut archive = tar::Archive::new(GzDecoder::new(input));
    let mut seen = HashSet::new();
    let mut left = PACKAGE_LIMIT;
    for (index, entry) in archive.entries()?.enumerate() {
        let entry = entry?;
        ensure!(index < 32 && entry.size() <= left, "Пакет слишком большой");
        left -= entry.size();
        let path = entry.path_bytes();
        let path = std::str::from_utf8(&path)?;
        let kind = entry.header().entry_type();
        if kind.is_dir() && matches!(path.trim_end_matches('/'), PREFIX | "goodwin-mini3/models") {
            continue;
        }
        ensure!(kind.is_file(), "В пакете есть специальный файл: {path}");
        let name = path
            .strip_prefix("goodwin-mini3/")
            .with_context(|| format!("Неожиданный путь в пакете: {path}"))?;
        ensure!(seen.insert(name.to_owned()), "Повторяющийся файл: {name}");
        if matches!(name, "README.md" | "LICENSE" | "start.sh") {
            continue;
        }
        ensure!(
            FILES.contains(&name) || name == SUMS,
            "Неожиданный файл: {name}"
        );
        let dest = output.join(name);
        let limit = if name == SUMS {
            METADATA_LIMIT
        } else {
            PACKAGE_LIMIT
        };
        copy_limited(entry, &dest, limit)?;
    }
    let sums = read_checksums(&output.join(SUMS)).context("Некорректный манифест пакета")?;
    for name in FILES {
        verify(&output.join(name), name, &sums)?;
    }
    Ok(())
}

fn copy_folder(source: &Path, output: &Path) -> Result<()> {
    let manifest = source.join(SUMS);
    let sums = manifest
        .is_file()
        .then(|| read_checksums(&manifest))
        .transpose()?;
    let mut left = PACKAGE_LIMIT;
    for name in FILES {
        let file = File::open(source.join(name)).with_context(|| {
            format!("В папке нет файла {name}. Выберите папку с архивом {ARCHIVE} или распакованной сборкой")
        })?;
        let dest = output.join(name);
        left -= copy_limited(file, &dest, left)?;
        if let Some(sums) = &sums {
            verify(&dest, name, sums)?;
        }
    }
    Ok(())
}

fn copy_limited(input: impl Read, dest: &Path, limit: u64) -> Result<u64> {
    let copied = io::copy(&mut input.take(limit + 1), &mut File::create(dest)?)?;
    ensure!(
        copied <= limit,
        "Файл превышает допустимый размер: {}",
        dest.display()
    );
    Ok(copied)
}

fn read_checksums(path: &Path) -> Result<HashMap<String, String>> {
    let mut manifest = String::new();
    File::open(path)?
        .take(METADATA_LIMIT + 1)
        .read_to_string(&mut manifest)?;
    ensure!(
        manifest.len() as u64 <= METADATA_LIMIT,
        "SHA256SUMS слишком большой"
    );
    let mut sums = HashMap::new();
    for line in manifest.lines() {
        let Some((hash, name)) = line.split_once(' ') else {
            bail!("Некорректная строка SHA256SUMS");
        };
        let name = name
            .strip_prefix([' ', '*'])
            .context("Некорректная строка SHA256SUMS")?;
        ensure!(
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "Некорректный SHA-256"
        );
        ensure!(
            sums.insert(name.to_owned(), hash.to_owned()).is_none(),
            "Повторяющаяся запись в SHA256SUMS: {name}"
        );
    }
    Ok(sums)
}

fn checksum(path: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    io::copy(
        &mut File::open(path).with_context(|| format!("Не удалось открыть {}", path.display()))?,
        &mut hash,
    )?;
    Ok(format!("{:x}", hash.finalize()))
}

fn verify(path: &Path, name: &str, sums: &HashMap<String, String>) -> Result<()> {
    let expected = sums
        .get(name)
        .with_context(|| format!("Нет SHA-256 для {name}"))?;
    ensure!(
        expected.eq_ignore_ascii_case(&checksum(path)?),
        "SHA-256 не совпадает для {name}"
    );
    Ok(())
}

fn build_payload(input: &Path, output: &Path) -> Result<u64> {
    let mut header = [0; 20];
    let valid_elf = File::open(input.join("goodwin"))?
        .read_exact(&mut header)
        .is_ok()
        && &header[..6] == b"\x7fELF\x02\x01"
        && header[18..20] == [0xb7, 0];
    ensure!(
        valid_elf,
        "В пакете ожидается Linux-бинарник Goodwin для AArch64"
    );

    let mut manifest = File::create(input.join(SUMS))?;
    for name in FILES {
        writeln!(manifest, "{}  {name}", checksum(&input.join(name))?)?;
    }
    let mut archive = tar::Builder::new(GzEncoder::new(File::create(output)?, Compression::fast()));
    let mut unpacked_bytes = 0;
    for name in FILES.into_iter().chain([SUMS]) {
        let file = input.join(name);
        unpacked_bytes += file.metadata()?.len();
        archive.append_path_with_name(file, name)?;
    }
    archive.into_inner()?.finish()?;
    Ok((output.metadata()?.len() + unpacked_bytes).div_ceil(1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_fixture(folder: &Path) {
        fs::create_dir(folder.join("models")).unwrap();
        let mut elf = [0; 20];
        elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
        elf[18] = 0xb7;
        fs::write(folder.join("goodwin"), elf).unwrap();
        for name in &FILES[1..] {
            fs::write(folder.join(name), name).unwrap();
        }
    }

    #[test]
    fn local_archive_and_unpacked_directory_produce_the_same_payload() {
        let input = tempfile::tempdir().unwrap();
        local_fixture(input.path());
        let unpacked = tempfile::tempdir().unwrap();
        let source = Source::Local(input.path().into());
        prepare(source, unpacked.path(), &|_| {}).unwrap();

        let mut archive = tar::Builder::new(GzEncoder::new(
            File::create(input.path().join(ARCHIVE)).unwrap(),
            Compression::fast(),
        ));
        for name in FILES.into_iter().chain([SUMS]) {
            archive
                .append_path_with_name(
                    unpacked.path().join("payload").join(name),
                    format!("{PREFIX}/{name}"),
                )
                .unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
        let packed = tempfile::tempdir().unwrap();
        prepare(Source::Local(input.path().into()), packed.path(), &|_| {}).unwrap();
        let selected_archive = tempfile::tempdir().unwrap();
        prepare(
            Source::Local(input.path().join(ARCHIVE)),
            selected_archive.path(),
            &|_| {},
        )
        .unwrap();
        assert_eq!(
            fs::read(unpacked.path().join("payload/SHA256SUMS")).unwrap(),
            fs::read(packed.path().join("payload/SHA256SUMS")).unwrap(),
        );
        assert_eq!(
            fs::read(unpacked.path().join("payload/SHA256SUMS")).unwrap(),
            fs::read(selected_archive.path().join("payload/SHA256SUMS")).unwrap(),
        );
    }

    #[test]
    fn local_checksum_mismatch_stops_package_preparation() {
        let input = tempfile::tempdir().unwrap();
        local_fixture(input.path());
        let sha = checksum(&input.path().join("goodwin")).unwrap();
        fs::write(input.path().join(SUMS), format!("{sha}  goodwin\n")).unwrap();
        let mut binary = File::options()
            .append(true)
            .open(input.path().join("goodwin"))
            .unwrap();
        binary.write_all(b"changed").unwrap();
        drop(binary);
        let work = tempfile::tempdir().unwrap();
        let error = prepare(Source::Local(input.path().into()), work.path(), &|_| {}).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("SHA-256 не совпадает для goodwin")
        );
        assert!(!work.path().join("payload.tar.gz").exists());
    }

    #[test]
    #[ignore = "downloads the public Station package from GitHub"]
    fn latest_public_release_has_a_valid_station_package() {
        let work = tempfile::tempdir().unwrap();
        let (tag, _) = prepare(Source::Latest, work.path(), &|line| println!("{line}")).unwrap();
        assert!(release_tag(&tag));
        assert!(work.path().join("payload.tar.gz").is_file());
    }
}
