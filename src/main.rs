use clap::{Parser, ValueEnum};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
use unpack::{
    ArchiveKind, CompressOptions, Level, Options, Progress, Selection, compress_with, detect, extract, extract_into,
    is_password_error, is_too_large, list, move_to_trash, test_archive,
};

/// opizenAI_Unpack 명령줄 도구
///
/// 해제: zip, 7z, rar, tar, tar.gz, tar.bz2, tar.xz, tar.zst, gz, bz2, xz, zst (분할 .001 포함)
/// 압축: -c 이름.zip | 이름.7z | 이름.tar.gz | 이름.tar.xz
#[derive(Parser, Debug)]
#[command(name = "unpack", version, about, long_about = None)]
struct Cli {
    /// 해제할 압축 파일 (여러 개 가능). -c와 함께 쓰면 압축할 파일·폴더
    #[arg(required = true)]
    archives: Vec<PathBuf>,

    /// 압축 파일 만들기 (형식은 확장자로 결정: .zip .7z .tar.gz .tar.xz)
    #[arg(short = 'c', long = "create", value_name = "ARCHIVE")]
    create: Option<PathBuf>,

    /// 압축 수준
    #[arg(long, value_enum, default_value_t = CliLevel::Normal)]
    level: CliLevel,

    /// 출력 폴더 (기본값: 압축 파일 옆에 확장자를 뺀 이름의 폴더)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// 해제하지 않고 내용 목록만 표시
    #[arg(short, long)]
    list: bool,

    /// 해제하지 않고 손상 여부만 검사
    #[arg(short, long)]
    test: bool,

    /// 이 항목만 풀기 (압축 파일 안 경로, 폴더면 그 안 전부. 여러 번 쓸 수 있음)
    #[arg(short = 'i', long = "include", value_name = "PATH")]
    include: Vec<String>,

    /// 이미 존재하는 파일 덮어쓰기
    #[arg(short = 'f', long)]
    overwrite: bool,

    /// 암호 (zip, 7z, rar — 해제와 압축 모두)
    #[arg(short, long)]
    password: Option<String>,

    /// 문제없이 풀리면 압축 파일을 휴지통으로 보내기
    #[arg(long)]
    trash: bool,

    /// 압축 폭탄·디스크 공간 검사를 건너뛰고 풀기
    #[arg(long)]
    allow_large: bool,

