//! `unpack` 라이브러리: 여러 압축 형식을 안전하게 해제합니다.
//!
//! 지원 형식: zip, 7z, rar(풀기만), tar, tar.gz/tgz, tar.bz2/tbz2, tar.xz/txz, tar.zst/tzst,
//! 단일 스트림 gz / bz2 / xz / zst.
//!
//! 모든 항목 경로는 대상 폴더 밖으로 나갈 수 없도록 검사됩니다(Zip Slip 방지).

use anyhow::{Context, Result, bail};

mod compress;
pub use compress::{ArchiveKind, CompressStats, compress, default_archive_path};

/// 프로그램 이름
pub const APP_NAME: &str = "opizenAI_Unpack";
use std::fs::{self, File};
use std::io::{self, BufReader, Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 압축 스트림 코덱.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Gz,
    Bz2,
    Xz,
    Zst,
}

/// 압축 파일 형식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    SevenZ,
    /// RAR (풀기만 지원, 분할 압축 포함)
    Rar,
    /// tar (선택적으로 압축됨)
    Tar(Option<Codec>),
    /// 단일 압축 스트림. 내용이 tar이면 자동으로 tar로 처리합니다.
    Stream(Codec),
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Format::Zip => "zip",
            Format::SevenZ => "7z",
            Format::Rar => "rar",
            Format::Tar(None) => "tar",
            Format::Tar(Some(Codec::Gz)) => "tar.gz",
            Format::Tar(Some(Codec::Bz2)) => "tar.bz2",
            Format::Tar(Some(Codec::Xz)) => "tar.xz",
            Format::Tar(Some(Codec::Zst)) => "tar.zst",
            Format::Stream(Codec::Gz) => "gz",
            Format::Stream(Codec::Bz2) => "bz2",
            Format::Stream(Codec::Xz) => "xz",
            Format::Stream(Codec::Zst) => "zst",
        };
        f.write_str(s)
    }
}

/// 해제 옵션.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// 이미 존재하는 파일을 덮어쓸지 여부
    pub overwrite: bool,
    /// 암호 (zip, 7z)
    pub password: Option<String>,
    /// 해제되는 각 항목 이름 출력
    pub verbose: bool,
}

/// 해제 결과 통계.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub files: usize,
    pub dirs: usize,
    pub skipped: usize,
    pub bytes: u64,
    /// 건너뛴 항목 등에 대한 경고 메시지
    pub warnings: Vec<String>,
    /// 실제로 사용된 출력 경로
    pub output: PathBuf,
}

/// 목록 표시용 항목 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// 암호가 없거나 틀려서 해제할 수 없을 때의 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordError {
    /// 암호를 입력했지만 틀린 경우 `true`
    pub wrong: bool,
}

impl std::fmt::Display for PasswordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.wrong { "암호가 올바르지 않습니다" } else { "암호가 필요합니다" })
    }
}

impl std::error::Error for PasswordError {}

/// 오류가 암호 문제(없음/틀림)인지 확인합니다.
pub fn is_password_error(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<PasswordError>())
}

/// 우클릭 메뉴 등에서 쓰는 해제 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 알아서 풀기: 최상위 항목이 하나면 그대로, 여러 개면 압축 파일 이름의 폴더에 해제
    Smart,
    /// 여기에 풀기: 폴더를 만들지 않고 그대로 해제 (이미 있는 파일은 건너뜀)
    Here,
    /// 항상 압축 파일 이름의 새 폴더에 해제
    NewFolder,
}

// ---------------------------------------------------------------------------
// 형식 감지
// ---------------------------------------------------------------------------

/// 확장자와 매직 바이트로 형식을 감지합니다.
pub fn detect(path: &Path) -> Result<Format> {
    if let Some(f) = detect_by_name(path) {
        return Ok(f);
    }
    let mut head = [0u8; 512];
    let mut file = File::open(path).with_context(|| format!("열 수 없음: {}", path.display()))?;
    let n = read_up_to(&mut file, &mut head)?;
    detect_by_magic(&head[..n])
        .with_context(|| format!("지원하지 않거나 알 수 없는 형식: {}", path.display()))
}

