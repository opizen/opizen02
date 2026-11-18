//! `unpack` 라이브러리: 여러 압축 형식을 안전하게 해제합니다.
//!
//! 지원 형식: zip, 7z, rar(풀기만), tar, tar.gz/tgz, tar.bz2/tbz2, tar.xz/txz, tar.zst/tzst,
//! 단일 스트림 gz / bz2 / xz / zst, 그리고 이들의 단순 분할 파일(`이름.zip.001`, `이름.7z.001` …).
//!
//! 모든 항목 경로는 대상 폴더 밖으로 나갈 수 없도록 검사됩니다(Zip Slip 방지).
//! 압축 폭탄을 막기 위해 디스크 공간·압축률을 미리 확인하고, 쓰는 동안에도 크기를 제한합니다.

use anyhow::{Context, Result, bail};
use std::fs::{self, File};
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod compress;
pub use compress::{ArchiveKind, CompressOptions, CompressStats, Level, compress, compress_with, default_archive_path};

/// 프로그램 이름
pub const APP_NAME: &str = "opizenAI_Unpack";

/// 압축 폭탄 판단 기준
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// 압축률(풀린 크기 ÷ 압축 크기)이 이보다 높고
    pub ratio: u64,
    /// 풀린 크기가 이보다 크면 압축 폭탄으로 의심합니다
    pub min_size: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { ratio: 100, min_size: 1 << 30 }
    }
}

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
    /// RAR (풀기만 지원)
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

// ---------------------------------------------------------------------------
// 진행률 · 취소
// ---------------------------------------------------------------------------

/// 작업 진행 상황. 작업 스레드가 갱신하고 화면이 읽습니다. `cancel()`로 멈출 수 있습니다.
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
    cancel: AtomicBool,
    current: Mutex<String>,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    /// 0.0 ~ 1.0 (전체 크기를 모르면 `None`)
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| (self.done.load(Ordering::Relaxed) as f64 / total as f64).min(1.0) as f32)
    }
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }
    /// 지금 처리 중인 항목 이름
    pub fn current(&self) -> String {
        self.current.lock().map(|s| s.clone()).unwrap_or_default()
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    /// 새 작업을 시작할 때 초기화 (취소 상태는 유지)
    pub fn reset(&self, total: u64) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }
    pub(crate) fn add(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }
    pub(crate) fn set_current(&self, s: &str) {
        if let Ok(mut c) = self.current.lock() {
            s.clone_into(&mut c);
        }
    }
}

/// 사용자가 작업을 취소함
#[derive(Debug, Clone, Copy)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("취소했습니다")
    }
}
impl std::error::Error for Cancelled {}

/// 압축 폭탄이 의심되거나 디스크 공간이 부족함 ([`Options::allow_large`]로 무시 가능)
#[derive(Debug, Clone)]
pub struct TooLarge(pub String);
impl std::fmt::Display for TooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for TooLarge {}

fn chain_has<T: std::error::Error + Send + Sync + 'static>(e: &anyhow::Error) -> bool {
    // context()로 붙인 값은 downcast_ref로만 찾을 수 있음
    e.downcast_ref::<T>().is_some()
        || e.chain().any(|c| {
            c.downcast_ref::<T>().is_some()
                || c.downcast_ref::<io::Error>().and_then(|io| io.get_ref()).is_some_and(|inner| inner.downcast_ref::<T>().is_some())
        })
}

/// 취소 때문에 끝난 오류인지
pub fn is_cancelled(e: &anyhow::Error) -> bool {
    chain_has::<Cancelled>(e)
}

/// 압축 폭탄 / 디스크 공간 부족 오류인지
pub fn is_too_large(e: &anyhow::Error) -> bool {
    chain_has::<TooLarge>(e)
}

/// 일부 항목만 풀 때 쓰는 선택
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// 결과 경로에서 떼어 낼 앞부분 (예: 보고 있던 폴더 `사진/여행/`)
    pub prefix: String,
    /// 고른 항목 전체 경로 (폴더면 그 안 전부)
    pub names: Vec<String>,
}

impl Selection {
    /// 선택에 들면 결과 상대 경로를, 아니면 `None`
    fn map(&self, name: &str) -> Option<String> {
        let name = name.trim_end_matches('/');
        let hit = self.names.iter().any(|n| {
            let n = n.trim_end_matches('/');
            name == n || name.strip_prefix(n).is_some_and(|rest| rest.starts_with('/'))
        });
        if !hit {
            return None;
        }
        let rel = name.strip_prefix(self.prefix.trim_end_matches('/')).map(|r| r.trim_start_matches('/')).unwrap_or(name);
        (!rel.is_empty()).then(|| rel.to_string())
    }
}

