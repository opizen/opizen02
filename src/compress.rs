//! 압축하기: 파일·폴더를 zip / 7z / tar.gz / tar.xz 로 묶습니다.

use anyhow::{Context, Result, bail};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// 만들 수 있는 압축 형식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    SevenZ,
    TarGz,
    TarXz,
}

impl ArchiveKind {
    pub const ALL: [ArchiveKind; 4] = [Self::Zip, Self::SevenZ, Self::TarGz, Self::TarXz];

    /// 파일 확장자 (점 포함)
    pub fn ext(self) -> &'static str {
        match self {
            Self::Zip => ".zip",
            Self::SevenZ => ".7z",
            Self::TarGz => ".tar.gz",
            Self::TarXz => ".tar.xz",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Zip => "ZIP",
            Self::SevenZ => "7Z",
            Self::TarGz => "TAR.GZ",
            Self::TarXz => "TAR.XZ",
        }
    }

    /// 암호(AES-256)를 지원하는지
    pub fn supports_password(self) -> bool {
        matches!(self, Self::Zip | Self::SevenZ)
    }

    /// 출력 파일 이름으로 형식을 고릅니다.
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
            Some(Self::TarXz)
        } else if name.ends_with(".7z") {
            Some(Self::SevenZ)
        } else if name.ends_with(".zip") {
            Some(Self::Zip)
        } else {
            None
        }
    }
}

/// 압축 결과 통계.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompressStats {
    pub files: usize,
    pub dirs: usize,
    /// 원본 크기 합계
    pub bytes: u64,
    /// 건너뛴 항목(심볼릭 링크 등)에 대한 메시지
    pub warnings: Vec<String>,
    pub output: PathBuf,
}

struct Entry {
    /// 압축 파일 안의 이름 (`/` 구분)
    name: String,
    path: PathBuf,
    is_dir: bool,
}

/// 선택한 항목들로 만들 기본 압축 파일 경로.
///
/// 하나면 그 이름(`보고서.docx` → `보고서.zip`, `사진` 폴더 → `사진.zip`),
/// 여러 개면 들어 있는 폴더 이름을 씁니다. 이미 있으면 `(2)`를 붙입니다.
pub fn default_archive_path(inputs: &[PathBuf], kind: ArchiveKind) -> PathBuf {
    let first = inputs.first().cloned().unwrap_or_default();
    let parent = first.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = if inputs.len() == 1 {
        let name = first.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if first.is_dir() {
            name
        } else {
            match name.rfind('.') {
                Some(i) if i > 0 => name[..i].to_string(),
                _ => name,
            }
        }
    } else {
        fs::canonicalize(&parent)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "압축".into())
    };
    unique_archive_path(&parent.join(format!("{stem}{}", kind.ext())), kind)
}

/// `이름.tar.gz` 처럼 확장자가 두 단계인 경우도 `이름 (2).tar.gz`가 되도록 합니다.
fn unique_archive_path(path: &Path, kind: ArchiveKind) -> PathBuf {
    if fs::symlink_metadata(path).is_err() {
        return path.to_path_buf();
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = name.strip_suffix(kind.ext()).unwrap_or(&name).to_string();
    let parent = path.parent().unwrap_or(Path::new(""));
    (2..)
        .map(|n| parent.join(format!("{stem} ({n}){}", kind.ext())))
        .find(|p| fs::symlink_metadata(p).is_err())
        .expect("무한 반복자")
}

/// `inputs`(파일·폴더)를 `output`으로 압축합니다. 형식은 `kind`를 따릅니다.
///
/// 임시 파일(`*.part`)에 쓴 뒤 성공하면 이름을 바꾸므로, 실패해도 반쯤 만들어진
/// 압축 파일이 남지 않습니다.
pub fn compress(
    inputs: &[PathBuf],
    output: &Path,
    kind: ArchiveKind,
    password: Option<&str>,
) -> Result<CompressStats> {
    if inputs.is_empty() {
        bail!("압축할 파일이 없습니다");
    }
    let password = password.filter(|p| !p.is_empty());
    if password.is_some() && !kind.supports_password() {
        bail!("{} 형식은 암호를 지원하지 않습니다 (ZIP 또는 7Z를 사용하세요)", kind.label());
    }

    let mut stats = CompressStats { output: output.to_path_buf(), ..Default::default() };
    let output_abs = fs::canonicalize(output.parent().unwrap_or(Path::new(".")))
        .map(|p| p.join(output.file_name().unwrap_or_default()))
        .unwrap_or_else(|_| output.to_path_buf());
    let mut entries = Vec::new();
    for input in inputs {
        let name = input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .with_context(|| format!("이름이 없는 경로: {}", input.display()))?;
        collect(input, name, &output_abs, &mut entries, &mut stats)?;
    }

    let part = output.with_file_name(format!(
        "{}.part",
        output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    let result = match kind {
        ArchiveKind::Zip => write_zip(&entries, &part, password, &mut stats),
        ArchiveKind::SevenZ => write_7z(&entries, &part, password, &mut stats),
        ArchiveKind::TarGz => {
            let w = flate2::write::GzEncoder::new(create(&part)?, flate2::Compression::default());
            write_tar(&entries, w, &mut stats).and_then(|w| Ok(w.finish()?.flush()?))
        }
        ArchiveKind::TarXz => {
            let w = xz2::write::XzEncoder::new(create(&part)?, 6);
            write_tar(&entries, w, &mut stats).and_then(|w| Ok(w.finish()?.flush()?))
        }
    };
    if let Err(e) = result {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    fs::rename(&part, output).with_context(|| format!("저장 실패: {}", output.display()))?;
    Ok(stats)
}

fn create(path: &Path) -> Result<BufWriter<File>> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir)?;
    }
    Ok(BufWriter::new(
        File::create(path).with_context(|| format!("파일을 만들 수 없음: {}", path.display()))?,
    ))
}

/// 폴더를 재귀적으로 훑어 항목 목록을 만듭니다. 심볼릭 링크는 따라가지 않습니다.
fn collect(
    path: &Path,
    name: String,
    output: &Path,
    out: &mut Vec<Entry>,
    stats: &mut CompressStats,
) -> Result<()> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("읽을 수 없음: {}", path.display()))?;
    if meta.file_type().is_symlink() {
        stats.warnings.push(format!("심볼릭 링크는 건너뜀: {name}"));
        return Ok(());
    }
    if meta.is_dir() {
        out.push(Entry { name: name.clone(), path: path.to_path_buf(), is_dir: true });
        let mut children: Vec<_> = fs::read_dir(path)
            .with_context(|| format!("폴더를 읽을 수 없음: {}", path.display()))?
            .collect::<io::Result<_>>()?;
        children.sort_by_key(|c| c.file_name());
        for child in children {
            let child_name = format!("{name}/{}", child.file_name().to_string_lossy());
            collect(&child.path(), child_name, output, out, stats)?;
        }
    } else if meta.is_file() {
        // 만들고 있는 압축 파일 자신은 넣지 않음
        let abs = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let part = output.with_extension(format!(
            "{}.part",
            output.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
        ));
        if abs == output || abs == part {
            return Ok(());
        }
        out.push(Entry { name, path: path.to_path_buf(), is_dir: false });
    }
    Ok(())
}

