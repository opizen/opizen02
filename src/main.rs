use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use unpack::{Options, detect, extract, list};

/// 압축 해제 프로그램 — zip, 7z, tar, tar.gz, tar.bz2, tar.xz, tar.zst, gz, bz2, xz, zst 지원
#[derive(Parser, Debug)]
#[command(name = "unpack", version, about)]
struct Cli {
    /// 해제할 압축 파일 (여러 개 가능)
    #[arg(required = true)]
    archives: Vec<PathBuf>,

    /// 출력 폴더 (기본값: 압축 파일 옆에 확장자를 뺀 이름의 폴더)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// 해제하지 않고 내용 목록만 표시
    #[arg(short, long)]
    list: bool,

    /// 이미 존재하는 파일 덮어쓰기
    #[arg(short = 'f', long)]
    overwrite: bool,

    /// 암호 (zip, 7z)
    #[arg(short, long)]
    password: Option<String>,

    /// 해제되는 파일 이름 출력
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let opts = Options { overwrite: cli.overwrite, password: cli.password, verbose: cli.verbose };
    let many = cli.archives.len() > 1;
    let mut failed = 0;

    for archive in &cli.archives {
        // 여러 파일을 같은 출력 폴더로 풀 때는 각자 하위 폴더로 분리
        let dest = cli.output.as_ref().map(|o| {
            if many { o.join(unpack::base_name(archive)) } else { o.clone() }
        });

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
            extract(archive, dest.as_deref(), &opts).map(|s| {
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
            eprintln!("오류: {}: {e:#}", archive.display());
            failed += 1;
        }
    }

    if failed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