fn detect_by_name(path: &Path) -> Option<Format> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let table: &[(&str, Format)] = &[
        (".tar.gz", Format::Tar(Some(Codec::Gz))),
        (".tgz", Format::Tar(Some(Codec::Gz))),
        (".tar.bz2", Format::Tar(Some(Codec::Bz2))),
        (".tbz2", Format::Tar(Some(Codec::Bz2))),
        (".tbz", Format::Tar(Some(Codec::Bz2))),
        (".tar.xz", Format::Tar(Some(Codec::Xz))),
        (".txz", Format::Tar(Some(Codec::Xz))),
        (".tar.zst", Format::Tar(Some(Codec::Zst))),
        (".tzst", Format::Tar(Some(Codec::Zst))),
        (".tar", Format::Tar(None)),
        (".zip", Format::Zip),
        (".jar", Format::Zip),
        (".7z", Format::SevenZ),
        (".rar", Format::Rar),
        (".gz", Format::Stream(Codec::Gz)),
        (".bz2", Format::Stream(Codec::Bz2)),
        (".xz", Format::Stream(Codec::Xz)),
        (".zst", Format::Stream(Codec::Zst)),
    ];
    table
        .iter()
        .find(|(ext, _)| name.ends_with(ext) && name.len() > ext.len())
        .map(|(_, f)| *f)
}

fn detect_by_magic(head: &[u8]) -> Option<Format> {
    let starts = |m: &[u8]| head.starts_with(m);
    if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") {
        Some(Format::Zip)
    } else if starts(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        Some(Format::SevenZ)
    } else if starts(b"Rar!\x1a\x07") {
        Some(Format::Rar)
    } else if starts(&[0x1F, 0x8B]) {
        Some(Format::Stream(Codec::Gz))
    } else if starts(b"BZh") {
        Some(Format::Stream(Codec::Bz2))
    } else if starts(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        Some(Format::Stream(Codec::Xz))
    } else if starts(&[0x28, 0xB5, 0x2F, 0xFD]) {
        Some(Format::Stream(Codec::Zst))
    } else if is_tar_header(head) {
        Some(Format::Tar(None))
    } else {
        None
    }
}

