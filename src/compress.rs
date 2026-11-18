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
    /// 만든 파일 (분할했으면 첫 조각 `.001`)
    pub output: PathBuf,
    /// 압축하지 않고 그대로 넣은 파일 수
    pub stored: usize,
    /// 분할한 조각 수 (나누지 않았으면 0)
    pub parts: usize,
    /// 만든 압축 파일 크기 (조각 합계)
    pub size: u64,
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

/// 압축 수준
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// 빠르게 (압축률 낮음)
    Fast,
    #[default]
    Normal,
    /// 최대 압축 (느림)
    Best,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Fast, Level::Normal, Level::Best];

    pub fn label(self) -> &'static str {
        match self {
            Level::Fast => "빠르게",
            Level::Normal => "보통",
            Level::Best => "최대",
        }
    }

    /// 1 / 6 / 9 (zip, gzip, xz, 7z 공통 단계)
    fn number(self) -> u32 {
        match self {
            Level::Fast => 1,
            Level::Normal => 6,
            Level::Best => 9,
        }
    }
}

/// 압축 옵션
#[derive(Debug, Clone, Default)]
pub struct CompressOptions {
    /// 암호 (zip, 7z — AES-256)
    pub password: Option<String>,
    pub level: Level,
    /// 진행률 / 취소
    pub progress: Option<std::sync::Arc<crate::Progress>>,
    /// `true`면 모든 파일을 압축합니다. 기본(`false`)은 이미 압축된 형식(jpg, mp4, zip …)이나
    /// 압축해도 작아지지 않는 파일을 압축 없이 그대로 넣어 시간을 아낍니다 (zip, 7z).
    pub always_compress: bool,
    /// 이 크기(바이트)마다 나눠 `이름.zip.001`, `.002` … 로 저장 (분할 압축)
    pub split: Option<u64>,
}

type Prog = Option<std::sync::Arc<crate::Progress>>;

/// 이미 압축된 형식 (다시 압축해도 거의 줄지 않음)
const PACKED_EXT: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "avif", "jxl", "mp3", "m4a", "aac", "ogg", "opus", "flac", "wma",
    "mp4", "m4v", "mkv", "avi", "mov", "webm", "wmv", "flv", "zip", "7z", "rar", "gz", "tgz", "xz", "txz", "bz2", "zst",
    "lz4", "lzma", "cab", "jar", "apk", "aab", "docx", "xlsx", "pptx", "odt", "ods", "odp", "epub", "hwpx", "woff",
    "woff2", "dmg", "iso",
];

/// 압축하지 않고 그대로 넣을 파일인지: 이미 압축된 형식이거나, 앞부분(최대 1 MiB)을
/// 압축해 봐도 2% 넘게 줄지 않으면 그대로 넣습니다.
pub fn is_incompressible(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if PACKED_EXT.contains(&ext.as_str()) {
        return true;
    }
    let Ok(f) = File::open(path) else { return false };
    let mut sample = Vec::new();
    if io::Read::read_to_end(&mut io::Read::take(f, 1 << 20), &mut sample).is_err() || sample.is_empty() {
        return false;
    }
    let mut packed = 0u64;
    {
        struct Count<'a>(&'a mut u64);
        impl Write for Count<'_> {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                *self.0 += b.len() as u64;
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut e = flate2::write::DeflateEncoder::new(Count(&mut packed), flate2::Compression::fast());
        if e.write_all(&sample).and_then(|_| e.finish().map(|_| ())).is_err() {
            return false;
        }
    }
    packed * 100 >= sample.len() as u64 * 98
}

/// `700m`, `4g`, `100k`, `1.5g`, `1048576` 같은 크기 글자를 바이트로 (1k = 1024)
pub fn parse_size(text: &str) -> Option<u64> {
    let t = text.trim().to_lowercase().replace(' ', "");
    let t = t.trim_end_matches('b').trim_end_matches('i');
    let (num, mul) = match t.chars().last()? {
        'k' => (&t[..t.len() - 1], 1u64 << 10),
        'm' => (&t[..t.len() - 1], 1 << 20),
        'g' => (&t[..t.len() - 1], 1 << 30),
        't' => (&t[..t.len() - 1], 1 << 40),
        _ => (t, 1),
    };
    let v: f64 = num.parse().ok()?;
    (v > 0.0 && v.is_finite()).then_some((v * mul as f64) as u64).filter(|&n| n > 0)
}