/// 해제 옵션.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// 이미 존재하는 파일을 덮어쓸지 여부
    pub overwrite: bool,
    /// 암호 (zip, 7z, rar)
    pub password: Option<String>,
    /// 해제되는 각 항목 이름 출력
    pub verbose: bool,
    /// 진행률 / 취소
    pub progress: Option<Arc<Progress>>,
    /// 일부 항목만 풀기
    pub select: Option<Arc<Selection>>,
    /// 압축 폭탄·디스크 공간 검사를 건너뜀
    pub allow_large: bool,
    /// 압축 폭탄 판단 기준
    pub limits: Limits,
    /// 실제로 쓰지 않고 읽기만 해서 손상 여부 확인 (보통은 [`test_archive`]를 쓰세요)
    pub test_only: bool,
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
    /// 수정 시간 (기록돼 있으면)
    pub modified: Option<SystemTime>,
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
    chain_has::<PasswordError>(e)
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
// 분할 파일(이름.zip.001 …)을 하나로 읽기
// ---------------------------------------------------------------------------

/// `이름.확장자.001` 이면 (`이름.확장자`, 001부터 이어지는 조각 목록)
fn split_parts(path: &Path) -> Option<(PathBuf, Vec<PathBuf>)> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let (inner, num) = name.rsplit_once('.')?;
    if num.len() < 3 || !num.chars().all(|c| c.is_ascii_digit()) || num.parse::<u32>().ok()? != 1 || inner.is_empty() {
        return None;
    }
    let width = num.len();
    let parent = path.parent().unwrap_or(Path::new(""));
    let parts: Vec<PathBuf> = (1..)
        .map(|i| parent.join(format!("{inner}.{i:0width$}")))
        .take_while(|p| p.is_file())
        .collect();
    (!parts.is_empty()).then(|| (parent.join(inner), parts))
}

/// 압축 파일을 이루는 모든 파일 (분할 조각 포함). 휴지통으로 보낼 때 씁니다.
pub fn archive_parts(path: &Path) -> Vec<PathBuf> {
    if let Some((_, parts)) = split_parts(path) {
        return parts;
    }
    // RAR 분할: 이름.part1.rar, 이름.part2.rar …
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if let Some(stem) = name.strip_suffix(".rar")
        && let Some(i) = stem.rfind(".part")
        && stem[i + 5..].chars().all(|c| c.is_ascii_digit())
        && stem.len() > i + 5
    {
        let base = &stem[..i];
        let parent = path.parent().unwrap_or(Path::new(""));
        let mut parts: Vec<PathBuf> = fs::read_dir(parent)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| {
                        let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                        n.strip_suffix(".rar")
                            .and_then(|s| s.strip_prefix(base))
                            .and_then(|s| s.strip_prefix(".part"))
                            .is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        parts.sort();
        if !parts.is_empty() {
            return parts;
        }
    }
    vec![path.to_path_buf()]
}

/// 여러 파일을 이어 붙인 것처럼 읽는 리더
struct MultiFile {
    files: Vec<(File, u64)>,
    total: u64,
    pos: u64,
}

impl MultiFile {
    fn open(parts: &[PathBuf]) -> io::Result<Self> {
        let mut files = Vec::new();
        let mut total = 0;
        for p in parts {
            let f = File::open(p)?;
            let len = f.metadata()?.len();
            total += len;
            files.push((f, len));
        }
        Ok(Self { files, total, pos: 0 })
    }
}

impl Read for MultiFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut start = 0;
        for (f, len) in &mut self.files {
            if self.pos < start + *len {
                f.seek(SeekFrom::Start(self.pos - start))?;
                let want = buf.len().min((start + *len - self.pos) as usize);
                let n = f.read(&mut buf[..want])?;
                self.pos += n as u64;
                return Ok(n);
            }
            start += *len;
        }
        Ok(0)
    }
}

impl Seek for MultiFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let new = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => self.total as i64 + n,
            SeekFrom::Current(n) => self.pos as i64 + n,
        };
        if new < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "음수 위치"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

/// 압축 파일 입력 (보통 파일 하나, 분할이면 여러 조각)
enum Input {
    One(File),
    Many(MultiFile),
}