fn is_tar_header(head: &[u8]) -> bool {
    head.len() >= 262 && &head[257..262] == b"ustar"
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

// ---------------------------------------------------------------------------
// 경로 안전성
// ---------------------------------------------------------------------------

/// 압축 파일 내부 경로를 대상 폴더 기준의 안전한 상대 경로로 변환합니다.
/// 절대 경로, `..`, 드라이브 접두사 등이 있으면 `None`을 반환합니다.
pub fn sanitize(name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    let mut out = PathBuf::new();
    for comp in Path::new(&normalized).components() {
        match comp {
            Component::Normal(c) => {
                // Windows 드라이브 문자(C:) 형태 차단
                if out.as_os_str().is_empty() && c.to_string_lossy().ends_with(':') {
                    return None;
                }
                out.push(c);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if out.as_os_str().is_empty() { None } else { Some(out) }
}

/// 심볼릭 링크 대상이 대상 폴더 안에 머무는지 경로상으로 검사합니다.
fn symlink_target_is_safe(link_rel: &Path, target: &str) -> bool {
    let target = Path::new(target);
    if target.is_absolute() || target.to_string_lossy().contains('\\') {
        return false;
    }
    let mut depth: isize = link_rel.components().count() as isize - 1;
    for comp in target.components() {
        match comp {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// 공통 쓰기 도우미
// ---------------------------------------------------------------------------

struct Sink<'a> {
    root: PathBuf,
    opts: &'a Options,
    stats: Stats,
}

enum Prepared {
    Write(PathBuf),
    Skip,
}

impl<'a> Sink<'a> {
    fn new(root: &Path, opts: &'a Options) -> Result<Self> {
        fs::create_dir_all(root)
            .with_context(|| format!("출력 폴더를 만들 수 없음: {}", root.display()))?;
        Ok(Self {
            root: root.to_path_buf(),
            opts,
            stats: Stats { output: root.to_path_buf(), ..Default::default() },
        })
    }

    fn warn(&mut self, msg: String) {
        self.stats.warnings.push(msg);
        self.stats.skipped += 1;
    }

    fn warn_unsafe(&mut self, name: &str) {
        self.warn(format!("안전하지 않은 경로를 건너뜀: {name}"));
    }

    fn dir(&mut self, rel: &Path) -> Result<()> {
        let path = self.root.join(rel);
        fs::create_dir_all(&path)
            .with_context(|| format!("폴더 생성 실패: {}", path.display()))?;
        self.stats.dirs += 1;
        if self.opts.verbose {
            println!("  {}/", rel.display());
        }
        Ok(())
    }

    /// 파일을 쓸 준비: 부모 폴더 생성, 덮어쓰기 정책 적용, 기존 심볼릭 링크 제거.
    fn prepare(&mut self, rel: &Path) -> Result<Prepared> {
        let path = self.root.join(rel);
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !self.opts.overwrite {
                self.warn(format!("이미 존재하여 건너뜀: {}", rel.display()));
                return Ok(Prepared::Skip);
            }
            if meta.is_dir() && !meta.file_type().is_symlink() {
                bail!("같은 이름의 폴더가 이미 존재함: {}", path.display());
            }
            // 기존 심볼릭 링크를 통해 바깥에 쓰는 일을 막기 위해 먼저 삭제
            fs::remove_file(&path)
                .with_context(|| format!("기존 파일 삭제 실패: {}", path.display()))?;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("폴더 생성 실패: {}", parent.display()))?;
        }
        Ok(Prepared::Write(path))
    }

    fn file(&mut self, rel: &Path, reader: &mut dyn Read, mode: Option<u32>, mtime: Option<SystemTime>) -> Result<()> {
        let Prepared::Write(path) = self.prepare(rel)? else {
            return Ok(());
        };
        let mut out =
            File::create(&path).with_context(|| format!("파일 생성 실패: {}", path.display()))?;
        let n = io::copy(reader, &mut out)
            .with_context(|| format!("쓰기 실패: {}", path.display()))?;
        // 압축 파일에 기록된 수정 시간을 그대로 유지
        if let Some(t) = mtime {
            let _ = out.set_modified(t);
        }
        drop(out);
        set_mode(&path, mode);
        self.stats.files += 1;
        self.stats.bytes += n;
        if self.opts.verbose {
            println!("  {}", rel.display());
        }
        Ok(())
    }

    fn symlink(&mut self, rel: &Path, target: &str) -> Result<()> {
        if !symlink_target_is_safe(rel, target) {
            self.warn(format!("폴더 밖을 가리키는 심볼릭 링크를 건너뜀: {} -> {target}", rel.display()));
            return Ok(());
        }
        let Prepared::Write(path) = self.prepare(rel)? else {
            return Ok(());
        };
        make_symlink(target, &path)?;
        self.stats.files += 1;
        if self.opts.verbose {
            println!("  {} -> {target}", rel.display());
        }
        Ok(())
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(m) = mode {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(m & 0o777));
    }
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: Option<u32>) {}

#[cfg(unix)]
fn make_symlink(target: &str, path: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, path)
        .with_context(|| format!("심볼릭 링크 생성 실패: {}", path.display()))
}

#[cfg(not(unix))]
fn make_symlink(target: &str, path: &Path) -> Result<()> {
    // 심볼릭 링크를 지원하지 않는 플랫폼에서는 대상 경로를 텍스트로 저장
    fs::write(path, target).with_context(|| format!("파일 생성 실패: {}", path.display()))
}

// ---------------------------------------------------------------------------
// 해제
// ---------------------------------------------------------------------------

/// 압축 파일 이름에서 확장자를 뗀 기본 이름을 반환합니다.
pub fn base_name(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_lowercase();
    // 분할 RAR: 이름.part1.rar → 이름
    if let Some(stem) = lower.strip_suffix(".rar").filter(|s| !s.is_empty()) {
        let mut keep = stem.len();
        if let Some(i) = stem.rfind(".part")
            && i > 0
            && stem[i + 5..].chars().all(|c| c.is_ascii_digit())
            && stem.len() > i + 5
        {
            keep = i;
        }
        return name[..keep].to_string();
    }
    let exts = [
        ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".tbz", ".txz", ".tzst",
        ".tar", ".zip", ".jar", ".7z", ".gz", ".bz2", ".xz", ".zst",
    ];
    for ext in exts {
        if lower.ends_with(ext) && lower.len() > ext.len() {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    format!("{name}.out")
}

/// 압축 파일을 해제합니다.
///
/// `dest`가 `None`이면 압축 파일 옆에 기본 이름으로 폴더를 만듭니다
/// (단일 파일 스트림은 압축 파일과 같은 폴더에 풉니다).
pub fn extract(archive: &Path, dest: Option<&Path>, opts: &Options) -> Result<Stats> {
    let parent = archive.parent().map(Path::to_path_buf).unwrap_or_default();
    extract_impl(archive, dest, parent, opts)
}

/// `parent` 폴더 안에 압축 파일 이름으로 된 폴더를 만들어 해제합니다
/// (단일 파일 스트림은 `parent` 바로 아래에 파일을 만듭니다).
pub fn extract_into(archive: &Path, parent: &Path, opts: &Options) -> Result<Stats> {
    extract_impl(archive, None, parent.to_path_buf(), opts)
}

fn extract_impl(archive: &Path, dest: Option<&Path>, parent: PathBuf, opts: &Options) -> Result<Stats> {
    let format = detect(archive)?;
    let default_dir = || parent.join(base_name(archive));

    match format {
        Format::Zip => extract_zip(archive, &dest.map(PathBuf::from).unwrap_or_else(default_dir), opts),
        Format::SevenZ => {
            extract_7z(archive, &dest.map(PathBuf::from).unwrap_or_else(default_dir), opts)
        }
        Format::Rar => extract_rar(archive, &dest.map(PathBuf::from).unwrap_or_else(default_dir), opts),
        Format::Tar(codec) => {
            let reader = decoder(codec, open_buffered(archive)?)?;
            extract_tar(reader, &dest.map(PathBuf::from).unwrap_or_else(default_dir), opts)
        }
        Format::Stream(codec) => {
            let mut reader = decoder(Some(codec), open_buffered(archive)?)?;
            let mut head = vec![0u8; 512];
            let n = read_up_to(&mut reader, &mut head)?;
            head.truncate(n);
            let is_tar = is_tar_header(&head);
            let mut full = Cursor::new(head).chain(reader);
            if is_tar {
                extract_tar(Box::new(full), &dest.map(PathBuf::from).unwrap_or_else(default_dir), opts)
            } else {
                let root = dest.map(PathBuf::from).unwrap_or(parent.clone());
                write_plain(&mut full, &root, &base_name(archive), opts)
            }
        }
    }
}

/// 압축되지 않은 단일 파일 내용을 `root/name`에 씁니다.
fn write_plain(reader: &mut dyn Read, root: &Path, name: &str, opts: &Options) -> Result<Stats> {
    let root = if root.as_os_str().is_empty() { PathBuf::from(".") } else { root.to_path_buf() };
    let mut sink = Sink::new(&root, opts)?;
    let rel = PathBuf::from(name);
    sink.file(&rel, reader, None, None)?;
    sink.stats.output = root.join(rel);
    Ok(sink.stats)
}

/// 단일 파일 스트림(.gz 등)이면서 내용이 tar가 아닌지 확인합니다.
fn plain_stream(archive: &Path) -> Result<Option<Box<dyn Read>>> {
    let Format::Stream(codec) = detect(archive)? else { return Ok(None) };
    let mut reader = decoder(Some(codec), open_buffered(archive)?)?;
    let mut head = vec![0u8; 512];
    let n = read_up_to(&mut reader, &mut head)?;
    head.truncate(n);
    if is_tar_header(&head) {
        Ok(None)
    } else {
        Ok(Some(Box::new(Cursor::new(head).chain(reader))))
    }
}

/// `path`가 이미 있으면 `이름 (2)`, `이름 (3)` … 처럼 겹치지 않는 경로를 돌려줍니다.
pub fn unique_path(path: &Path) -> PathBuf {
    if fs::symlink_metadata(path).is_err() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or(Path::new(""));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // 확장자는 파일 이름 앞부분이 아닌 마지막 점 뒤만 분리 (".bashrc"는 확장자 없음)
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && !path.is_dir() => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.clone(), String::new()),
    };
    (2..)
        .map(|n| parent.join(format!("{stem} ({n}){ext}")))
        .find(|p| fs::symlink_metadata(p).is_err())
        .expect("무한 반복자")
}

/// 우클릭 메뉴용 해제: 기존 파일을 덮어쓰지 않고, 이름이 겹치면 `(2)`를 붙입니다.
///
/// - [`Mode::Here`]: `parent`에 그대로 풉니다. 이미 있는 파일은 덮어쓰지 않고 건너뜁니다.
/// - [`Mode::Smart`]: 임시 폴더에 푼 뒤, 최상위 항목이 하나뿐이면 `parent`로 바로 옮기고
///   여러 개면 압축 파일 이름의 폴더로 정리합니다.
/// - [`Mode::NewFolder`]: 항상 압축 파일 이름의 새 폴더에 풉니다.
///
/// 단일 파일 스트림(.gz 등)은 두 방식 모두 `parent`에 파일 하나를 만듭니다.
pub fn extract_auto(archive: &Path, parent: &Path, mode: Mode, opts: &Options) -> Result<Stats> {
    let base = base_name(archive);
    if let Some(mut reader) = plain_stream(archive)? {
        let target = unique_path(&parent.join(&base));
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(base);
        return write_plain(&mut reader, parent, &name, opts);
    }

    if mode == Mode::Here {
        let mut stats = extract(archive, Some(parent), opts)?;
        stats.output = parent.to_path_buf();
        return Ok(stats);
    }

    if mode == Mode::NewFolder {
        let dest = unique_path(&parent.join(&base));
        return extract(archive, Some(&dest), opts);
    }

    // 스마트 해제: 같은 드라이브의 임시 폴더에 풀고 이름만 바꿔 옮깁니다(복사 없음).
    let tmp = (0..)
        .map(|n| parent.join(format!(".unpack-tmp-{}-{n}", std::process::id())))
        .find(|p| fs::symlink_metadata(p).is_err())
        .expect("무한 반복자");
    let result = extract(archive, Some(&tmp), opts).and_then(|mut stats| {
        let mut entries: Vec<_> = fs::read_dir(&tmp)?.collect::<io::Result<_>>()?;
        let single = (entries.len() == 1).then(|| entries.remove(0));
        let output = match single {
            Some(entry) if fs::symlink_metadata(parent.join(entry.file_name())).is_err() => {
                let target = parent.join(entry.file_name());
                fs::rename(entry.path(), &target)
                    .with_context(|| format!("이동 실패: {}", target.display()))?;
                fs::remove_dir(&tmp)?;
                target
            }
            _ => {
                let target = unique_path(&parent.join(&base));
                fs::rename(&tmp, &target)
                    .with_context(|| format!("이동 실패: {}", target.display()))?;
                target
            }
        };
        stats.output = output;
        Ok(stats)
    });
    if result.is_err() && tmp.exists() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

fn open_buffered(path: &Path) -> Result<BufReader<File>> {
    let f = File::open(path).with_context(|| format!("열 수 없음: {}", path.display()))?;
    Ok(BufReader::new(f))
}

fn decoder(codec: Option<Codec>, r: BufReader<File>) -> Result<Box<dyn Read>> {
    Ok(match codec {
        None => Box::new(r),
        Some(Codec::Gz) => Box::new(flate2::read::MultiGzDecoder::new(r)),
        Some(Codec::Bz2) => Box::new(bzip2::read::MultiBzDecoder::new(r)),
        Some(Codec::Xz) => Box::new(xz2::read::XzDecoder::new_multi_decoder(r)),
        Some(Codec::Zst) => Box::new(zstd::stream::read::Decoder::with_buffer(r)?),
    })
}

fn extract_tar(reader: Box<dyn Read>, root: &Path, opts: &Options) -> Result<Stats> {
    let mut sink = Sink::new(root, opts)?;
    let mut ar = tar::Archive::new(reader);
    ar.set_overwrite(true);
    for entry in ar.entries().context("tar 읽기 실패")? {
        let mut entry = entry.context("tar 항목 읽기 실패")?;
        let raw = entry.path()?.to_string_lossy().into_owned();
        let Some(rel) = sanitize(&raw) else {
            sink.warn_unsafe(&raw);
            continue;
        };
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            sink.dir(&rel)?;
            continue;
        }
        if kind.is_symlink() {
            let target = entry
                .link_name()?
                .map(|t| t.to_string_lossy().into_owned())
                .unwrap_or_default();
            sink.symlink(&rel, &target)?;
            continue;
        }
        if !(kind.is_file() || kind.is_hard_link() || kind.is_contiguous()) {
            // 장치 파일, FIFO 등은 건너뜀 (pax/gnu 확장 헤더는 tar 크레이트가 처리)
            continue;
        }
        if kind.is_hard_link() {
            // 하드 링크는 tar 크레이트의 안전 검사(unpack_in)에 맡깁니다.
            if matches!(sink.prepare(&rel)?, Prepared::Skip) {
                continue;
            }
            if !entry.unpack_in(&sink.root)? {
                sink.warn_unsafe(&raw);
            } else {
                sink.stats.files += 1;
            }
            continue;
        }
        let mode = entry.header().mode().ok();
        let mtime = entry.header().mtime().ok().filter(|&t| t > 0).map(|t| UNIX_EPOCH + Duration::from_secs(t));
        sink.file(&rel, &mut entry, mode, mtime)?;
    }
    Ok(sink.stats)
}

fn extract_zip(path: &Path, root: &Path, opts: &Options) -> Result<Stats> {
    let mut zip = zip::ZipArchive::new(open_buffered(path)?).context("zip 읽기 실패")?;
    let mut sink = Sink::new(root, opts)?;
    for i in 0..zip.len() {
        let mut file = match &opts.password {
            Some(pw) => zip.by_index_decrypt(i, pw.as_bytes()),
            None => zip.by_index(i),
        }
        .map_err(|e| match e {
            zip::result::ZipError::UnsupportedArchive(m) if m == zip::result::ZipError::PASSWORD_REQUIRED => {
                anyhow::Error::new(PasswordError { wrong: false })
            }
            zip::result::ZipError::InvalidPassword => anyhow::Error::new(PasswordError { wrong: true }),
            e => anyhow::Error::new(e).context(format!("zip 항목 #{i} 읽기 실패")),
        })?;
        let raw = zip_entry_name(&file);
        let Some(rel) = sanitize(&raw) else {
            sink.warn_unsafe(&raw);
            continue;
        };
        if file.is_dir() {
            sink.dir(&rel)?;
        } else if file.is_symlink() {
            let mut target = String::new();
            file.read_to_string(&mut target)?;
            sink.symlink(&rel, &target)?;
        } else {
            let mode = file.unix_mode();
            let mtime = file.last_modified().and_then(zip_time_to_system);
            sink.file(&rel, &mut file, mode, mtime)?;
        }
    }
    Ok(sink.stats)
}

/// zip 항목 이름. UTF-8이 아니면(한국어 Windows에서 만든 zip 등) CP949로 읽습니다.
fn zip_entry_name<R: Read>(file: &zip::read::ZipFile<'_, R>) -> String {
    decode_zip_name(file.name_raw()).unwrap_or_else(|| file.name().map(|n| n.into_owned()).unwrap_or_default())
}

/// 원본 바이트를 UTF-8 → CP949 순서로 해석. 둘 다 아니면 `None`(zip 크레이트의 CP437 해석 사용).
pub fn decode_zip_name(raw: &[u8]) -> Option<String> {
    if let Ok(s) = std::str::from_utf8(raw) {
        return Some(s.to_owned());
    }
    let (s, _, had_errors) = encoding_rs::EUC_KR.decode(raw);
    (!had_errors).then(|| s.into_owned())
}

/// zip의 날짜(현지 시각 기준)를 시스템 시간으로
fn zip_time_to_system(t: zip::DateTime) -> Option<SystemTime> {
    let pdt = time::PrimitiveDateTime::try_from(t).ok()?;
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    Some(SystemTime::from(pdt.assume_offset(offset)))
}

fn sevenz_password(opts: &Options) -> sevenz_rust2::Password {
    match &opts.password {
        Some(p) => sevenz_rust2::Password::from(p.as_str()),
        None => sevenz_rust2::Password::empty(),
    }
}

fn sevenz_error(e: sevenz_rust2::Error, what: &str, opts: &Options) -> anyhow::Error {
    match e {
        sevenz_rust2::Error::PasswordRequired => anyhow::Error::new(PasswordError { wrong: false }),
        sevenz_rust2::Error::MaybeBadPassword(_) => {
            anyhow::Error::new(PasswordError { wrong: opts.password.is_some() })
        }
        e => anyhow::anyhow!("7z {what} 실패: {e}"),
    }
}

fn extract_7z(path: &Path, root: &Path, opts: &Options) -> Result<Stats> {
    let mut reader = sevenz_rust2::ArchiveReader::open(path, sevenz_password(opts))
        .map_err(|e| sevenz_error(e, "열기", opts))?;
    let mut sink = Sink::new(root, opts)?;
    let mut failure: Option<anyhow::Error> = None;
    let result = reader.for_each_entries(|entry, data| {
        if entry.is_anti_item() {
            return Ok(true);
        }
        let Some(rel) = sanitize(entry.name()) else {
            sink.warn_unsafe(entry.name());
            return Ok(true);
        };
        let r = if entry.is_directory() {
            sink.dir(&rel)
        } else {
            let mtime = entry.has_last_modified_date.then(|| SystemTime::from(entry.last_modified_date()));
            sink.file(&rel, data, None, mtime)
        };
        if let Err(e) = r {
            failure = Some(e);
            return Err(io::Error::other("중단됨").into());
        }
        Ok(true)
    });
    if let Some(e) = failure {
        return Err(e);
    }
    result.map_err(|e| sevenz_error(e, "해제", opts))?;
    Ok(sink.stats)
}

// ---------------------------------------------------------------------------
// RAR (UnRAR 라이브러리 사용 — 풀기만 가능)
// ---------------------------------------------------------------------------

#[cfg(feature = "rar")]
fn rar_error(e: unrar::error::UnrarError, opts: &Options, encrypted: bool) -> anyhow::Error {
    use unrar::error::Code;
    match e.code {
        Code::MissingPassword => anyhow::Error::new(PasswordError { wrong: false }),
        Code::BadPassword => anyhow::Error::new(PasswordError { wrong: true }),
        // RAR4는 암호가 틀리면 데이터 오류로 나옴
        Code::BadData if encrypted && opts.password.is_some() => anyhow::Error::new(PasswordError { wrong: true }),
        _ => anyhow::anyhow!("RAR 처리 실패: {e}"),
    }
}

#[cfg(feature = "rar")]
fn rar_open<'a>(path: &'a Path, opts: &'a Options) -> unrar::Archive<'a> {
    match &opts.password {
        Some(pw) => unrar::Archive::with_password(path, pw.as_str()),
        None => unrar::Archive::new(path),
    }
}

#[cfg(feature = "rar")]
fn extract_rar(path: &Path, root: &Path, opts: &Options) -> Result<Stats> {
    let mut sink = Sink::new(root, opts)?;
    let mut cur = rar_open(path, opts).open_for_processing().map_err(|e| rar_error(e, opts, true))?;
    while let Some(header) = cur.read_header().map_err(|e| rar_error(e, opts, true))? {
        let entry = header.entry();
        let raw = entry.filename.to_string_lossy().replace('\\', "/");
        let encrypted = entry.is_encrypted();
        let size = entry.unpacked_size;
        let is_dir = entry.is_directory();
        cur = match sanitize(&raw) {
            None => {
                sink.warn_unsafe(&raw);
                header.skip()
            }
            Some(rel) if is_dir => {
                sink.dir(&rel)?;
                header.skip()
            }
            Some(rel) => match sink.prepare(&rel)? {
                Prepared::Skip => header.skip(),
                Prepared::Write(target) => {
                    let next = header.extract_to(&target).map_err(|e| rar_error(e, opts, encrypted))?;
                    sink.stats.files += 1;
                    sink.stats.bytes += size;
                    if opts.verbose {
                        println!("  {}", rel.display());
                    }
                    Ok(next)
                }
            },
        }
        .map_err(|e| rar_error(e, opts, encrypted))?;
    }
    Ok(sink.stats)
}

#[cfg(feature = "rar")]
fn list_rar(path: &Path, opts: &Options) -> Result<Vec<EntryInfo>> {
    let archive = rar_open(path, opts).open_for_listing().map_err(|e| rar_error(e, opts, true))?;
    let mut out = Vec::new();
    for entry in archive {
        let e = entry.map_err(|e| rar_error(e, opts, true))?;
        // 분할 압축에서 이어지는 조각은 한 번만 표시
        if e.is_split_before() {
            continue;
        }
        out.push(EntryInfo {
            name: e.filename.to_string_lossy().replace('\\', "/"),
            size: e.unpacked_size,
            is_dir: e.is_directory(),
        });
    }
    Ok(out)
}

#[cfg(not(feature = "rar"))]
fn extract_rar(_: &Path, _: &Path, _: &Options) -> Result<Stats> {
    bail!("이 빌드는 RAR를 지원하지 않습니다 (rar 기능을 켜고 빌드하세요)")
}

#[cfg(not(feature = "rar"))]
fn list_rar(_: &Path, _: &Options) -> Result<Vec<EntryInfo>> {
    bail!("이 빌드는 RAR를 지원하지 않습니다 (rar 기능을 켜고 빌드하세요)")
}

// ---------------------------------------------------------------------------
// 목록
// ---------------------------------------------------------------------------

/// 압축 파일의 항목 목록을 반환합니다 (해제하지 않음).
pub fn list(archive: &Path, opts: &Options) -> Result<Vec<EntryInfo>> {
    match detect(archive)? {
        Format::Zip => {
            let mut zip = zip::ZipArchive::new(open_buffered(archive)?).context("zip 읽기 실패")?;
            let mut out = Vec::with_capacity(zip.len());
            for i in 0..zip.len() {
                let f = zip.by_index_raw(i)?;
                out.push(EntryInfo {
                    name: zip_entry_name(&f),
                    size: f.size(),
                    is_dir: f.is_dir(),
                });
            }
            Ok(out)
        }
        Format::SevenZ => {
            let reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_password(opts))
                .map_err(|e| sevenz_error(e, "열기", opts))?;
            Ok(reader
                .archive()
                .files
                .iter()
                .map(|e| EntryInfo { name: e.name().to_string(), size: e.size(), is_dir: e.is_directory() })
                .collect())
        }
        Format::Rar => list_rar(archive, opts),
        Format::Tar(codec) => list_tar(decoder(codec, open_buffered(archive)?)?),
        Format::Stream(codec) => {
            let mut reader = decoder(Some(codec), open_buffered(archive)?)?;
            let mut head = vec![0u8; 512];
            let n = read_up_to(&mut reader, &mut head)?;
            head.truncate(n);
            if is_tar_header(&head) {
                list_tar(Box::new(Cursor::new(head).chain(reader)))
            } else {
                let size = n as u64 + io::copy(&mut reader, &mut io::sink())?;
                Ok(vec![EntryInfo { name: base_name(archive), size, is_dir: false }])
            }
        }
    }
}

fn list_tar(reader: Box<dyn Read>) -> Result<Vec<EntryInfo>> {
    let mut ar = tar::Archive::new(reader);
    let mut out = Vec::new();
    for entry in ar.entries()? {
        let entry = entry?;
        out.push(EntryInfo {
            name: entry.path()?.to_string_lossy().into_owned(),
            size: entry.header().size().unwrap_or(0),
            is_dir: entry.header().entry_type().is_dir(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_traversal() {
        assert_eq!(sanitize("a/b.txt"), Some(PathBuf::from("a/b.txt")));
        assert_eq!(sanitize("./a/./b"), Some(PathBuf::from("a/b")));
        assert_eq!(sanitize("a\\b"), Some(PathBuf::from("a/b")));
        assert_eq!(sanitize("../evil"), None);
        assert_eq!(sanitize("a/../../evil"), None);
        assert_eq!(sanitize("/etc/passwd"), None);
        assert_eq!(sanitize("C:/Windows"), None);
        assert_eq!(sanitize(""), None);
    }

    #[test]
    fn symlink_safety() {
        assert!(symlink_target_is_safe(Path::new("a/link"), "../b"));
        assert!(symlink_target_is_safe(Path::new("a/link"), "c/d"));
        assert!(!symlink_target_is_safe(Path::new("link"), "../x"));
        assert!(!symlink_target_is_safe(Path::new("a/link"), "../../x"));
        assert!(!symlink_target_is_safe(Path::new("link"), "/etc/passwd"));
    }

    #[test]
    fn names() {
        assert_eq!(base_name(Path::new("dir/foo.tar.gz")), "foo");
        assert_eq!(base_name(Path::new("Foo.ZIP")), "Foo");
        assert_eq!(base_name(Path::new("log.txt.gz")), "log.txt");
        assert_eq!(base_name(Path::new("noext")), "noext.out");
        assert_eq!(detect_by_name(Path::new("x.TGZ")), Some(Format::Tar(Some(Codec::Gz))));
        assert_eq!(detect_by_name(Path::new("x.gz")), Some(Format::Stream(Codec::Gz)));
        assert_eq!(detect_by_name(Path::new(".gz")), None);
    }
}
