//! 파일 작업: 복사 · 이동 · 이름 바꾸기 · 새 폴더 · 휴지통.
//!
//! - 같은 이름이 있으면 덮어쓰지 않고 `이름 (2)`로 만듭니다.
//! - 폴더를 자기 자신 안으로 옮기거나 복사하는 것은 막습니다.
//! - 이동은 같은 드라이브면 이름만 바꾸고(즉시), 다른 드라이브면 복사한 뒤 원본을 지웁니다.
//! - 진행률과 취소를 지원하고, 취소되면 쓰다 만 파일을 지웁니다.

use anyhow::{Context, Result, bail};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use unpack::{Cancelled, Progress, unique_path};

fn size_of(path: &Path) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::read_dir(path).map(|rd| rd.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// `srcs`를 `dest` 폴더로 복사(`mv == false`) 또는 이동합니다. 만든 경로들을 돌려줍니다.
pub fn transfer(srcs: &[PathBuf], dest: &Path, mv: bool, progress: &Progress) -> Result<Vec<PathBuf>> {
    if !dest.is_dir() {
        bail!("대상 폴더가 없습니다: {}", dest.display());
    }
    let dest_abs = fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf());
    for s in srcs {
        let s_abs = fs::canonicalize(s).unwrap_or_else(|_| s.clone());
        if s.is_dir() && dest_abs.starts_with(&s_abs) {
            bail!("폴더를 자기 자신 안으로 {}할 수 없습니다: {}", if mv { "옮길" } else { "복사할" }, crate::file_name(s));
        }
    }
    progress.reset(srcs.iter().map(|s| size_of(s)).sum());

    let mut made = Vec::new();
    for src in srcs {
        if progress.is_cancelled() {
            return Err(Cancelled.into());
        }
        let name = src.file_name().with_context(|| format!("이름이 없는 경로: {}", src.display()))?;
        // 이미 그 폴더에 있는 것을 이동하면 할 일 없음
        if mv && src.parent().map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())) == Some(dest_abs.clone()) {
            progress.add(size_of(src));
            continue;
        }
        let target = unique_path(&dest.join(name));
        progress.set_current(&name.to_string_lossy());
        if mv && fs::rename(src, &target).is_ok() {
            progress.add(size_of(&target));
            made.push(target);
            continue;
        }
        // 다른 드라이브로 이동하거나 복사: 복사한 뒤 (이동이면) 원본 삭제
        if let Err(e) = copy_tree(src, &target, progress) {
            let _ = if target.is_dir() { fs::remove_dir_all(&target) } else { fs::remove_file(&target) };
            return Err(e);
        }
        if mv {
            let r = if src.is_dir() { fs::remove_dir_all(src) } else { fs::remove_file(src) };
            r.with_context(|| format!("복사는 했지만 원본을 지우지 못했습니다: {}", src.display()))?;
        }
        made.push(target);
    }
    Ok(made)
}

fn copy_tree(src: &Path, target: &Path, progress: &Progress) -> Result<()> {
    let meta = fs::symlink_metadata(src).with_context(|| format!("읽을 수 없음: {}", src.display()))?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        {
            let link = fs::read_link(src)?;
            std::os::unix::fs::symlink(link, target)?;
        }
        return Ok(());
    }
    if meta.is_dir() {
        fs::create_dir(target).with_context(|| format!("폴더를 만들 수 없음: {}", target.display()))?;
        for e in fs::read_dir(src)?.flatten() {
            copy_tree(&e.path(), &target.join(e.file_name()), progress)?;
        }
        return Ok(());
    }
    let mut from = fs::File::open(src).with_context(|| format!("열 수 없음: {}", src.display()))?;
    let mut to = fs::File::create(target).with_context(|| format!("만들 수 없음: {}", target.display()))?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if progress.is_cancelled() {
            drop(to);
            let _ = fs::remove_file(target);
            return Err(Cancelled.into());
        }
        let n = from.read(&mut buf)?;
        if n == 0 {
            break;
        }
        to.write_all(&buf[..n])?;
        progress.add(n as u64);
    }
    // 수정 시간·권한 유지
    if let Ok(t) = meta.modified() {
        let _ = to.set_modified(t);
    }
    drop(to);
    let _ = fs::set_permissions(target, meta.permissions());
    Ok(())
}

