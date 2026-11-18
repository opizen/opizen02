//! 왼쪽 사이드바: 즐겨찾기(홈·바탕 화면·문서·다운로드·사진)와 드라이브.

use std::path::{Path, PathBuf};

pub struct Place {
    pub label: String,
    pub path: PathBuf,
    pub drive: bool,
    /// 사용자가 추가한 즐겨찾기 (뺄 수 있음)
    pub pinned: bool,
}

pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }))
}

/// Linux: ~/.config/user-dirs.dirs 에 적힌 폴더 (한국어 환경이면 "바탕화면" 등)
#[cfg(not(windows))]
fn xdg_dir(key: &str) -> Option<PathBuf> {
    let home = home();
    let text = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).ok()?;
    let line = text.lines().find(|l| l.starts_with(&format!("XDG_{key}_DIR=")))?;
    let v = line.split_once('=')?.1.trim().trim_matches('"');
    let p = PathBuf::from(v.replace("$HOME", &home.to_string_lossy()));
    p.is_dir().then_some(p)
}

/// 즐겨찾기 (있는 폴더만)
pub fn favorites() -> Vec<Place> {
    let home = home();
    let mut out = vec![Place { label: "홈".into(), path: home.clone(), drive: false, pinned: false }];
    let list: [(&str, &str, &str); 4] = [
        ("바탕 화면", "Desktop", "DESKTOP"),
        ("문서", "Documents", "DOCUMENTS"),
        ("다운로드", "Downloads", "DOWNLOAD"),
        ("사진", "Pictures", "PICTURES"),
    ];
    for (label, folder, _xdg) in list {
        #[cfg(not(windows))]
        let path = xdg_dir(_xdg).unwrap_or_else(|| home.join(folder));
        #[cfg(windows)]
        let path = home.join(folder);
        if path.is_dir() && path != home {
            out.push(Place { label: label.into(), path, drive: false, pinned: false });
        }
    }
    out
}

/// 기본 즐겨찾기 + 사용자가 추가한 폴더 (없어진 폴더는 뺌, 겹치면 한 번만)
pub fn favorites_with(pins: &[PathBuf]) -> Vec<Place> {
    let mut out = favorites();
    for p in pins {
        if p.is_dir() && !out.iter().any(|x| &x.path == p) {
            let label = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
            out.push(Place { label, path: p.clone(), drive: false, pinned: true });
        }
    }
    out
}

/// 드라이브 (Windows: C:, D: … / Linux: 루트와 연결된 저장 장치)
pub fn drives() -> Vec<Place> {
    let mut out = Vec::new();
    if cfg!(windows) {
        // A:, B: 는 플로피 드라이브라 확인하면 느려질 수 있어 건너뜀
        for c in 'C'..='Z' {
            let p = PathBuf::from(format!("{c}:\\"));
            if p.exists() {
                out.push(Place { label: format!("{c}: 드라이브"), path: p, drive: true, pinned: false });
            }
        }
    } else {
        out.push(Place { label: "컴퓨터 (/)".into(), path: PathBuf::from("/"), drive: true, pinned: false });
        let user = std::env::var("USER").unwrap_or_default();
        for base in [format!("/media/{user}"), "/run/media".to_string(), "/mnt".to_string()] {
            if let Ok(rd) = std::fs::read_dir(&base) {
                for e in rd.flatten().filter(|e| e.path().is_dir()) {
                    out.push(Place { label: e.file_name().to_string_lossy().into_owned(), path: e.path(), drive: true, pinned: false });
                }
            }
        }
    }
    out
}

/// 경로를 주소 줄 조각으로: ("C:", C:\), ("사용자", C:\사용자) …
pub fn crumbs(path: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = path
        .ancestors()
        .map(|p| {
            let label = match p.file_name() {
                Some(n) => n.to_string_lossy().into_owned(),
                None => {
                    let s = p.display().to_string();
                    let s = s.trim_end_matches(['\\', '/']).to_string();
                    if s.is_empty() { "/".into() } else { s }
                }
            };
            (label, p.to_path_buf())
        })
        .filter(|(l, _)| !l.is_empty())
        .collect();
    v.reverse();
    v
}
