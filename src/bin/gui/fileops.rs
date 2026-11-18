//! 파일 작업: 복사 · 이동 · 이름 바꾸기(여러 개 한꺼번에도) · 새 폴더 · 휴지통 · 되돌리기.
//!
//! - 같은 이름이 있으면 [`OnConflict`]에 따라 둘 다 유지(`이름 (2)`) · 덮어쓰기 · 건너뛰기.
//!   덮어쓸 때 원래 있던 파일은 휴지통으로 보냅니다(되살릴 수 있게). 폴더끼리는 내용을 합칩니다.
//! - 폴더를 자기 자신 안으로 옮기거나 복사하는 것은 막습니다.
//! - 이동은 같은 드라이브면 이름만 바꾸고(즉시), 다른 드라이브면 복사한 뒤 원본을 지웁니다.
//! - 진행률과 취소를 지원하고, 취소되면 쓰다 만 파일을 지웁니다.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use unpack::{Cancelled, Progress, unique_path};

/// 같은 이름이 이미 있을 때
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OnConflict {
    /// 새 이름 `이름 (2)`로 (기본)
    #[default]
    KeepBoth,
    /// 바꾸기 (원래 것은 휴지통으로, 폴더끼리는 내용 합치기)
    Overwrite,
    /// 이 항목은 건너뛰기
    Skip,
}

/// 복사·이동 결과
#[derive(Debug, Default)]
pub struct Transferred {
    /// (원래 경로, 새로 생긴 경로)
    pub pairs: Vec<(PathBuf, PathBuf)>,
    pub skipped: usize,
    /// 덮어쓰면서 휴지통으로 보낸 원래 파일들
    pub replaced: Vec<PathBuf>,
}