    /// 해제되는 파일 이름 출력
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliLevel {
    Fast,
    Normal,
    Best,
}

impl From<CliLevel> for Level {
    fn from(l: CliLevel) -> Self {
        match l {
            CliLevel::Fast => Level::Fast,
            CliLevel::Normal => Level::Normal,
            CliLevel::Best => Level::Best,
        }
    }
}

/// 터미널이면 진행률 한 줄을 갱신하면서 `job`을 실행
fn with_progress<T>(show: bool, job: impl FnOnce(Option<Arc<Progress>>) -> T) -> T {
    if !show || !std::io::stderr().is_terminal() {
        return job(None);
    }
    let p = Progress::new();
    let watcher = {
        let p = p.clone();
        std::thread::spawn(move || {
            while Arc::strong_count(&p) > 1 {
                if let Some(f) = p.fraction() {
                    let name: String = p.current().chars().take(50).collect();
                    eprint!("\r\x1b[K{:>3}%  {name}", (f * 100.0) as u32);
                    let _ = std::io::stderr().flush();
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            eprint!("\r\x1b[K");
        })
    };
    let r = job(Some(p));
    let _ = watcher.join();
    r
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let select = (!cli.include.is_empty()).then(|| Arc::new(Selection { prefix: String::new(), names: cli.include.clone() }));
    let base = Options {
        overwrite: cli.overwrite,
        password: cli.password.clone(),
        verbose: cli.verbose,
        allow_large: cli.allow_large,
        select,
        ..Default::default()
    };
    if let Some(out) = &cli.create {
        return create(&cli, out);
    }
    let many = cli.archives.len() > 1;
    let mut failed = 0;

    for archive in &cli.archives {
        let result = if cli.list {
            list(archive, &base).map(|entries| print_list(archive, &entries))
        } else if cli.test {
            with_progress(true, |p| test_archive(archive, &Options { progress: p, ..base.clone() })).map(|s| {
                println!("이상 없음: {} (파일 {}개, {})", archive.display(), s.files, human(s.bytes));
            })
        } else {
            let r = with_progress(!cli.verbose, |p| {
                let opts = Options { progress: p, ..base.clone() };
                // 여러 파일을 같은 출력 폴더로 풀 때는 각자 하위 폴더로 분리
                match &cli.output {
                    Some(o) if many => extract_into(archive, o, &opts),
                    o => extract(archive, o.as_deref(), &opts),
                }
            });
            r.map(|s| {
                for w in &s.warnings {
                    eprintln!("경고: {w}");
                }
                println!(
                    "완료: {} → {} (파일 {}개, 폴더 {}개, {}{})",
                    archive.display(),
                    s.output.display(),
                    s.files,
                    s.dirs,
                    human(s.bytes),
                    if s.skipped > 0 { format!(", 건너뜀 {}개", s.skipped) } else { String::new() }
                );
                if cli.trash {
                    if !s.warnings.is_empty() {
                        eprintln!("경고가 있어 압축 파일을 지우지 않았습니다: {}", archive.display());
                    } else if let Err(e) = move_to_trash(archive) {
                        eprintln!("경고: {e:#}");
                    } else {
                        println!("휴지통으로 보냄: {}", archive.display());
                    }
                }
            })
        };

        if let Err(e) = result {
            let hint = if is_password_error(&e) {
                " (-p 옵션으로 암호를 지정하세요)"
            } else if is_too_large(&e) {
                " (그래도 풀려면 --allow-large)"
            } else {
                ""
            };
            eprintln!("오류: {}: {e:#}{hint}", archive.display());
            failed += 1;
        }
    }

    if failed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn print_list(archive: &Path, entries: &[unpack::EntryInfo]) {
    // `| head` 처럼 출력이 먼저 닫혀도 멈추지 않도록 쓰기 오류는 조용히 끝냄
    let mut out = std::io::stdout().lock();
    let fmt = detect(archive).map(|f| f.to_string()).unwrap_or_default();
    if writeln!(out, "{} [{fmt}] — {}개 항목", archive.display(), entries.len()).is_err() {
        return;
    }
    for e in entries {
        let date = e.modified.map(fmt_time).unwrap_or_else(|| " ".repeat(16));
        let size = if e.is_dir { "<DIR>".to_string() } else { e.size.to_string() };
        if writeln!(out, "{size:>12}  {date}  {}", e.name).is_err() {
            return;
        }
    }
}

fn fmt_time(t: std::time::SystemTime) -> String {
    let utc = time::OffsetDateTime::from(t);
    let local = time::UtcOffset::current_local_offset().map(|o| utc.to_offset(o)).unwrap_or(utc);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        local.year(),
        local.month() as u8,
        local.day(),
        local.hour(),
        local.minute()
    )
}

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

fn create(cli: &Cli, out: &Path) -> ExitCode {
    let Some(kind) = ArchiveKind::from_path(out) else {
        eprintln!("오류: 출력 파일 확장자는 .zip .7z .tar.gz .tar.xz 중 하나여야 합니다");
        return ExitCode::FAILURE;
    };
    let r = with_progress(true, |p| {
        compress_with(
            &cli.archives,
            out,
            kind,
            &CompressOptions { password: cli.password.clone(), level: cli.level.into(), progress: p },
        )
    });
    match r {
        Ok(s) => {
            for w in &s.warnings {
                eprintln!("경고: {w}");
            }
            let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
            println!("완료: {} (파일 {}개, 폴더 {}개, {} → {})", out.display(), s.files, s.dirs, human(s.bytes), human(size));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("오류: {e:#}");
            ExitCode::FAILURE
        }
    }
}