/// 다 만든 파일을 `size`바이트씩 `이름.001`, `.002` … 로 나눕니다. 나눈 조각 경로들을 돌려줍니다.
pub fn split_file(path: &Path, size: u64, progress: &Prog) -> Result<Vec<PathBuf>> {
    if size == 0 {
        bail!("분할 크기는 0보다 커야 합니다");
    }
    let total = fs::metadata(path)?.len();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let count = total.div_ceil(size).max(1);
    let parts: Vec<PathBuf> = (1..=count).map(|i| path.with_file_name(format!("{name}.{i:03}"))).collect();
    if let Some(p) = parts.iter().find(|p| p.exists()) {
        bail!("이미 있는 파일과 이름이 겹칩니다: {}", p.display());
    }
    let mut src = File::open(path)?;
    let result = (|| -> Result<()> {
        for part in &parts {
            if progress.as_ref().is_some_and(|p| p.is_cancelled()) {
                return Err(crate::Cancelled.into());
            }
            let mut out = BufWriter::new(File::create(part).with_context(|| format!("만들 수 없음: {}", part.display()))?);
            io::copy(&mut io::Read::take(&mut src, size), &mut out)?;
            out.flush()?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        for p in &parts {
            let _ = fs::remove_file(p);
        }
        return Err(e);
    }
    fs::remove_file(path)?;
    Ok(parts)
}

/// `inputs`(파일·폴더)를 `output`으로 압축합니다 (보통 수준). [`compress_with`] 참고.
pub fn compress(inputs: &[PathBuf], output: &Path, kind: ArchiveKind, password: Option<&str>) -> Result<CompressStats> {
    compress_with(inputs, output, kind, &CompressOptions { password: password.map(str::to_owned), ..Default::default() })
}

/// `inputs`(파일·폴더)를 `output`으로 압축합니다. 형식은 `kind`를 따릅니다.
///
/// 임시 파일(`*.part`)에 쓴 뒤 성공하면 이름을 바꾸므로, 실패하거나 취소해도 반쯤 만들어진
/// 압축 파일이 남지 않습니다.
pub fn compress_with(inputs: &[PathBuf], output: &Path, kind: ArchiveKind, opts: &CompressOptions) -> Result<CompressStats> {
    let password = opts.password.as_deref();
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

    if let Some(p) = &opts.progress {
        let total: u64 = entries.iter().filter(|e| !e.is_dir).filter_map(|e| fs::metadata(&e.path).ok()).map(|m| m.len()).sum();
        p.reset(total);
    }
    let level = opts.level.number();
    let progress = opts.progress.clone();
    let smart = !opts.always_compress;
    if opts.split == Some(0) {
        bail!("분할 크기는 0보다 커야 합니다");
    }

    let part = output.with_file_name(format!(
        "{}.part",
        output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    let result = match kind {
        ArchiveKind::Zip => write_zip(&entries, &part, password, level, smart, &progress, &mut stats),
        ArchiveKind::SevenZ => write_7z(&entries, &part, password, level, smart, &progress, &mut stats),
        ArchiveKind::TarGz => {
            let w = flate2::write::GzEncoder::new(create(&part)?, flate2::Compression::new(level));
            write_tar(&entries, w, &progress, &mut stats).and_then(|w| Ok(w.finish()?.flush()?))
        }
        ArchiveKind::TarXz => {
            let w = xz2::write::XzEncoder::new(create(&part)?, level);
            write_tar(&entries, w, &progress, &mut stats).and_then(|w| Ok(w.finish()?.flush()?))
        }
    };
    if let Err(e) = result {
        let _ = fs::remove_file(&part);
        // 라이브러리가 오류를 글자로 바꿔 버려도 취소였다면 취소로 알림
        if opts.progress.as_ref().is_some_and(|p| p.is_cancelled()) && !crate::is_cancelled(&e) {
            return Err(e.context(crate::Cancelled));
        }
        return Err(e);
    }
    fs::rename(&part, output).with_context(|| format!("저장 실패: {}", output.display()))?;
    stats.size = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    if let Some(size) = opts.split
        && fs::metadata(output).map(|m| m.len() > size).unwrap_or(false)
    {
        match split_file(output, size, &progress) {
            Ok(parts) => {
                stats.parts = parts.len();
                stats.output = parts[0].clone();
            }
            Err(e) => {
                let _ = fs::remove_file(output);
                return Err(e);
            }
        }
    }
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


/// 원본 파일을 읽으면서 진행률을 더하고 취소를 확인
fn source(path: &Path, progress: &Prog) -> Result<crate::Counted<File>> {
    if let Some(p) = progress {
        p.set_current(&path.file_name().unwrap_or_default().to_string_lossy());
    }
    let f = File::open(path).with_context(|| format!("열 수 없음: {}", path.display()))?;
    Ok(crate::Counted { inner: f, progress: progress.clone() })
}

fn write_zip(entries: &[Entry], part: &Path, password: Option<&str>, level: u32, smart: bool, progress: &Prog, stats: &mut CompressStats) -> Result<()> {
    let mut zip = zip::ZipWriter::new(create(part)?);
    for e in entries {
        zip_add(&mut zip, e, password, level, smart, progress, stats)?;
    }
    zip.finish()?.flush()?;
    Ok(())
}

/// zip에 항목 하나 넣기 (폴더 / 파일)
fn zip_add<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    e: &Entry,
    password: Option<&str>,
    level: u32,
    smart: bool,
    progress: &Prog,
    stats: &mut CompressStats,
) -> Result<()> {
    use zip::write::SimpleFileOptions;
    let mut opts = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(level as i64));
    if let Some(t) = zip_time(&e.path) {
        opts = opts.last_modified_time(t);
    }
    if let Some(m) = unix_mode(&e.path) {
        opts = opts.unix_permissions(m);
    }
    if e.is_dir {
        zip.add_directory(format!("{}/", e.name), opts)?;
        stats.dirs += 1;
        return Ok(());
    }
    let size = fs::metadata(&e.path)?.len();
    opts = opts.large_file(size >= u32::MAX as u64);
    if smart && is_incompressible(&e.path) {
        opts = opts.compression_method(zip::CompressionMethod::Stored).compression_level(None);
        stats.stored += 1;
    }
    if let Some(pw) = password {
        opts = opts.with_aes_encryption(zip::AesMode::Aes256, pw);
    }
    zip.start_file(e.name.as_str(), opts)?;
    let mut f = source(&e.path, progress)?;
    stats.bytes += io::copy(&mut f, zip)?;
    stats.files += 1;
    Ok(())
}

fn write_7z(entries: &[Entry], part: &Path, password: Option<&str>, level: u32, smart: bool, progress: &Prog, stats: &mut CompressStats) -> Result<()> {
    use sevenz_rust2::{
        ArchiveEntry, ArchiveWriter, Password,
        encoder_options::{AesEncoderOptions, Lzma2Options},
    };
    let mut w = ArchiveWriter::new(create(part)?).map_err(|e| anyhow::anyhow!("7z 만들기 실패: {e}"))?;
    let methods = |store: bool| {
        let main: sevenz_rust2::EncoderConfiguration =
            if store { sevenz_rust2::EncoderMethod::COPY.into() } else { Lzma2Options::from_level(level).into() };
        match password {
            Some(pw) => vec![AesEncoderOptions::new(Password::from(pw)).into(), main],
            None => vec![main],
        }
    };
    w.set_content_methods(methods(false));
    if password.is_some() {
        w.set_encrypt_header(true);
    }
    let mut storing = false;
    for e in entries {
        // 파일마다 따로 압축되므로, 압축이 안 되는 파일은 그대로(COPY) 넣음
        if !e.is_dir && smart {
            let store = is_incompressible(&e.path);
            if store != storing {
                w.set_content_methods(methods(store));
                storing = store;
            }
            if store {
                stats.stored += 1;
            }
        }
        let entry = ArchiveEntry::from_path(&e.path, e.name.clone());
        if e.is_dir {
            w.push_archive_entry::<File>(entry, None).map_err(|x| anyhow::anyhow!("7z 쓰기 실패: {x}"))?;
            stats.dirs += 1;
        } else {
            let f = source(&e.path, progress)?;
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

fn write_tar<W: Write>(entries: &[Entry], w: W, progress: &Prog, stats: &mut CompressStats) -> Result<W> {
    let mut b = tar::Builder::new(w);
    b.follow_symlinks(false);
    for e in entries {
        if e.is_dir {
            b.append_path_with_name(&e.path, &e.name).with_context(|| format!("추가 실패: {}", e.path.display()))?;
            stats.dirs += 1;
        } else {
            // 진행률·취소를 위해 파일 내용은 직접 읽어 넣음 (권한·수정 시간은 그대로)
            let meta = fs::metadata(&e.path).with_context(|| format!("읽을 수 없음: {}", e.path.display()))?;
            let mut header = tar::Header::new_gnu();
            header.set_metadata(&meta);
            header.set_size(meta.len());
            b.append_data(&mut header, &e.name, source(&e.path, progress)?)
                .with_context(|| format!("추가 실패: {}", e.path.display()))?;
            stats.files += 1;
            stats.bytes += meta.len();
        }
    }
    Ok(b.into_inner()?)
}

// ----- 압축 파일 고치기 (항목 추가 · 지우기) -----

/// 고칠 수 있는 압축 파일이면 그 형식 (zip · 7z · tar.gz · tar.xz, 분할되지 않은 것만)
pub fn editable_kind(archive: &Path) -> Option<ArchiveKind> {
    let kind = ArchiveKind::from_path(archive)?;
    (crate::archive_parts(archive).len() == 1).then_some(kind)
}

/// 압축 파일 고치기 옵션
#[derive(Debug, Clone, Default)]
pub struct EditOptions {
    /// 암호 (있는 압축 파일을 읽을 때, 그리고 새로 넣는 파일에)
    pub password: Option<String>,
    pub level: Level,
    pub always_compress: bool,
    pub progress: Option<std::sync::Arc<crate::Progress>>,
}

/// 고친 결과
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditStats {
    pub added: usize,
    pub removed: usize,
}

/// `name`이 지울 목록의 항목이거나 그 안에 있는지
fn under(name: &str, list: &[String]) -> bool {
    let n = name.trim_end_matches('/');
    list.iter().any(|r| {
        let r = r.trim_end_matches('/');
        n == r || n.strip_prefix(r).is_some_and(|rest| rest.starts_with('/'))
    })
}

/// 압축 파일 고치기: `remove`(압축 파일 안 경로, 폴더면 그 안 전부)를 지우고,
/// `add`(파일·폴더)를 압축 파일 안 `prefix` 폴더에 넣습니다. 같은 이름의 파일은 새것으로 바뀝니다.
///
/// zip은 남길 항목을 다시 압축하지 않고 그대로 옮겨 빠르고, 암호가 걸린 항목도 유지됩니다.
/// 7z · tar.gz · tar.xz는 임시 폴더에 모두 푼 뒤 다시 압축합니다. 실패하면 원래 파일은 그대로입니다.
pub fn edit_archive(archive: &Path, remove: &[String], add: &[PathBuf], prefix: &str, opts: &EditOptions) -> Result<EditStats> {
    let Some(kind) = editable_kind(archive) else {
        bail!("이 형식은 고칠 수 없습니다 (zip · 7z · tar.gz · tar.xz만, 분할 압축과 RAR은 안 됨)");
    };
    if remove.is_empty() && add.is_empty() {
        return Ok(EditStats::default());
    }
    let prefix = prefix.trim_matches('/');
    let mut stats = CompressStats::default();
    let mut new_entries = Vec::new();
    let archive_abs = fs::canonicalize(archive).unwrap_or_else(|_| archive.to_path_buf());
    for p in add {
        let base = p.file_name().map(|n| n.to_string_lossy().into_owned()).with_context(|| format!("이름이 없는 경로: {}", p.display()))?;
        let name = if prefix.is_empty() { base } else { format!("{prefix}/{base}") };
        collect(p, name, &archive_abs, &mut new_entries, &mut stats)?;
    }
    if let Some(pr) = &opts.progress {
        let total: u64 = new_entries.iter().filter(|e| !e.is_dir).filter_map(|e| fs::metadata(&e.path).ok()).map(|m| m.len()).sum();
        pr.reset(total);
    }
    let result = if kind == ArchiveKind::Zip {
        edit_zip(archive, remove, &new_entries, opts)
    } else {
        edit_by_rebuild(archive, kind, remove, add, prefix, opts)
    };
    if let Err(e) = result {
        if opts.progress.as_ref().is_some_and(|p| p.is_cancelled()) && !crate::is_cancelled(&e) {
            return Err(e.context(crate::Cancelled));
        }
        return Err(e);
    }
    let (removed, _) = result?;
    Ok(EditStats { added: new_entries.iter().filter(|e| !e.is_dir).count(), removed })
}

fn part_of(archive: &Path) -> PathBuf {
    archive.with_file_name(format!("{}.part", archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
}

fn edit_zip(archive: &Path, remove: &[String], new_entries: &[Entry], opts: &EditOptions) -> Result<(usize, ())> {
    let mut src = zip::ZipArchive::new(File::open(archive).with_context(|| format!("열 수 없음: {}", archive.display()))?)
        .context("zip 읽기 실패")?;
    let part = part_of(archive);
    let password = opts.password.as_deref().filter(|p| !p.is_empty());
    let mut stats = CompressStats::default();
    let replaced: std::collections::HashSet<&str> = new_entries.iter().map(|e| e.name.as_str()).collect();
    let r = (|| -> Result<usize> {
        let mut w = zip::ZipWriter::new(create(&part)?);
        let mut removed = 0;
        for i in 0..src.len() {
            let f = src.by_index_raw(i)?;
            let name = crate::zip_entry_name(&f);
            let bare = name.trim_end_matches('/');
            if under(&name, remove) {
                if !f.is_dir() {
                    removed += 1;
                }
                continue;
            }
            if replaced.contains(bare) {
                continue; // 새것으로 바뀜
            }
            w.raw_copy_file(f)?;
        }
        for e in new_entries {
            zip_add(&mut w, e, password, opts.level.number(), !opts.always_compress, &opts.progress, &mut stats)?;
        }
        w.finish()?.flush()?;
        Ok(removed)
    })();
    match r {
        Ok(n) => {
            drop(src);
            fs::rename(&part, archive).with_context(|| format!("저장 실패: {}", archive.display()))?;
            Ok((n, ()))
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

/// 7z · tar.*: 임시 폴더에 모두 풀고 → 지우고/넣고 → 다시 압축
fn edit_by_rebuild(archive: &Path, kind: ArchiveKind, remove: &[String], add: &[PathBuf], prefix: &str, opts: &EditOptions) -> Result<(usize, ())> {
    let parent = archive.parent().unwrap_or(Path::new("."));
    let tmp = crate::unique_path(&parent.join(format!(".opizen-edit-{}", std::process::id())));
    fs::create_dir_all(&tmp)?;
    let r = (|| -> Result<usize> {
        let x = crate::Options { password: opts.password.clone(), allow_large: true, overwrite: true, ..Default::default() };
        crate::extract(archive, Some(&tmp), &x)?;
        let mut removed = 0;
        for name in remove {
            let p = tmp.join(name.trim_matches('/'));
            if !p.starts_with(&tmp) || name.split('/').any(|s| s == "..") {
                continue;
            }
            match fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => {
                    removed += walk_files(&p);
                    fs::remove_dir_all(&p)?;
                }
                Ok(_) => {
                    fs::remove_file(&p)?;
                    removed += 1;
                }
                Err(_) => {}
            }
        }
        let dest = if prefix.is_empty() { tmp.clone() } else { tmp.join(prefix) };
        fs::create_dir_all(&dest)?;
        for a in add {
            if let Some(name) = a.file_name() {
                copy_over(a, &dest.join(name))?;
            }
        }
        let mut inputs: Vec<PathBuf> = fs::read_dir(&tmp)?.flatten().map(|e| e.path()).collect();
        inputs.sort();
        if inputs.is_empty() {
            bail!("항목을 모두 지울 수는 없습니다 — 압축 파일 자체를 휴지통으로 보내세요");
        }
        let c = CompressOptions {
            password: opts.password.clone().filter(|p| !p.is_empty() && kind.supports_password()),
            level: opts.level,
            progress: opts.progress.clone(),
            always_compress: opts.always_compress,
            split: None,
        };
        compress_with(&inputs, archive, kind, &c)?;
        Ok(removed)
    })();
    let _ = fs::remove_dir_all(&tmp);
    r.map(|n| (n, ()))
}

fn walk_files(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| if e.path().is_dir() { walk_files(&e.path()) } else { 1 }).sum())
        .unwrap_or(0)
}

/// 덮어쓰며 복사 (폴더는 합침)
fn copy_over(src: &Path, dest: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        if dest.is_file() {
            fs::remove_file(dest)?;
        }
        fs::create_dir_all(dest)?;
        for e in fs::read_dir(src)?.flatten() {
            copy_over(&e.path(), &dest.join(e.file_name()))?;
        }
    } else {
        if dest.is_dir() {
            fs::remove_dir_all(dest)?;
        }
        fs::copy(src, dest).with_context(|| format!("복사 실패: {}", src.display()))?;
        if let Ok(t) = meta.modified() {
            let _ = File::options().write(true).open(dest).and_then(|f| f.set_modified(t));
        }
    }
    Ok(())
}
