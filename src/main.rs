use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use unpack::{ArchiveKind, Options, compress, detect, extract, extract_into, list};

/// opizenAI_Unpack 명령줄 도구
///
/// 해제: zip, 7z, tar, tar.gz, tar.bz2, tar.xz, tar.zst, gz, bz2, xz, zst
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

    /// 출력 폴더 (기본값: 압축 파일 옆에 확장자를 뺀 이름의 폴더)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// 해제하지 않고 내용 목록만 표시
    #[arg(short, long)]
    list: bool,

    /// 이미 존재하는 파일 덮어쓰기
    #[arg(short = 'f', long)]
    overwrite: bool,

    /// 암호 (zip, 7z — 해제와 압축 모두)
    #[arg(short, long)]
    password: Option<String>,

    /// 해제되는 파일 이름 출력
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let opts = Options { overwrite: cli.overwrite, password: cli.password, verbose: cli.verbose };
    if let Some(out) = &cli.create {
        return create(&cli.archives, out, opts.password.as_deref());
    }
    let many = cli.archives.len() > 1;
    let mut failed = 0;

    for archive in &cli.archives {

        let result = if cli.list {
            list(archive, &opts).map(|entries| {
                let fmt = detect(archive).map(|f| f.to_string()).unwrap_or_default();
                println!("{} [{fmt}] — {}개 항목", archive.display(), entries.len());
                for e in &entries {
                    if e.is_dir {
                        println!("{:>12}  {}", "<DIR>", e.name);
                    } else {
                        println!("{:>12}  {}", e.size, e.name);
                    }
                }
            })
        } else {
            // 여러 파일을 같은 출력 폴더로 풀 때는 각자 하위 폴더로 분리
            let result = match &cli.output {
                Some(o) if many => extract_into(archive, o, &opts),
                o => extract(archive, o.as_deref(), &opts),
            };
            result.map(|s| {
                for w in &s.warnings {
                    eprintln!("경고: {w}");
                }
                println!(
                    "완료: {} → {} (파일 {}개, 폴더 {}개, {} 바이트{})",
                    archive.display(),
                    s.output.display(),
                    s.files,
                    s.dirs,
                    s.bytes,
                    if s.skipped > 0 { format!(", 건너뜀 {}개", s.skipped) } else { String::new() }
                );
            })
        };

        if let Err(e) = result {
            let hint = if unpack::is_password_error(&e) { " (-p 옵션으로 암호를 지정하세요)" } else { "" };
            eprintln!("오류: {}: {e:#}{hint}", archive.display());
            failed += 1;
        }
    }

    if failed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn create(inputs: &[PathBuf], out: &std::path::Path, password: Option<&str>) -> ExitCode {
    let Some(kind) = ArchiveKind::from_path(out) else {
        eprintln!("오류: 출력 파일 확장자는 .zip .7z .tar.gz .tar.xz 중 하나여야 합니다");
        return ExitCode::FAILURE;
    };
    match compress(inputs, out, kind, password) {
        Ok(s) => {
            for w in &s.warnings {
                eprintln!("경고: {w}");
            }
            let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
            println!(
                "완료: {} (파일 {}개, 폴더 {}개, {} → {} 바이트)",
                out.display(),
                s.files,
                s.dirs,
                s.bytes,
                size
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("오류: {e:#}");
            ExitCode::FAILURE
        }
    }
}