/// 이름 바꾸기 (같은 폴더 안에서). 새 경로를 돌려줍니다.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains(['/', '\\']) || new_name == "." || new_name == ".." {
        bail!("쓸 수 없는 이름입니다");
    }
    if cfg!(windows) && new_name.contains([':', '*', '?', '"', '<', '>', '|']) {
        bail!("이름에 \\ / : * ? \" < > | 는 쓸 수 없습니다");
    }
    let target = path.with_file_name(new_name);
    if target == path {
        return Ok(target);
    }
    // 대소문자만 바꾸는 경우(Windows)는 같은 파일이므로 허용
    let same = fs::canonicalize(&target).ok() == fs::canonicalize(path).ok();
    if target.exists() && !same {
        bail!("같은 이름이 이미 있습니다: {new_name}");
    }
    fs::rename(path, &target).with_context(|| format!("이름을 바꾸지 못했습니다: {}", crate::file_name(path)))?;
    Ok(target)
}

/// `dir` 안에 "새 폴더"(겹치면 "새 폴더 (2)")를 만듭니다.
pub fn new_folder(dir: &Path) -> Result<PathBuf> {
    let p = unique_path(&dir.join("새 폴더"));
    fs::create_dir(&p).with_context(|| format!("폴더를 만들 수 없음: {}", p.display()))?;
    Ok(p)
}

/// 휴지통으로 보내기 (영구 삭제는 하지 않음)
pub fn to_trash(paths: &[PathBuf]) -> Result<()> {
    trash::delete_all(paths).map_err(|e| anyhow::anyhow!("휴지통으로 보내지 못했습니다: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("왼쪽");
        let b = t.path().join("오른쪽");
        fs::create_dir_all(a.join("사진/여행")).unwrap();
        fs::write(a.join("사진/여행/바다.jpg"), vec![1u8; 300_000]).unwrap();
        fs::write(a.join("메모.txt"), "메모").unwrap();
        fs::create_dir_all(&b).unwrap();
        (t, a, b)
    }

    #[test]
    fn copy_keeps_source_and_names_conflicts() {
        let (_t, a, b) = setup();
        let p = Progress::new();
        let made = transfer(&[a.join("사진"), a.join("메모.txt")], &b, false, &p).unwrap();
        assert_eq!(made, vec![b.join("사진"), b.join("메모.txt")]);
        assert_eq!(fs::read(b.join("사진/여행/바다.jpg")).unwrap().len(), 300_000);
        assert!(a.join("메모.txt").exists());
        assert_eq!(p.done(), p.total());
        // 한 번 더 복사하면 (2)
        let made = transfer(&[a.join("메모.txt")], &b, false, &Progress::new()).unwrap();
        assert_eq!(made, vec![b.join("메모 (2).txt")]);
    }

    #[test]
    fn move_removes_source() {
        let (_t, a, b) = setup();
        transfer(&[a.join("사진")], &b, true, &Progress::new()).unwrap();
        assert!(!a.join("사진").exists());
        assert!(b.join("사진/여행/바다.jpg").exists());
        // 같은 폴더로 이동은 아무 일도 안 함
        let made = transfer(&[b.join("사진")], &b, true, &Progress::new()).unwrap();
        assert!(made.is_empty());
        assert!(b.join("사진").exists() && !b.join("사진 (2)").exists());
    }

    #[test]
    fn refuses_folder_into_itself() {
        let (_t, a, _b) = setup();
        let e = transfer(&[a.join("사진")], &a.join("사진/여행"), true, &Progress::new()).unwrap_err();
        assert!(e.to_string().contains("자기 자신"), "{e}");
        assert!(a.join("사진/여행/바다.jpg").exists());
    }

    #[test]
    fn cancel_leaves_no_partial_copy() {
        let (_t, a, b) = setup();
        let p = Progress::new();
        p.cancel();
        let e = transfer(&[a.join("사진")], &b, false, &p).unwrap_err();
        assert!(unpack::is_cancelled(&e));
        assert!(!b.join("사진").exists());
        assert!(a.join("사진/여행/바다.jpg").exists());
    }

    #[test]
    fn rename_and_new_folder() {
        let (_t, a, _b) = setup();
        let p = rename(&a.join("메모.txt"), "할일.txt").unwrap();
        assert_eq!(p, a.join("할일.txt"));
        assert!(rename(&p, "사진").is_err()); // 이미 있음
        assert!(rename(&p, "a/b").is_err());
        let n1 = new_folder(&a).unwrap();
        let n2 = new_folder(&a).unwrap();
        assert_eq!((n1.file_name().unwrap(), n2.file_name().unwrap()), ("새 폴더".as_ref(), "새 폴더 (2)".as_ref()));
    }
}