fn zip_time(path: &Path) -> Option<zip::DateTime> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let utc = time::OffsetDateTime::from(modified);
    let local = time::UtcOffset::current_local_offset().map(|o| utc.to_offset(o)).unwrap_or(utc);
    zip::DateTime::try_from(time::PrimitiveDateTime::new(local.date(), local.time())).ok()
}

#[cfg(unix)]
fn unix_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).ok().map(|m| m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn unix_mode(_: &Path) -> Option<u32> {
    None
}

fn write_zip(entries: &[Entry], part: &Path, password: Option<&str>, stats: &mut CompressStats) -> Result<()> {
    use zip::write::SimpleFileOptions;
    let mut zip = zip::ZipWriter::new(create(part)?);
    for e in entries {
        let mut opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        if let Some(t) = zip_time(&e.path) {
            opts = opts.last_modified_time(t);
        }
        if let Some(m) = unix_mode(&e.path) {
            opts = opts.unix_permissions(m);
        }
        if e.is_dir {
            zip.add_directory(format!("{}/", e.name), opts)?;
            stats.dirs += 1;
            continue;
        }
        let size = fs::metadata(&e.path)?.len();
        opts = opts.large_file(size >= u32::MAX as u64);
        if let Some(pw) = password {
            opts = opts.with_aes_encryption(zip::AesMode::Aes256, pw);
        }
        zip.start_file(e.name.as_str(), opts)?;
        let mut f = File::open(&e.path).with_context(|| format!("열 수 없음: {}", e.path.display()))?;
        stats.bytes += io::copy(&mut f, &mut zip)?;
        stats.files += 1;
    }
    zip.finish()?.flush()?;
    Ok(())
}

fn write_7z(entries: &[Entry], part: &Path, password: Option<&str>, stats: &mut CompressStats) -> Result<()> {
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderMethod, Password, encoder_options::AesEncoderOptions};
    let mut w = ArchiveWriter::new(create(part)?).map_err(|e| anyhow::anyhow!("7z 만들기 실패: {e}"))?;
    if let Some(pw) = password {
        w.set_content_methods(vec![AesEncoderOptions::new(Password::from(pw)).into(), EncoderMethod::LZMA2.into()]);
        w.set_encrypt_header(true);
    }
    for e in entries {
        let entry = ArchiveEntry::from_path(&e.path, e.name.clone());
        if e.is_dir {
            w.push_archive_entry::<File>(entry, None).map_err(|x| anyhow::anyhow!("7z 쓰기 실패: {x}"))?;
            stats.dirs += 1;
        } else {
            let f = File::open(&e.path).with_context(|| format!("열 수 없음: {}", e.path.display()))?;
            let pushed = w
                .push_archive_entry(entry, Some(f))
                .map_err(|x| anyhow::anyhow!("7z 쓰기 실패: {}: {x}", e.name))?;
            stats.bytes += pushed.size;
            stats.files += 1;
        }
    }
    w.finish().map_err(|e| anyhow::anyhow!("7z 마무리 실패: {e}"))?.flush()?;
    Ok(())
}

fn write_tar<W: Write>(entries: &[Entry], w: W, stats: &mut CompressStats) -> Result<W> {
    let mut b = tar::Builder::new(w);
    b.follow_symlinks(false);
    for e in entries {
        b.append_path_with_name(&e.path, &e.name)
            .with_context(|| format!("추가 실패: {}", e.path.display()))?;
        if e.is_dir {
            stats.dirs += 1;
        } else {
            stats.files += 1;
            stats.bytes += fs::metadata(&e.path).map(|m| m.len()).unwrap_or(0);
        }
    }
    Ok(b.into_inner()?)
}