impl Read for Input {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Input::One(f) => f.read(buf),
            Input::Many(m) => m.read(buf),
        }
    }
}

impl Seek for Input {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        match self {
            Input::One(f) => f.seek(to),
            Input::Many(m) => m.seek(to),
        }
    }
}

fn open_input(path: &Path) -> Result<BufReader<Input>> {
    let input = match split_parts(path) {
        Some((_, parts)) => Input::Many(MultiFile::open(&parts).with_context(|| format!("열 수 없음: {}", path.display()))?),
        None => Input::One(File::open(path).with_context(|| format!("열 수 없음: {}", path.display()))?),
    };
    Ok(BufReader::with_capacity(256 * 1024, input))
}

/// 압축 파일 전체 크기 (분할 조각 합)
fn input_size(path: &Path) -> u64 {
    archive_parts(path).iter().filter_map(|p| fs::metadata(p).ok()).map(|m| m.len()).sum()
}

// ---------------------------------------------------------------------------
// 형식 감지
// ---------------------------------------------------------------------------

/// 확장자와 매직 바이트로 형식을 감지합니다. 분할 파일(`.001`)도 알아봅니다.
pub fn detect(path: &Path) -> Result<Format> {
    let name_path = split_parts(path).map(|(inner, _)| inner).unwrap_or_else(|| path.to_path_buf());
    if let Some(f) = detect_by_name(&name_path) {
        return Ok(f);
    }
    let mut head = [0u8; 512];
    let mut input = open_input(path)?;
    let n = read_up_to(&mut input, &mut head)?;
    detect_by_magic(&head[..n]).with_context(|| format!("지원하지 않거나 알 수 없는 형식: {}", path.display()))
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
    table.iter().find(|(ext, _)| name.ends_with(ext) && name.len() > ext.len()).map(|(_, f)| *f)
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

/// 읽은 만큼 진행률에 더하고, 취소되면 멈추는 리더 (tar·스트림처럼 전체 크기를 미리 모를 때 입력 쪽에 씀)
struct Counted<R> {
    inner: R,
    progress: Option<Arc<Progress>>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(p) = &self.progress {
            if p.is_cancelled() {
                return Err(io::Error::other(Cancelled));
            }
            let n = self.inner.read(buf)?;
            p.add(n as u64);
            Ok(n)
        } else {
            self.inner.read(buf)
        }
    }
}

struct Sink<'a> {
    root: PathBuf,
    opts: &'a Options,
    stats: Stats,
    /// 쓴 바이트를 진행률에 더할지 (입력 쪽에서 세는 경우 false)
    count_output: bool,
    /// 이보다 많이 쓰면 압축 폭탄으로 보고 멈춤
    limit: Option<u64>,
}

enum Prepared {
    Write(PathBuf),
    Skip,
}