fn size_of(path: &Path) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::read_dir(path).map(|rd| rd.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn canon(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// `srcs` 중 `dest`에 같은 이름이 이미 있는 것 (같은 폴더 안 이동은 빼고)
pub fn conflicts(srcs: &[PathBuf], dest: &Path, mv: bool) -> Vec<PathBuf> {
    let dest_abs = canon(dest);
    srcs.iter()
        .filter(|s| {
            let Some(name) = s.file_name() else { return false };
            let same_dir = s.parent().map(canon) == Some(dest_abs.clone());
            if same_dir {
                return false; // 같은 폴더: 이동이면 할 일 없고, 복사면 언제나 (2)
            }
            let _ = mv;
            fs::symlink_metadata(dest.join(name)).is_ok()
        })
        .cloned()
        .collect()
}

/// `srcs`를 `dest` 폴더로 복사(`mv == false`) 또는 이동합니다.
/// `choice`에 없는 항목은 [`OnConflict::KeepBoth`].
pub fn transfer(srcs: &[PathBuf], dest: &Path, mv: bool, choice: &HashMap<PathBuf, OnConflict>, progress: &Progress) -> Result<Transferred> {
    if !dest.is_dir() {
        bail!("대상 폴더가 없습니다: {}", dest.display());
    }
    let dest_abs = canon(dest);
    for s in srcs {
        if s.is_dir() && dest_abs.starts_with(canon(s)) {
            bail!("폴더를 자기 자신 안으로 {}할 수 없습니다: {}", if mv { "옮길" } else { "복사할" }, crate::file_name(s));
        }
    }
    progress.reset(srcs.iter().map(|s| size_of(s)).sum());

    let mut out = Transferred::default();
    for src in srcs {
        if progress.is_cancelled() {
            return Err(Cancelled.into());
        }
        let name = src.file_name().with_context(|| format!("이름이 없는 경로: {}", src.display()))?;
        // 이미 그 폴더에 있는 것을 이동하면 할 일 없음
        if mv && src.parent().map(canon) == Some(dest_abs.clone()) {
            progress.add(size_of(src));
            continue;
        }
        let plain = dest.join(name);
        let exists = fs::symlink_metadata(&plain).is_ok();
        let how = if exists { choice.get(src).copied().unwrap_or_default() } else { OnConflict::KeepBoth };
        progress.set_current(&name.to_string_lossy());
        match how {
            OnConflict::Skip => {
                progress.add(size_of(src));
                out.skipped += 1;
            }
            OnConflict::Overwrite if src.is_dir() && plain.is_dir() => {
                merge_dir(src, &plain, mv, progress, &mut out)?;
                if mv {
                    let _ = remove_empty_tree(src);
                }
                out.pairs.push((src.clone(), plain));
            }
            OnConflict::Overwrite if src.is_dir() == plain.is_dir() => {
                put_aside(&plain, &mut out)?;
                move_or_copy(src, &plain, mv, progress)?;
                out.pairs.push((src.clone(), plain));
            }
            // 파일 ↔ 폴더처럼 종류가 다르면 덮어쓰지 않고 둘 다 유지
            _ => {
                let target = unique_path(&plain);
                move_or_copy(src, &target, mv, progress)?;
                out.pairs.push((src.clone(), target));
            }
        }
    }
    Ok(out)
}

/// 덮어쓸 원래 파일을 휴지통으로 (안 되면 지움)
fn put_aside(path: &Path, out: &mut Transferred) -> Result<()> {
    if trash::delete(path).is_err() {
        if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }
            .with_context(|| format!("덮어쓸 파일을 지우지 못했습니다: {}", path.display()))?;
    }
    out.replaced.push(path.to_path_buf());
    Ok(())
}

/// 폴더끼리 덮어쓰기: 안의 내용을 하나씩 합침 (같은 파일은 바꿈)
fn merge_dir(src: &Path, dest: &Path, mv: bool, progress: &Progress, out: &mut Transferred) -> Result<()> {
    for e in fs::read_dir(src)?.flatten() {
        let from = e.path();
        let to = dest.join(e.file_name());
        if progress.is_cancelled() {
            return Err(Cancelled.into());
        }
        match (from.is_dir(), fs::symlink_metadata(&to).map(|m| m.is_dir())) {
            (true, Ok(true)) => merge_dir(&from, &to, mv, progress, out)?,
            (a, Ok(b)) if a == b => {
                put_aside(&to, out)?;
                move_or_copy(&from, &to, mv, progress)?;
            }
            (_, Ok(_)) => move_or_copy(&from, &unique_path(&to), mv, progress)?,
            (_, Err(_)) => move_or_copy(&from, &to, mv, progress)?,
        }
    }
    Ok(())
}

fn remove_empty_tree(dir: &Path) -> std::io::Result<()> {
    for e in fs::read_dir(dir)?.flatten() {
        if e.path().is_dir() {
            let _ = remove_empty_tree(&e.path());
        }
    }
    fs::remove_dir(dir)
}

/// 하나를 옮기거나(같은 드라이브면 이름만 바꿈) 복사
fn move_or_copy(src: &Path, target: &Path, mv: bool, progress: &Progress) -> Result<()> {
    if mv && fs::rename(src, target).is_ok() {
        progress.add(size_of(target));
        return Ok(());
    }
    if let Err(e) = copy_tree(src, target, progress) {
        let _ = if target.is_dir() { fs::remove_dir_all(target) } else { fs::remove_file(target) };
        return Err(e);
    }
    if mv {
        let r = if src.is_dir() { fs::remove_dir_all(src) } else { fs::remove_file(src) };
        r.with_context(|| format!("복사는 했지만 원본을 지우지 못했습니다: {}", src.display()))?;
    }
    Ok(())
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

fn check_name(new_name: &str) -> Result<&str> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains(['/', '\\']) || new_name == "." || new_name == ".." {
        bail!("쓸 수 없는 이름입니다");
    }
    if cfg!(windows) && new_name.contains([':', '*', '?', '"', '<', '>', '|']) {
        bail!("이름에 \\ / : * ? \" < > | 는 쓸 수 없습니다");
    }
    Ok(new_name)
}

/// 이름 바꾸기 (같은 폴더 안에서). 새 경로를 돌려줍니다.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf> {
    let new_name = check_name(new_name)?;
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

/// 여러 개 이름 바꾸기에서 `i`번째(0부터)가 받을 이름: `기본 (시작+i).확장자`
pub fn numbered_name(path: &Path, base: &str, start: usize, i: usize) -> String {
    let ext = if path.is_dir() { None } else { path.extension().map(|e| e.to_string_lossy().into_owned()) };
    match ext {
        Some(e) => format!("{} ({}).{e}", base.trim(), start + i),
        None => format!("{} ({})", base.trim(), start + i),
    }
}

/// 여러 개를 `기본 (1).jpg`, `기본 (2).jpg` … 로 한꺼번에 바꿉니다. (원래, 새) 경로 쌍을 돌려줍니다.
/// 서로 이름이 겹쳐도 되도록 먼저 임시 이름으로 바꾼 뒤 최종 이름으로 바꿉니다.
pub fn batch_rename(paths: &[PathBuf], base: &str, start: usize) -> Result<Vec<(PathBuf, PathBuf)>> {
    check_name(base)?;
    let targets: Vec<PathBuf> = paths.iter().enumerate().map(|(i, p)| p.with_file_name(numbered_name(p, base, start, i))).collect();
    for t in &targets {
        if t.exists() && !paths.iter().any(|p| canon(p) == canon(t)) {
            bail!("같은 이름이 이미 있습니다: {}", crate::file_name(t));
        }
    }
    let tag = std::process::id();
    let mut temps = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        let tmp = p.with_file_name(format!(".opizen-rename-{tag}-{i}"));
        if let Err(e) = fs::rename(p, &tmp) {
            // 되돌리고 실패
            for (orig, t) in paths.iter().zip(&temps) {
                let _ = fs::rename(t, orig);
            }
            return Err(e).with_context(|| format!("이름을 바꾸지 못했습니다: {}", crate::file_name(p)));
        }
        temps.push(tmp);
    }
    let mut done = Vec::new();
    for ((orig, tmp), target) in paths.iter().zip(&temps).zip(&targets) {
        fs::rename(tmp, target).with_context(|| format!("이름을 바꾸지 못했습니다: {}", crate::file_name(orig)))?;
        done.push((orig.clone(), target.clone()));
    }
    Ok(done)
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

/// 휴지통으로 보낸 것들을 원래 자리로 되살립니다 (같은 경로 중 가장 최근 것).
pub fn restore_from_trash(paths: &[PathBuf]) -> Result<usize> {
    use trash::os_limited::{list, restore_all};
    let items = list().map_err(|e| anyhow::anyhow!("휴지통을 읽지 못했습니다: {e}"))?;
    let mut chosen = Vec::new();
    for p in paths {
        let best = items
            .iter()
            .filter(|it| it.original_path() == *p || canon(&it.original_path()) == canon(p))
            .max_by_key(|it| it.time_deleted);
        if let Some(it) = best {
            chosen.push(it.clone());
        }
    }
    if chosen.is_empty() {
        bail!("휴지통에서 찾지 못했습니다 (이미 비웠거나 되살렸을 수 있습니다)");
    }
    let n = chosen.len();
    restore_all(chosen).map_err(|e| anyhow::anyhow!("되살리지 못했습니다: {e}"))?;
    Ok(n)
}

/// 되돌릴 수 있는 작업 기록 (Ctrl+Z)
#[derive(Debug, Clone)]
pub enum Undo {
    /// (원래 경로, 옮긴 경로) (+ 덮어쓰면서 휴지통으로 보낸 원래 파일)
    Moved(Vec<(PathBuf, PathBuf)>, Vec<PathBuf>),
    /// 복사로 생긴 것들 (+ 덮어쓰면서 휴지통으로 보낸 원래 파일)
    Copied(Vec<PathBuf>, Vec<PathBuf>),
    /// (원래 경로, 바뀐 경로)
    Renamed(Vec<(PathBuf, PathBuf)>),
    NewFolder(PathBuf),
    Trashed(Vec<PathBuf>),
}

impl Undo {
    pub fn label(&self) -> String {
        match self {
            Undo::Moved(v, _) => format!("이동 {}개", v.len()),
            Undo::Copied(v, _) => format!("복사 {}개", v.len()),
            Undo::Renamed(v) => format!("이름 바꾸기 {}개", v.len()),
            Undo::NewFolder(p) => format!("새 폴더 {}", crate::file_name(p)),
            Undo::Trashed(v) => format!("휴지통으로 보낸 {}개", v.len()),
        }
    }

    /// 되돌리기. 다시 선택해 보여 줄 경로들을 돌려줍니다.
    pub fn revert(&self) -> Result<Vec<PathBuf>> {
        let p = Progress::new();
        match self {
            Undo::Moved(pairs, replaced) => {
                let mut back = Vec::new();
                for (orig, now) in pairs {
                    let target = if fs::symlink_metadata(orig).is_ok() { unique_path(orig) } else { orig.clone() };
                    if let Some(dir) = target.parent() {
                        fs::create_dir_all(dir)?;
                    }
                    move_or_copy(now, &target, true, &p)?;
                    back.push(target);
                }
                if !replaced.is_empty() {
                    restore_from_trash(replaced)?;
                }
                Ok(back)
            }
            Undo::Copied(made, replaced) => {
                to_trash(made)?;
                if !replaced.is_empty() {
                    restore_from_trash(replaced)?;
                }
                Ok(replaced.clone())
            }
            Undo::Renamed(pairs) => {
                // 여러 개였으면 서로 겹칠 수 있으니 임시 이름을 거침
                let tag = std::process::id();
                let mut temps = Vec::new();
                for (i, (_, now)) in pairs.iter().enumerate() {
                    let tmp = now.with_file_name(format!(".opizen-undo-{tag}-{i}"));
                    fs::rename(now, &tmp).with_context(|| format!("되돌리지 못했습니다: {}", crate::file_name(now)))?;
                    temps.push(tmp);
                }
                let mut back = Vec::new();
                for ((orig, _), tmp) in pairs.iter().zip(temps) {
                    let target = if orig.exists() { unique_path(orig) } else { orig.clone() };
                    fs::rename(&tmp, &target)?;
                    back.push(target);
                }
                Ok(back)
            }
            Undo::NewFolder(path) => {
                if fs::read_dir(path).map(|mut d| d.next().is_some()).unwrap_or(false) {
                    bail!("새 폴더에 이미 파일이 있어 되돌리지 않았습니다");
                }
                fs::remove_dir(path)?;
                Ok(vec![])
            }
            Undo::Trashed(paths) => {
                restore_from_trash(paths)?;
                Ok(paths.clone())
            }
        }
    }
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

    fn none() -> HashMap<PathBuf, OnConflict> {
        HashMap::new()
    }

    #[test]
    fn copy_keeps_source_and_names_conflicts() {
        let (_t, a, b) = setup();
        let p = Progress::new();
        let r = transfer(&[a.join("사진"), a.join("메모.txt")], &b, false, &none(), &p).unwrap();
        let made: Vec<_> = r.pairs.iter().map(|x| x.1.clone()).collect();
        assert_eq!(made, vec![b.join("사진"), b.join("메모.txt")]);
        assert_eq!(fs::read(b.join("사진/여행/바다.jpg")).unwrap().len(), 300_000);
        assert!(a.join("메모.txt").exists());
        assert_eq!(p.done(), p.total());
        // 한 번 더 복사하면 (2)
        assert_eq!(conflicts(&[a.join("메모.txt")], &b, false), vec![a.join("메모.txt")]);
        let r = transfer(&[a.join("메모.txt")], &b, false, &none(), &Progress::new()).unwrap();
        assert_eq!(r.pairs[0].1, b.join("메모 (2).txt"));
    }

    #[test]
    fn skip_and_overwrite() {
        let (_t, a, b) = setup();
        fs::write(b.join("메모.txt"), "예전").unwrap();
        let skip: HashMap<_, _> = [(a.join("메모.txt"), OnConflict::Skip)].into();
        let r = transfer(&[a.join("메모.txt")], &b, false, &skip, &Progress::new()).unwrap();
        assert_eq!((r.skipped, r.pairs.len()), (1, 0));
        assert_eq!(fs::read_to_string(b.join("메모.txt")).unwrap(), "예전");

        let over: HashMap<_, _> = [(a.join("메모.txt"), OnConflict::Overwrite)].into();
        let r = transfer(&[a.join("메모.txt")], &b, true, &over, &Progress::new()).unwrap();
        assert_eq!(r.pairs, vec![(a.join("메모.txt"), b.join("메모.txt"))]);
        assert_eq!(fs::read_to_string(b.join("메모.txt")).unwrap(), "메모");
        assert!(!a.join("메모.txt").exists());
        assert!(!b.join("메모 (2).txt").exists());
    }

    #[test]
    fn overwrite_merges_folders() {
        let (_t, a, b) = setup();
        fs::create_dir_all(b.join("사진/가족")).unwrap();
        fs::write(b.join("사진/가족/집.jpg"), "집").unwrap();
        fs::create_dir_all(b.join("사진/여행")).unwrap();
        fs::write(b.join("사진/여행/바다.jpg"), "옛 바다").unwrap();
        let over: HashMap<_, _> = [(a.join("사진"), OnConflict::Overwrite)].into();
        transfer(&[a.join("사진")], &b, true, &over, &Progress::new()).unwrap();
        assert_eq!(fs::read(b.join("사진/여행/바다.jpg")).unwrap().len(), 300_000);
        assert!(b.join("사진/가족/집.jpg").exists());
        assert!(!a.join("사진").exists());
    }

    #[test]
    fn move_removes_source_and_undo_moves_back() {
        let (_t, a, b) = setup();
        let r = transfer(&[a.join("사진")], &b, true, &none(), &Progress::new()).unwrap();
        assert!(!a.join("사진").exists());
        assert!(b.join("사진/여행/바다.jpg").exists());
        Undo::Moved(r.pairs, r.replaced).revert().unwrap();
        assert!(a.join("사진/여행/바다.jpg").exists());
        assert!(!b.join("사진").exists());
        // 같은 폴더로 이동은 아무 일도 안 함
        let r = transfer(&[a.join("사진")], &a, true, &none(), &Progress::new()).unwrap();
        assert!(r.pairs.is_empty());
    }

    #[test]
    fn refuses_folder_into_itself() {
        let (_t, a, _b) = setup();
        let e = transfer(&[a.join("사진")], &a.join("사진/여행"), true, &none(), &Progress::new()).unwrap_err();
        assert!(e.to_string().contains("자기 자신"), "{e}");
        assert!(a.join("사진/여행/바다.jpg").exists());
    }

    #[test]
    fn cancel_leaves_no_partial_copy() {
        let (_t, a, b) = setup();
        let p = Progress::new();
        p.cancel();
        let e = transfer(&[a.join("사진")], &b, false, &none(), &p).unwrap_err();
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
        Undo::NewFolder(n2.clone()).revert().unwrap();
        assert!(!n2.exists());
    }

    #[test]
    fn batch_rename_numbers_and_undo() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        for n in ["b.jpg", "a.jpg", "c.png", "여행 (1).jpg"] {
            fs::write(d.join(n), n).unwrap();
        }
        let files = vec![d.join("여행 (1).jpg"), d.join("a.jpg"), d.join("c.png")];
        let done = batch_rename(&files, "여행", 1).unwrap();
        // 원래 "여행 (1).jpg"가 이미 그 이름이어도 겹치지 않게 처리
        assert_eq!(fs::read_to_string(d.join("여행 (1).jpg")).unwrap(), "여행 (1).jpg");
        assert_eq!(fs::read_to_string(d.join("여행 (2).jpg")).unwrap(), "a.jpg");
        assert_eq!(fs::read_to_string(d.join("여행 (3).png")).unwrap(), "c.png");
        // b.jpg가 있는데 그 이름으로 바꾸려 하면 거절
        assert!(batch_rename(&[d.join("여행 (2).jpg")], "b", 0).is_ok_and(|v| v[0].1 == d.join("b (0).jpg")));
        Undo::Renamed(vec![(d.join("여행 (2).jpg"), d.join("b (0).jpg"))]).revert().unwrap();
        Undo::Renamed(done).revert().unwrap();
        for n in ["a.jpg", "c.png", "여행 (1).jpg", "b.jpg"] {
            assert_eq!(fs::read_to_string(d.join(n)).unwrap(), n);
        }
        fs::write(d.join("x (1).jpg"), "").unwrap();
        assert!(batch_rename(&[d.join("a.jpg")], "x", 1).is_err());
    }
}
