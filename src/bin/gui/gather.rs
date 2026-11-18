//! 여러 파일을 선택하고 우클릭 메뉴를 누르면 Windows 탐색기는 파일마다 프로그램을
//! 따로 실행합니다. 압축하기는 선택한 파일 전체를 한 압축 파일로 만들어야 하므로,
//! 비슷한 시점에 실행된 프로세스들이 임시 폴더에 경로를 적어 두고 그중 하나(리더)가
//! 모두 모아서 처리합니다. 나머지 프로세스는 창을 띄우지 않고 바로 끝납니다.
//!
//! Linux 파일 관리자는 선택한 파일을 한 번에 넘겨 주지만, 같은 방식으로 동작해도
//! 결과는 같습니다(짧은 대기만 추가됨).

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

const POLL: Duration = Duration::from_millis(150);
/// 새 경로가 이만큼 들어오지 않으면 다 모였다고 봅니다.
const QUIET: Duration = Duration::from_millis(500);
/// 리더가 비정상 종료했을 때 남은 잠금을 무시하는 시간
const STALE: Duration = Duration::from_secs(15);

/// 리더이면 `Some(모든 경로)`, 다른 프로세스가 처리할 것이면 `None`.
pub fn gather(tag: &str, paths: Vec<PathBuf>) -> Option<Vec<PathBuf>> {
    let Some(first) = paths.first() else { return Some(paths) };
    let mut h = DefaultHasher::new();
    first.parent().unwrap_or(Path::new("")).hash(&mut h);
    let dir = std::env::temp_dir().join(unpack::APP_NAME).join(format!("gather-{tag}-{:016x}", h.finish()));

    for _ in 0..5 {
        match try_gather(&dir, &paths) {
            Ok(result) => return result,
            // 리더가 막 폴더를 지운 경우 등: 처음부터 다시
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(_) => break,
        }
    }
    // 모으기에 실패하면 받은 경로만 처리
    Some(paths)
}

fn try_gather(dir: &Path, paths: &[PathBuf]) -> std::io::Result<Option<Vec<PathBuf>>> {
    fs::create_dir_all(dir)?;
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mine = dir.join(format!("{}-{nanos}.list", std::process::id()));
    let body: String = paths.iter().map(|p| format!("{}\n", p.display())).collect();
    fs::write(&mine, body)?;

    let leader = dir.join("leader.lock");
    match OpenOptions::new().write(true).create_new(true).open(&leader) {
        Ok(mut f) => {
            let _ = writeln!(f, "{}", std::process::id());
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            let stale = fs::metadata(&leader)
                .and_then(|m| m.modified())
                .map(|t| t.elapsed().unwrap_or_default() > STALE)
                .unwrap_or(false);
            if stale {
                let _ = fs::remove_file(&leader);
                let _ = fs::remove_file(&mine);
                return Err(ErrorKind::NotFound.into());
            }
            return Ok(None);
        }
        Err(e) => return Err(e),
    }

    // 리더: 더 이상 새 목록이 들어오지 않을 때까지 기다림
    let mut count = 0;
    let mut last_change = Instant::now();
    loop {
        sleep(POLL);
        let n = list_files(dir).len();
        if n != count {
            count = n;
            last_change = Instant::now();
        } else if last_change.elapsed() >= QUIET {
            break;
        }
    }

    let mut all = Vec::new();
    for file in list_files(dir) {
        if let Ok(text) = fs::read_to_string(&file) {
            all.extend(text.lines().filter(|l| !l.is_empty()).map(PathBuf::from));
        }
    }
    let _ = fs::remove_dir_all(dir);
    all.sort();
    all.dedup();
    Ok(Some(all))
}

fn list_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "list"))
                .collect()
        })
        .unwrap_or_default()
}