impl<'a> Sink<'a> {
    fn new(root: &Path, opts: &'a Options) -> Result<Self> {
        if !opts.test_only {
            fs::create_dir_all(root).with_context(|| format!("출력 폴더를 만들 수 없음: {}", root.display()))?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            opts,
            stats: Stats { output: root.to_path_buf(), ..Default::default() },
            count_output: true,
            limit: None,
        })
    }

    fn check_cancel(&self) -> Result<()> {
        if self.opts.progress.as_ref().is_some_and(|p| p.is_cancelled()) {
            return Err(Cancelled.into());
        }
        Ok(())
    }

    /// 압축 파일 안 이름을 선택·안전 검사를 거쳐 상대 경로로. 건너뛸 항목이면 `None`.
    fn entry_path(&mut self, raw: &str) -> Option<PathBuf> {
        let mapped = match &self.opts.select {
            Some(sel) => sel.map(raw)?,
            None => raw.to_string(),
        };
        match sanitize(&mapped) {
            Some(p) => Some(p),
            None => {
                self.warn_unsafe(raw);
                None
            }
        }
    }

    fn warn(&mut self, msg: String) {
        self.stats.warnings.push(msg);
        self.stats.skipped += 1;
    }

    fn warn_unsafe(&mut self, name: &str) {
        self.warn(format!("안전하지 않은 경로를 건너뜀: {name}"));
    }

    fn dir(&mut self, rel: &Path) -> Result<()> {
        if !self.opts.test_only {
            let path = self.root.join(rel);
            fs::create_dir_all(&path).with_context(|| format!("폴더 생성 실패: {}", path.display()))?;
        }
        self.stats.dirs += 1;
        if self.opts.verbose {
            println!("  {}/", rel.display());
        }
        Ok(())
    }

    /// 파일을 쓸 준비: 부모 폴더 생성, 덮어쓰기 정책 적용, 기존 심볼릭 링크 제거.
    fn prepare(&mut self, rel: &Path) -> Result<Prepared> {
        self.check_cancel()?;
        if let Some(p) = &self.opts.progress {
            p.set_current(&rel.to_string_lossy());
        }
        let path = self.root.join(rel);
        if self.opts.test_only {
            return Ok(Prepared::Write(path));
        }
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !self.opts.overwrite {
                self.warn(format!("이미 존재하여 건너뜀: {}", rel.display()));
                return Ok(Prepared::Skip);
            }
            if meta.is_dir() && !meta.file_type().is_symlink() {
                bail!("같은 이름의 폴더가 이미 존재함: {}", path.display());
            }
            // 기존 심볼릭 링크를 통해 바깥에 쓰는 일을 막기 위해 먼저 삭제
            fs::remove_file(&path).with_context(|| format!("기존 파일 삭제 실패: {}", path.display()))?;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("폴더 생성 실패: {}", parent.display()))?;
        }
        Ok(Prepared::Write(path))
    }

    /// 읽으면서 쓰기: 진행률, 취소, 크기 제한을 모두 여기서 확인
    fn copy(&mut self, reader: &mut dyn Read, out: &mut dyn Write) -> Result<u64> {
        let mut buf = vec![0u8; 256 * 1024];
        let mut n_total = 0u64;
        loop {
            self.check_cancel()?;
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            out.write_all(&buf[..n])?;
            n_total += n as u64;
            if self.count_output
                && let Some(p) = &self.opts.progress
            {
                p.add(n as u64);
            }
            if let Some(limit) = self.limit
                && self.stats.bytes + n_total > limit
            {
                return Err(TooLarge(format!(
                    "풀린 크기가 예상({})보다 훨씬 커서 멈췄습니다. 압축 폭탄일 수 있습니다",
                    human(limit)
                ))
                .into());
            }
        }
        Ok(n_total)
    }

    fn file(&mut self, rel: &Path, reader: &mut dyn Read, mode: Option<u32>, mtime: Option<SystemTime>) -> Result<()> {
        let Prepared::Write(path) = self.prepare(rel)? else {
            return Ok(());
        };
        let n = if self.opts.test_only {
            self.copy(reader, &mut io::sink())?
        } else {
            let mut out = File::create(&path).with_context(|| format!("파일 생성 실패: {}", path.display()))?;
            let n = match self.copy(reader, &mut out) {
                Ok(n) => n,
                Err(e) => {
                    // 반쯤 쓴 파일은 남기지 않음
                    drop(out);
                    let _ = fs::remove_file(&path);
                    return Err(e.context(format!("쓰기 실패: {}", path.display())));
                }
            };
            // 압축 파일에 기록된 수정 시간을 그대로 유지
            if let Some(t) = mtime {
                let _ = out.set_modified(t);
            }
            drop(out);
            set_mode(&path, mode);
            n
        };
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
        if !self.opts.test_only {
            make_symlink(target, &path)?;
        }
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
    std::os::unix::fs::symlink(target, path).with_context(|| format!("심볼릭 링크 생성 실패: {}", path.display()))
}

#[cfg(not(unix))]
fn make_symlink(target: &str, path: &Path) -> Result<()> {
    // 심볼릭 링크를 지원하지 않는 플랫폼에서는 대상 경로를 텍스트로 저장
    fs::write(path, target).with_context(|| format!("파일 생성 실패: {}", path.display()))
}

/// 사람이 읽기 쉬운 크기 (오류 문구용)
fn human(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", U[i]) }
}

// ---------------------------------------------------------------------------
// 해제
// ---------------------------------------------------------------------------

/// 압축 파일 이름에서 확장자를 뗀 기본 이름을 반환합니다.
pub fn base_name(path: &Path) -> String {
    // 분할 파일: 이름.zip.001 → 이름.zip 기준
    let path = split_parts(path).map(|(inner, _)| inner).unwrap_or_else(|| path.to_path_buf());
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
        ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".tbz", ".txz", ".tzst", ".tar", ".zip", ".jar",
        ".7z", ".gz", ".bz2", ".xz", ".zst",
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

/// 압축 파일을 실제로 풀지 않고 끝까지 읽어서 손상 여부를 확인합니다.
/// zip·7z·rar·gz·bz2·xz·zst는 저장된 체크섬(CRC)까지 확인합니다.
pub fn test_archive(archive: &Path, opts: &Options) -> Result<Stats> {
    let opts = Options { test_only: true, allow_large: true, overwrite: true, select: None, ..opts.clone() };
    let mut s = extract_impl(archive, Some(Path::new("")), PathBuf::new(), &opts)?;
    s.output = archive.to_path_buf();
    Ok(s)
}

/// 풀린 크기의 합 (목록에서 바로 알 수 있는 형식만)
fn known_unpacked_size(format: Format, archive: &Path, opts: &Options) -> Option<u64> {
    match format {
        Format::Zip | Format::SevenZ | Format::Rar => {
            let entries = list(archive, &Options { progress: None, ..opts.clone() }).ok()?;
            Some(
                entries
                    .iter()
                    .filter(|e| !e.is_dir && opts.select.as_ref().is_none_or(|s| s.map(&e.name).is_some()))
                    .map(|e| e.size)
                    .sum(),
            )
        }
        _ => None,
    }
}

/// 디스크 공간 · 압축률 검사 (압축 폭탄 방어). 쓰는 동안 쓸 크기 제한을 돌려줍니다.
fn safety_check(format: Format, archive: &Path, dest: &Path, opts: &Options) -> Result<Option<u64>> {
    let packed = input_size(archive).max(1);
    let unpacked = known_unpacked_size(format, archive, opts);
    if let (Some(p), Some(total)) = (&opts.progress, unpacked) {
        p.reset(total);
    } else if let Some(p) = &opts.progress {
        // 전체 크기를 모르면 압축 파일을 읽은 양으로 진행률 표시
        p.reset(packed);
    }
    if opts.test_only {
        return Ok(None);
    }
    if !opts.allow_large
        && let Some(total) = unpacked {
            if total > opts.limits.min_size && total / packed > opts.limits.ratio {
                return Err(TooLarge(format!(
                    "압축률이 비정상적으로 높습니다 ({} → {}, {}배). 압축 폭탄일 수 있습니다",
                    human(packed),
                    human(total),
                    total / packed
                ))
                .into());
            }
            // 대상 폴더(또는 가장 가까운 상위 폴더)의 남은 공간
            let mut probe = dest.to_path_buf();
            while !probe.exists() {
                if !probe.pop() {
                    break;
                }
            }
            if probe.as_os_str().is_empty() {
                probe = PathBuf::from(".");
            }
            if let Ok(free) = fs4::available_space(&probe)
                && total + (16 << 20) > free
            {
                return Err(TooLarge(format!("디스크 공간이 부족합니다 (필요 {}, 남은 공간 {})", human(total), human(free))).into());
            }
        }
    if opts.allow_large {
        return Ok(None);
    }
    // 쓰는 동안 제한: 알면 예상 크기 + 여유, 모르면 압축 크기의 ratio배 (최소 min_size)
    Ok(Some(match unpacked {
        Some(total) => total + total / 10 + (1 << 20),
        None => packed.saturating_mul(opts.limits.ratio).max(opts.limits.min_size),
    }))
}

fn extract_impl(archive: &Path, dest: Option<&Path>, parent: PathBuf, opts: &Options) -> Result<Stats> {
    let r = extract_inner(archive, dest, parent, opts);
    // 라이브러리가 오류를 글자로 바꿔 버려도 취소였다면 취소로 알림
    match r {
        Err(e) if opts.progress.as_ref().is_some_and(|p| p.is_cancelled()) && !is_cancelled(&e) => Err(e.context(Cancelled)),
        r => r,
    }
}

fn extract_inner(archive: &Path, dest: Option<&Path>, parent: PathBuf, opts: &Options) -> Result<Stats> {
    let format = detect(archive)?;
    let default_dir = || parent.join(base_name(archive));
    let target = dest.map(PathBuf::from).unwrap_or_else(default_dir);
    let limit = safety_check(format, archive, &target, opts)?;

    match format {
        Format::Zip => extract_zip(archive, &target, opts, limit),
        Format::SevenZ => extract_7z(archive, &target, opts, limit),
        Format::Rar => extract_rar(archive, &target, opts),
        Format::Tar(codec) => {
            let reader = decoder(codec, counted_input(archive, opts)?)?;
            extract_tar(reader, &target, opts, limit, false)
        }
        Format::Stream(codec) => {
            let mut reader = decoder(Some(codec), counted_input(archive, opts)?)?;
            let mut head = vec![0u8; 512];
            let n = read_up_to(&mut reader, &mut head)?;
            head.truncate(n);
            let is_tar = is_tar_header(&head);
            let full = Cursor::new(head).chain(reader);
            if is_tar {
                extract_tar(Box::new(full), &target, opts, limit, false)
            } else {
                let root = dest.map(PathBuf::from).unwrap_or(parent.clone());
                write_plain(&mut Box::new(full), &root, &base_name(archive), opts, limit)
            }
        }
    }
}

/// 진행률을 압축 파일 읽은 양으로 세는 입력 (tar, 스트림)
fn counted_input(archive: &Path, opts: &Options) -> Result<Box<dyn Read>> {
    Ok(Box::new(BufReader::new(Counted { inner: open_input(archive)?, progress: opts.progress.clone() })))
}

/// 압축되지 않은 단일 파일 내용을 `root/name`에 씁니다.
fn write_plain(reader: &mut dyn Read, root: &Path, name: &str, opts: &Options, limit: Option<u64>) -> Result<Stats> {
    let root = if root.as_os_str().is_empty() && !opts.test_only { PathBuf::from(".") } else { root.to_path_buf() };
    let mut sink = Sink::new(&root, opts)?;
    sink.count_output = false;
    sink.limit = limit;
    let rel = PathBuf::from(name);
    sink.file(&rel, reader, None, None)?;
    sink.stats.output = root.join(rel);
    Ok(sink.stats)
}

/// 단일 파일 스트림(.gz 등)이면서 내용이 tar가 아닌지 확인합니다.
fn is_plain_stream(archive: &Path) -> Result<bool> {
    let Format::Stream(codec) = detect(archive)? else { return Ok(false) };
    let mut reader = decoder(Some(codec), Box::new(open_input(archive)?))?;
    let mut head = vec![0u8; 512];
    let n = read_up_to(&mut reader, &mut head)?;
    Ok(!is_tar_header(&head[..n]))
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
/// 단일 파일 스트림(.gz 등)은 모든 방식에서 `parent`에 파일 하나를 만듭니다.
pub fn extract_auto(archive: &Path, parent: &Path, mode: Mode, opts: &Options) -> Result<Stats> {
    let base = base_name(archive);
    if is_plain_stream(archive)? {
        let target = unique_path(&parent.join(&base));
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(base);
        let Format::Stream(codec) = detect(archive)? else { unreachable!() };
        let limit = safety_check(Format::Stream(codec), archive, parent, opts)?;
        let mut reader = decoder(Some(codec), counted_input(archive, opts)?)?;
        return write_plain(&mut reader, parent, &name, opts, limit);
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
                fs::rename(entry.path(), &target).with_context(|| format!("이동 실패: {}", target.display()))?;
                fs::remove_dir(&tmp)?;
                target
            }
            _ => {
                let target = unique_path(&parent.join(&base));
                fs::rename(&tmp, &target).with_context(|| format!("이동 실패: {}", target.display()))?;
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

/// 압축 파일(분할 조각 포함)을 휴지통으로 보냅니다.
pub fn move_to_trash(archive: &Path) -> Result<()> {
    for p in archive_parts(archive) {
        trash::delete(&p).with_context(|| format!("휴지통으로 보내지 못함: {}", p.display()))?;
    }
    Ok(())
}

fn decoder(codec: Option<Codec>, r: Box<dyn Read>) -> Result<Box<dyn Read>> {
    Ok(match codec {
        None => r,
        Some(Codec::Gz) => Box::new(flate2::read::MultiGzDecoder::new(r)),
        Some(Codec::Bz2) => Box::new(bzip2::read::MultiBzDecoder::new(r)),
        Some(Codec::Xz) => Box::new(xz2::read::XzDecoder::new_multi_decoder(r)),
        Some(Codec::Zst) => Box::new(zstd::stream::read::Decoder::new(r)?),
    })
}

fn extract_tar(reader: Box<dyn Read>, root: &Path, opts: &Options, limit: Option<u64>, count_output: bool) -> Result<Stats> {
    let mut sink = Sink::new(root, opts)?;
    sink.count_output = count_output;
    sink.limit = limit;
    let mut ar = tar::Archive::new(reader);
    ar.set_overwrite(true);
    for entry in ar.entries().context("tar 읽기 실패")? {
        sink.check_cancel()?;
        let mut entry = entry.context("tar 항목 읽기 실패")?;
        let raw = entry.path()?.to_string_lossy().into_owned();
        let Some(rel) = sink.entry_path(&raw) else {
            continue;
        };
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            sink.dir(&rel)?;
            continue;
        }
        if kind.is_symlink() {
            let target = entry.link_name()?.map(|t| t.to_string_lossy().into_owned()).unwrap_or_default();
            sink.symlink(&rel, &target)?;
            continue;
        }
        if !(kind.is_file() || kind.is_hard_link() || kind.is_contiguous()) {
            // 장치 파일, FIFO 등은 건너뜀 (pax/gnu 확장 헤더는 tar 크레이트가 처리)
            continue;
        }
        if kind.is_hard_link() {
            if opts.test_only || opts.select.is_some() {
                sink.stats.files += 1;
                continue;
            }
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
    // tar 끝 표시 뒤의 나머지(gzip·xz 등의 체크섬)까지 읽어야 손상을 알아챔
    io::copy(&mut ar.into_inner(), &mut io::sink()).context("압축 데이터 끝부분 확인 실패 (파일이 손상되었을 수 있음)")?;
    Ok(sink.stats)
}

fn zip_open_error(e: zip::result::ZipError, i: usize) -> anyhow::Error {
    match e {
        zip::result::ZipError::UnsupportedArchive(m) if m == zip::result::ZipError::PASSWORD_REQUIRED => {
            anyhow::Error::new(PasswordError { wrong: false })
        }
        zip::result::ZipError::InvalidPassword => anyhow::Error::new(PasswordError { wrong: true }),
        e => anyhow::Error::new(e).context(format!("zip 항목 #{i} 읽기 실패")),
    }
}

fn extract_zip(path: &Path, root: &Path, opts: &Options, limit: Option<u64>) -> Result<Stats> {
    let mut zip = zip::ZipArchive::new(open_input(path)?).context("zip 읽기 실패")?;
    let mut sink = Sink::new(root, opts)?;
    sink.limit = limit;
    for i in 0..zip.len() {
        sink.check_cancel()?;
        // 선택 풀기면 이름부터 보고 고르기 (암호 확인은 실제로 풀 항목에서만)
        if opts.select.is_some() {
            let name = zip.by_index_raw(i).map(|f| zip_entry_name(&f)).map_err(|e| zip_open_error(e, i))?;
            if opts.select.as_ref().is_some_and(|s| s.map(&name).is_none()) {
                continue;
            }
        }
        let mut file = match &opts.password {
            Some(pw) => zip.by_index_decrypt(i, pw.as_bytes()),
            None => zip.by_index(i),
        }
        .map_err(|e| zip_open_error(e, i))?;
        let raw = zip_entry_name(&file);
        let Some(rel) = sink.entry_path(&raw) else {
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

fn local_offset() -> time::UtcOffset {
    time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC)
}

/// zip의 날짜(현지 시각 기준)를 시스템 시간으로
fn zip_time_to_system(t: zip::DateTime) -> Option<SystemTime> {
    let pdt = time::PrimitiveDateTime::try_from(t).ok()?;
    Some(SystemTime::from(pdt.assume_offset(local_offset())))
}

/// MS-DOS 날짜·시간(현지 시각, RAR 등에서 사용)을 시스템 시간으로
#[cfg_attr(not(feature = "rar"), allow(dead_code))]
fn dos_time_to_system(v: u32) -> Option<SystemTime> {
    let date = (v >> 16) as u16;
    let tm = v as u16;
    zip::DateTime::try_from((date, tm)).ok().and_then(zip_time_to_system)
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
        sevenz_rust2::Error::MaybeBadPassword(_) => anyhow::Error::new(PasswordError { wrong: opts.password.is_some() }),
        e => anyhow::anyhow!("7z {what} 실패: {e}"),
    }
}

fn open_7z(path: &Path, opts: &Options) -> Result<sevenz_rust2::ArchiveReader<BufReader<Input>>> {
    sevenz_rust2::ArchiveReader::new(open_input(path)?, sevenz_password(opts)).map_err(|e| sevenz_error(e, "열기", opts))
}

fn extract_7z(path: &Path, root: &Path, opts: &Options, limit: Option<u64>) -> Result<Stats> {
    let mut reader = open_7z(path, opts)?;
    let mut sink = Sink::new(root, opts)?;
    sink.limit = limit;
    let mut failure: Option<anyhow::Error> = None;
    let result = reader.for_each_entries(|entry, data| {
        if entry.is_anti_item() {
            return Ok(true);
        }
        let Some(rel) = sink.entry_path(entry.name()) else {
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
        sink.check_cancel()?;
        let entry = header.entry();
        let raw = entry.filename.to_string_lossy().replace('\\', "/");
        let encrypted = entry.is_encrypted();
        let size = entry.unpacked_size;
        let is_dir = entry.is_directory();
        let split_cont = entry.is_split_before();
        cur = match sink.entry_path(&raw) {
            None => header.skip(),
            Some(rel) if is_dir => {
                sink.dir(&rel)?;
                header.skip()
            }
            Some(rel) if opts.test_only => {
                let next = header.test().map_err(|e| rar_error(e, opts, encrypted))?;
                if !split_cont {
                    sink.stats.files += 1;
                    sink.stats.bytes += size;
                }
                if let Some(p) = &opts.progress {
                    p.add(size);
                }
                let _ = rel;
                Ok(next)
            }
            Some(rel) => match sink.prepare(&rel)? {
                Prepared::Skip => header.skip(),
                Prepared::Write(target) => {
                    let next = header.extract_to(&target).map_err(|e| rar_error(e, opts, encrypted))?;
                    sink.stats.files += 1;
                    sink.stats.bytes += size;
                    if let Some(p) = &opts.progress {
                        p.add(size);
                    }
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
            modified: dos_time_to_system(e.file_time),
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
            let mut zip = zip::ZipArchive::new(open_input(archive)?).context("zip 읽기 실패")?;
            let mut out = Vec::with_capacity(zip.len());
            for i in 0..zip.len() {
                let f = zip.by_index_raw(i)?;
                out.push(EntryInfo {
                    name: zip_entry_name(&f),
                    size: f.size(),
                    is_dir: f.is_dir(),
                    modified: f.last_modified().and_then(zip_time_to_system),
                });
            }
            Ok(out)
        }
        Format::SevenZ => {
            let reader = open_7z(archive, opts)?;
            Ok(reader
                .archive()
                .files
                .iter()
                .map(|e| EntryInfo {
                    name: e.name().to_string(),
                    size: e.size(),
                    is_dir: e.is_directory(),
                    modified: e.has_last_modified_date.then(|| SystemTime::from(e.last_modified_date())),
                })
                .collect())
        }
        Format::Rar => list_rar(archive, opts),
        Format::Tar(codec) => list_tar(decoder(codec, Box::new(open_input(archive)?))?),
        Format::Stream(codec) => {
            let mut reader = decoder(Some(codec), Box::new(open_input(archive)?))?;
            let mut head = vec![0u8; 512];
            let n = read_up_to(&mut reader, &mut head)?;
            head.truncate(n);
            if is_tar_header(&head) {
                list_tar(Box::new(Cursor::new(head).chain(reader)))
            } else {
                let size = n as u64 + io::copy(&mut reader, &mut io::sink())?;
                let modified = fs::metadata(archive).and_then(|m| m.modified()).ok();
                Ok(vec![EntryInfo { name: base_name(archive), size, is_dir: false, modified }])
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
            modified: entry.header().mtime().ok().filter(|&t| t > 0).map(|t| UNIX_EPOCH + Duration::from_secs(t)),
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

    #[test]
    fn selection_maps_paths() {
        let s = Selection { prefix: "사진/여행".into(), names: vec!["사진/여행/제주".into(), "사진/여행/a.jpg".into()] };
        assert_eq!(s.map("사진/여행/제주/바다.jpg").as_deref(), Some("제주/바다.jpg"));
        assert_eq!(s.map("사진/여행/제주/").as_deref(), Some("제주"));
        assert_eq!(s.map("사진/여행/a.jpg").as_deref(), Some("a.jpg"));
        assert_eq!(s.map("사진/여행/제주도.jpg"), None); // 이름이 비슷해도 다른 항목
        assert_eq!(s.map("사진/여행/b.jpg"), None);
    }

    #[test]
    fn dos_time() {
        // 2020-01-02 03:04:06 (DOS: 날짜 0x5022, 시간 0x1883)
        let t = dos_time_to_system((0x5022u32 << 16) | 0x1883).unwrap();
        let utc = time::OffsetDateTime::from(t).to_offset(local_offset());
        assert_eq!((utc.year(), utc.month() as u8, utc.day(), utc.hour(), utc.minute(), utc.second()), (2020, 1, 2, 3, 4, 6));
    }
}
