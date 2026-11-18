use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use tempfile::tempdir;
use unpack::{Options, extract, list};

fn make_tar(builder_out: impl Write) -> impl Write {
    let mut b = tar::Builder::new(builder_out);
    let data = b"hello tar";
    let mut h = tar::Header::new_gnu();
    h.set_size(data.len() as u64);
    h.set_mode(0o644);
    h.set_cksum();
    b.append_data(&mut h, "dir/hello.txt", &data[..]).unwrap();
    b.into_inner().unwrap()
}

#[test]
fn zip_roundtrip() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("a.zip");
    let mut z = zip::ZipWriter::new(File::create(&path).unwrap());
    let o = zip::write::SimpleFileOptions::default();
    z.add_directory("sub/", o).unwrap();
    z.start_file("sub/x.txt", o).unwrap();
    z.write_all(b"zip data").unwrap();
    z.finish().unwrap();

    let s = extract(&path, None, &Options::default()).unwrap();
    assert_eq!(s.files, 1);
    assert_eq!(fs::read_to_string(tmp.path().join("a/sub/x.txt")).unwrap(), "zip data");
    assert_eq!(list(&path, &Options::default()).unwrap().len(), 2);
}

#[test]
fn zip_slip_is_blocked() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("evil.zip");
    let mut z = zip::ZipWriter::new(File::create(&path).unwrap());
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("../escaped.txt", o).unwrap();
    z.write_all(b"bad").unwrap();
    z.start_file("ok.txt", o).unwrap();
    z.write_all(b"good").unwrap();
    z.finish().unwrap();

    let out = tmp.path().join("out");
    let s = extract(&path, Some(&out), &Options::default()).unwrap();
    assert_eq!(s.skipped, 1);
    assert!(!tmp.path().join("escaped.txt").exists());
    assert!(out.join("ok.txt").exists());
}

#[test]
fn tar_gz_and_tar_slip() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("b.tar.gz");
    let gz = flate2::write::GzEncoder::new(File::create(&path).unwrap(), Default::default());
    make_tar(gz);

    extract(&path, None, &Options::default()).unwrap();
    assert_eq!(fs::read_to_string(tmp.path().join("b/dir/hello.txt")).unwrap(), "hello tar");

    // 경로 탈출 항목이 들어 있는 tar (헤더 이름을 직접 기록)
    let evil = tmp.path().join("evil.tar");
    let mut b = tar::Builder::new(File::create(&evil).unwrap());
    let mut h = tar::Header::new_old();
    h.as_old_mut().name[..9].copy_from_slice(b"../x.txt\0");
    h.set_size(3);
    h.set_cksum();
    b.append(&h, &b"bad"[..]).unwrap();
    b.finish().unwrap();
    let s = extract(&evil, None, &Options::default()).unwrap();
    assert_eq!(s.skipped, 1);
    assert!(!tmp.path().join("x.txt").exists());
}

#[test]
fn compressed_tar_detected_by_content() {
    // 확장자가 .xz 뿐이어도 내용이 tar이면 tar로 해제
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("c.xz");
    let enc = xz2::write::XzEncoder::new(File::create(&path).unwrap(), 6);
    make_tar(enc);
    let s = extract(&path, None, &Options::default()).unwrap();
    assert_eq!(s.files, 1);
    assert!(tmp.path().join("c/dir/hello.txt").exists());
}

#[test]
fn single_stream_files() {
    let tmp = tempdir().unwrap();
    let p = tmp.path();

    let mut gz = flate2::write::GzEncoder::new(File::create(p.join("log.txt.gz")).unwrap(), Default::default());
    gz.write_all(b"gzip!").unwrap();
    gz.finish().unwrap();

    let mut bz = bzip2::write::BzEncoder::new(File::create(p.join("b.txt.bz2")).unwrap(), Default::default());
    bz.write_all(b"bzip2!").unwrap();
    bz.finish().unwrap();

    let zst = zstd::encode_all(&b"zstd!"[..], 3).unwrap();
    fs::write(p.join("z.txt.zst"), zst).unwrap();

    for (arc, out, want) in [("log.txt.gz", "log.txt", "gzip!"), ("b.txt.bz2", "b.txt", "bzip2!"), ("z.txt.zst", "z.txt", "zstd!")] {
        extract(&p.join(arc), None, &Options::default()).unwrap();
        assert_eq!(fs::read_to_string(p.join(out)).unwrap(), want);
    }
}

#[test]
fn magic_detection_without_extension() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("mystery");
    let mut z = zip::ZipWriter::new(File::create(&path).unwrap());
    z.start_file("m.txt", zip::write::SimpleFileOptions::default()).unwrap();
    z.write_all(b"magic").unwrap();
    z.finish().unwrap();
    extract(&path, None, &Options::default()).unwrap();
    assert!(tmp.path().join("mystery.out/m.txt").exists());
}

#[test]
fn overwrite_policy() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("d.tar");
    make_tar(File::create(&path).unwrap());
    let target = tmp.path().join("d/dir/hello.txt");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "keep").unwrap();

    let s = extract(&path, None, &Options::default()).unwrap();
    assert_eq!(s.skipped, 1);
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep");

    let opts = Options { overwrite: true, ..Default::default() };
    extract(&path, None, &opts).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "hello tar");
}

#[test]
fn sevenz_roundtrip() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join("inner")).unwrap();
    fs::write(src.join("inner/s.txt"), "seven").unwrap();
    let path = tmp.path().join("e.7z");
    sevenz_rust2::compress_to_path(&src, &path).unwrap();

    extract(&path, None, &Options::default()).unwrap();
    let found = walk(&tmp.path().join("e")).into_iter().find(|p| p.ends_with("inner/s.txt"));
    assert_eq!(fs::read_to_string(found.expect("s.txt 없음")).unwrap(), "seven");
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() { out.extend(walk(&p)) } else { out.push(p) }
    }
    out
}

// ---------------------------------------------------------------------------
// 우클릭 메뉴용 해제 (extract_auto)
// ---------------------------------------------------------------------------

use unpack::{Mode, extract_auto, is_password_error};

fn zip_with(path: &Path, files: &[(&str, &str)]) {
    let mut z = zip::ZipWriter::new(File::create(path).unwrap());
    for (name, body) in files {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    z.finish().unwrap();
}

fn no_temp_left(dir: &Path) -> bool {
    fs::read_dir(dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".unpack-tmp"))
}

#[test]
fn smart_single_root_goes_directly_into_parent() {
    let tmp = tempdir().unwrap();
    let p = tmp.path();
    let arc = p.join("project-v1.zip");
    zip_with(&arc, &[("project/a.txt", "A"), ("project/sub/b.txt", "B")]);

    let s = extract_auto(&arc, p, Mode::Smart, &Options::default()).unwrap();
    assert_eq!(s.output, p.join("project"));
    assert_eq!(fs::read_to_string(p.join("project/sub/b.txt")).unwrap(), "B");
    assert!(no_temp_left(p));

    // 같은 이름이 이미 있으면 압축 파일 이름 폴더로 (겹치지 않게)
    let s = extract_auto(&arc, p, Mode::Smart, &Options::default()).unwrap();
    assert_eq!(s.output, p.join("project-v1"));
    assert!(p.join("project-v1/project/a.txt").exists());
    let s = extract_auto(&arc, p, Mode::Smart, &Options::default()).unwrap();
    assert_eq!(s.output, p.join("project-v1 (2)"));
    assert!(no_temp_left(p));
}

#[test]
fn smart_many_roots_get_a_folder() {
    let tmp = tempdir().unwrap();
    let p = tmp.path();
    let arc = p.join("loose.zip");
    zip_with(&arc, &[("a.txt", "A"), ("b.txt", "B")]);
    let s = extract_auto(&arc, p, Mode::Smart, &Options::default()).unwrap();
    assert_eq!(s.output, p.join("loose"));
    assert!(p.join("loose/a.txt").exists() && p.join("loose/b.txt").exists());
    assert!(!p.join("a.txt").exists());
}

#[test]
fn new_folder_mode_never_overwrites() {
    let tmp = tempdir().unwrap();
    let p = tmp.path();
    let arc = p.join("data.zip");
    zip_with(&arc, &[("only/x.txt", "X")]);
    fs::create_dir(p.join("data")).unwrap();
    let s = extract_auto(&arc, p, Mode::NewFolder, &Options::default()).unwrap();
    assert_eq!(s.output, p.join("data (2)"));
    assert!(p.join("data (2)/only/x.txt").exists());
}

#[test]
fn auto_single_stream_gets_unique_name() {
    let tmp = tempdir().unwrap();
    let p = tmp.path();
    let mut gz = flate2::write::GzEncoder::new(File::create(p.join("log.txt.gz")).unwrap(), Default::default());
    gz.write_all(b"new").unwrap();
    gz.finish().unwrap();
    fs::write(p.join("log.txt"), "old").unwrap();
    let s = extract_auto(&p.join("log.txt.gz"), p, Mode::Smart, &Options::default()).unwrap();
    assert_eq!(s.output, p.join("log (2).txt"));
    assert_eq!(fs::read_to_string(p.join("log.txt")).unwrap(), "old");
    assert_eq!(fs::read_to_string(p.join("log (2).txt")).unwrap(), "new");
}

#[test]
fn encrypted_zip_reports_password_error_and_cleans_up() {
    let tmp = tempdir().unwrap();
    let p = tmp.path();
    let arc = p.join("secret.zip");
    let mut z = zip::ZipWriter::new(File::create(&arc).unwrap());
    let o = zip::write::SimpleFileOptions::default().with_aes_encryption(zip::AesMode::Aes256, "pw123");
    z.start_file("s.txt", o).unwrap();
    z.write_all(b"top secret").unwrap();
    z.finish().unwrap();

    let e = extract_auto(&arc, p, Mode::Smart, &Options::default()).unwrap_err();
    assert!(is_password_error(&e), "{e:#}");
    assert!(no_temp_left(p));

    let wrong = Options { password: Some("nope".into()), ..Default::default() };
    let e = extract_auto(&arc, p, Mode::Smart, &wrong).unwrap_err();
    assert!(is_password_error(&e), "{e:#}");

    let right = Options { password: Some("pw123".into()), ..Default::default() };
    let s = extract_auto(&arc, p, Mode::Smart, &right).unwrap();
    assert_eq!(fs::read_to_string(&s.output).unwrap(), "top secret");
}

#[test]
fn encrypted_7z_reports_password_error() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("s.txt"), "seven secret").unwrap();
    let arc = tmp.path().join("secret.7z");
    sevenz_rust2::compress_to_path_encrypted(&src, &arc, "pw".into()).unwrap();

    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();
    let e = extract_auto(&arc, &out, Mode::NewFolder, &Options::default()).unwrap_err();
    assert!(is_password_error(&e), "{e:#}");
    let right = Options { password: Some("pw".into()), ..Default::default() };
    extract_auto(&arc, &out, Mode::NewFolder, &right).unwrap();
}

// ---------------------------------------------------------------------------
// 압축하기
// ---------------------------------------------------------------------------

use unpack::{ArchiveKind, compress, compress_with, default_archive_path};

fn sample_tree(root: &Path) -> Vec<std::path::PathBuf> {
    fs::create_dir_all(root.join("사진/여행")).unwrap();
    fs::write(root.join("사진/여행/바다.txt"), "파도").unwrap();
    fs::write(root.join("사진/메모.txt"), "memo").unwrap();
    fs::write(root.join("보고서.docx"), vec![7u8; 100_000]).unwrap();
    vec![root.join("사진"), root.join("보고서.docx")]
}

#[test]
fn compress_roundtrip_all_formats() {
    for kind in ArchiveKind::ALL {
        let tmp = tempdir().unwrap();
        let inputs = sample_tree(tmp.path());
        let out = default_archive_path(&inputs, kind);
        let s = compress(&inputs, &out, kind, None).unwrap();
        assert_eq!((s.files, s.dirs), (3, 2), "{kind:?}");
        assert!(!out.with_file_name(format!("{}.part", out.file_name().unwrap().to_string_lossy())).exists());

        let dest = tmp.path().join("check");
        extract(&out, Some(&dest), &Options::default()).unwrap();
        assert_eq!(fs::read_to_string(dest.join("사진/여행/바다.txt")).unwrap(), "파도", "{kind:?}");
        assert_eq!(fs::read(dest.join("보고서.docx")).unwrap().len(), 100_000, "{kind:?}");
    }
}

#[test]
fn compress_with_password() {
    for kind in [ArchiveKind::Zip, ArchiveKind::SevenZ] {
        let tmp = tempdir().unwrap();
        let inputs = sample_tree(tmp.path());
        let out = tmp.path().join(format!("secret{}", kind.ext()));
        compress(&inputs, &out, kind, Some("비밀123")).unwrap();

        let e = extract(&out, Some(&tmp.path().join("x")), &Options::default()).unwrap_err();
        assert!(is_password_error(&e), "{kind:?}: {e:#}");
        let opts = Options { password: Some("비밀123".into()), ..Default::default() };
        let dest = tmp.path().join("ok");
        extract(&out, Some(&dest), &opts).unwrap();
        assert_eq!(fs::read_to_string(dest.join("사진/메모.txt")).unwrap(), "memo", "{kind:?}");
    }
    let tmp = tempdir().unwrap();
    let inputs = sample_tree(tmp.path());
    assert!(compress(&inputs, &tmp.path().join("a.tar.gz"), ArchiveKind::TarGz, Some("pw")).is_err());
}

#[test]
fn default_archive_names() {
    let tmp = tempdir().unwrap();
    let p = tmp.path();
    let inputs = sample_tree(p);
    assert_eq!(default_archive_path(&inputs[..1], ArchiveKind::Zip), p.join("사진.zip"));
    assert_eq!(default_archive_path(&inputs[1..], ArchiveKind::SevenZ), p.join("보고서.7z"));
    let parent_name = fs::canonicalize(p).unwrap().file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(default_archive_path(&inputs, ArchiveKind::Zip), p.join(format!("{parent_name}.zip")));
    fs::write(p.join("사진.tar.gz"), "").unwrap();
    assert_eq!(default_archive_path(&inputs[..1], ArchiveKind::TarGz), p.join("사진 (2).tar.gz"));
}

#[test]
fn here_mode_extracts_in_place_without_overwriting() {
    let tmp = tempdir().unwrap();
    let p = tmp.path();
    let arc = p.join("loose.zip");
    zip_with(&arc, &[("a.txt", "new A"), ("b.txt", "B")]);
    fs::write(p.join("a.txt"), "old A").unwrap();
    let s = extract_auto(&arc, p, Mode::Here, &Options::default()).unwrap();
    assert_eq!(s.output, p);
    assert_eq!(fs::read_to_string(p.join("a.txt")).unwrap(), "old A");
    assert_eq!(fs::read_to_string(p.join("b.txt")).unwrap(), "B");
    assert_eq!(s.skipped, 1);
    assert!(!p.join("loose").exists());
}

// ---------------------------------------------------------------------------
// 버그 수정: 한글(CP949) 이름, 수정 시간 유지
// ---------------------------------------------------------------------------

#[test]
fn cp949_zip_names_are_decoded() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cp949.zip");
    let names: Vec<String> = list(&fixture, &Options::default()).unwrap().into_iter().map(|e| e.name).collect();
    assert!(names.contains(&"한글폴더/한글문서.txt".to_string()), "{names:?}");
    assert!(names.contains(&"영문.txt".to_string()), "{names:?}");

    let tmp = tempdir().unwrap();
    extract(&fixture, Some(tmp.path()), &Options::default()).unwrap();
    assert_eq!(fs::read_to_string(tmp.path().join("한글폴더/한글문서.txt")).unwrap(), "안녕");
}

#[test]
fn utf8_names_still_work() {
    assert_eq!(unpack::decode_zip_name("사진/바다.jpg".as_bytes()).as_deref(), Some("사진/바다.jpg"));
    assert_eq!(unpack::decode_zip_name(&"한글".encode_cp949()).as_deref(), Some("한글"));
}

trait Cp949 {
    fn encode_cp949(&self) -> Vec<u8>;
}
impl Cp949 for str {
    fn encode_cp949(&self) -> Vec<u8> {
        encoding_rs::EUC_KR.encode(self).0.into_owned()
    }
}

fn mtime(p: &Path) -> std::time::SystemTime {
    fs::metadata(p).unwrap().modified().unwrap()
}

fn close_to(a: std::time::SystemTime, b: std::time::SystemTime, secs: u64) -> bool {
    let d = a.duration_since(b).or_else(|_| b.duration_since(a)).unwrap();
    d.as_secs() <= secs
}

#[test]
fn modification_times_are_preserved() {
    use std::time::{Duration, UNIX_EPOCH};
    let old = UNIX_EPOCH + Duration::from_secs(1_577_934_246); // 2020-01-02 03:04:06 UTC
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let f = src.join("old.txt");
    fs::write(&f, "old").unwrap();
    File::options().write(true).open(&f).unwrap().set_modified(old).unwrap();

    for kind in ArchiveKind::ALL {
        let arc = tmp.path().join(format!("t{}", kind.ext()));
        compress(std::slice::from_ref(&f), &arc, kind, None).unwrap();
        let out = tmp.path().join(format!("out-{}", kind.label()));
        extract(&arc, Some(&out), &Options::default()).unwrap();
        // zip은 2초 단위로 저장됨
        assert!(close_to(mtime(&out.join("old.txt")), old, 2), "{kind:?}: {:?}", mtime(&out.join("old.txt")));
    }

    // 다른 프로그램이 만든 zip (현지 시각 2020-01-02 03:04:06)
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cp949.zip");
    let out = tmp.path().join("fixture");
    extract(&fixture, Some(&out), &Options::default()).unwrap();
    let t = mtime(&out.join("영문.txt"));
    assert!(close_to(t, old, 14 * 3600 + 2), "{t:?}"); // 시간대 차이 허용
}

// ---------------------------------------------------------------------------
// RAR 풀기 (fixtures/rar: unrar 크레이트의 테스트 데이터)
// ---------------------------------------------------------------------------

#[cfg(feature = "rar")]
mod rar {
    use super::*;

    fn fx(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rar").join(name)
    }

    #[test]
    fn detects_and_lists() {
        assert_eq!(unpack::detect(&fx("version.rar")).unwrap(), unpack::Format::Rar);
        // 확장자가 달라도 내용(매직 바이트)으로 인식
        let tmp = tempdir().unwrap();
        let renamed = tmp.path().join("mystery.bin");
        fs::copy(fx("version.rar"), &renamed).unwrap();
        assert_eq!(unpack::detect(&renamed).unwrap(), unpack::Format::Rar);

        let names: Vec<String> = list(&fx("unicode.rar"), &Options::default()).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["te…―st✌".to_string()]);
        assert_eq!(unpack::base_name(Path::new("영화.part1.rar")), "영화");
        assert_eq!(unpack::base_name(Path::new("a.b.rar")), "a.b");
    }

    #[test]
    fn extracts_with_time() {
        let tmp = tempdir().unwrap();
        let s = extract(&fx("version.rar"), Some(tmp.path()), &Options::default()).unwrap();
        assert_eq!((s.files, s.bytes), (1, 11));
        assert_eq!(fs::read(tmp.path().join("VERSION")).unwrap().len(), 11);
        let year_2015 = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_420_070_400);
        assert!(mtime(&tmp.path().join("VERSION")) < year_2015 + std::time::Duration::from_secs(366 * 86400));
        // 알아서 풀기도 동작
        let s = extract_auto(&fx("version.rar"), tmp.path(), Mode::Smart, &Options::default()).unwrap();
        assert!(s.output.ends_with("VERSION (2)") || s.output.ends_with("version"), "{:?}", s.output);
    }

    #[test]
    fn passwords() {
        let tmp = tempdir().unwrap();
        // 이름은 보이고 내용만 암호화
        assert_eq!(list(&fx("crypted.rar"), &Options::default()).unwrap().len(), 1);
        let e = extract(&fx("crypted.rar"), Some(&tmp.path().join("a")), &Options::default()).unwrap_err();
        assert!(is_password_error(&e), "{e:#}");
        let wrong = Options { password: Some("nope".into()), ..Default::default() };
        let e = extract(&fx("crypted.rar"), Some(&tmp.path().join("b")), &wrong).unwrap_err();
        assert!(is_password_error(&e), "{e:#}");
        let right = Options { password: Some("unrar".into()), ..Default::default() };
        extract(&fx("crypted.rar"), Some(&tmp.path().join("c")), &right).unwrap();
        assert_eq!(fs::read_to_string(tmp.path().join("c/.gitignore")).unwrap(), "target\nCargo.lock\n");

        // 이름까지 암호화(-hp): 목록부터 암호 필요
        let e = list(&fx("comment-hpw-password.rar"), &Options::default()).unwrap_err();
        assert!(is_password_error(&e), "{e:#}");
        let pw = Options { password: Some("password".into()), ..Default::default() };
        assert_eq!(list(&fx("comment-hpw-password.rar"), &pw).unwrap().len(), 1);
    }
}

// ---------------------------------------------------------------------------
// 4~13번 기능
// ---------------------------------------------------------------------------

use std::sync::Arc;
use unpack::{CompressOptions, Level, Limits, Progress, Selection, is_cancelled, is_too_large, test_archive};

fn big_tree(root: &Path) -> Vec<std::path::PathBuf> {
    fs::create_dir_all(root.join("자료/하위")).unwrap();
    // 압축은 되지만 수준에 따라 결과가 달라지는 내용 (단어를 의사 난수로 섞음)
    let words = ["압축", "파일", "풀기", "opizenAI", "Unpack", "테스트", "문장", "zip", "7z", "자료", "폴더", "사진"];
    let mut x: u64 = 12345;
    let mut text = String::new();
    for _ in 0..60_000 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        text.push_str(words[(x >> 33) as usize % words.len()]);
        text.push(if (x >> 20).is_multiple_of(7) { '\n' } else { ' ' });
    }
    fs::write(root.join("자료/본문.txt"), &text).unwrap();
    fs::write(root.join("자료/하위/a.txt"), "A").unwrap();
    fs::write(root.join("자료/하위/b.txt"), "B").unwrap();
    vec![root.join("자료")]
}

#[test]
fn progress_reaches_total() {
    let tmp = tempdir().unwrap();
    let inputs = big_tree(tmp.path());
    for kind in ArchiveKind::ALL {
        let arc = tmp.path().join(format!("p{}", kind.ext()));
        let p = Progress::new();
        compress_with(&inputs, &arc, kind, &CompressOptions { progress: Some(p.clone()), ..Default::default() }).unwrap();
        assert_eq!(p.done(), p.total(), "압축 {kind:?}");
        assert!(p.total() > 0);

        let p = Progress::new();
        let opts = Options { progress: Some(p.clone()), ..Default::default() };
        extract(&arc, Some(&tmp.path().join(format!("out-{}", kind.label()))), &opts).unwrap();
        assert!(p.total() > 0, "{kind:?}");
        assert_eq!(p.fraction(), Some(1.0), "풀기 {kind:?}: {}/{}", p.done(), p.total());
    }
}

#[test]
fn cancel_stops_and_cleans_up() {
    let tmp = tempdir().unwrap();
    let inputs = big_tree(tmp.path());
    let arc = tmp.path().join("c.zip");
    compress(&inputs, &arc, ArchiveKind::Zip, None).unwrap();

    // 시작 전에 취소해 두면 바로 멈춤
    let p = Progress::new();
    p.cancel();
    let out = tmp.path().join("c-out");
    let e = extract(&arc, Some(&out), &Options { progress: Some(p.clone()), ..Default::default() }).unwrap_err();
    assert!(is_cancelled(&e), "{e:#}");

    // 압축 취소: .part 파일과 결과가 남지 않음
    let arc2 = tmp.path().join("c2.7z");
    let e = compress_with(&inputs, &arc2, ArchiveKind::SevenZ, &CompressOptions { progress: Some(p), ..Default::default() }).unwrap_err();
    assert!(is_cancelled(&e), "{e:#}");
    assert!(!arc2.exists());
    assert!(fs::read_dir(tmp.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part")));

    // 알아서 풀기 취소: 임시 폴더가 남지 않음
    let p = Progress::new();
    p.cancel();
    let e = extract_auto(&arc, tmp.path(), Mode::Smart, &Options { progress: Some(p), ..Default::default() }).unwrap_err();
    assert!(is_cancelled(&e));
    assert!(fs::read_dir(tmp.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".unpack-tmp")));
}

#[test]
fn extract_selected_entries() {
    let tmp = tempdir().unwrap();
    let inputs = big_tree(tmp.path());
    for kind in ArchiveKind::ALL {
        let arc = tmp.path().join(format!("s{}", kind.ext()));
        compress(&inputs, &arc, kind, None).unwrap();
        // "자료" 폴더를 보면서 "하위" 폴더만 골라 풀기 → 결과는 하위/a.txt, 하위/b.txt
        let sel = Selection { prefix: "자료".into(), names: vec!["자료/하위".into()] };
        let out = tmp.path().join(format!("sel-{}", kind.label()));
        let s = extract(&arc, Some(&out), &Options { select: Some(Arc::new(sel)), ..Default::default() }).unwrap();
        assert_eq!(s.files, 2, "{kind:?}");
        assert_eq!(fs::read_to_string(out.join("하위/a.txt")).unwrap(), "A", "{kind:?}");
        assert!(!out.join("본문.txt").exists() && !out.join("자료").exists(), "{kind:?}");
    }
}

#[test]
fn listing_has_modified_dates() {
    let tmp = tempdir().unwrap();
    let inputs = big_tree(tmp.path());
    for kind in ArchiveKind::ALL {
        let arc = tmp.path().join(format!("d{}", kind.ext()));
        compress(&inputs, &arc, kind, None).unwrap();
        let entries = list(&arc, &Options::default()).unwrap();
        let file = entries.iter().find(|e| e.name.ends_with("a.txt")).unwrap();
        let age = std::time::SystemTime::now().duration_since(file.modified.expect("날짜 없음")).unwrap_or_default();
        assert!(age.as_secs() < 3600, "{kind:?}: {age:?}");
    }
}

#[test]
fn compression_levels_differ() {
    let tmp = tempdir().unwrap();
    let inputs = big_tree(tmp.path());
    for kind in [ArchiveKind::Zip, ArchiveKind::TarGz, ArchiveKind::TarXz, ArchiveKind::SevenZ] {
        let size = |level: Level| {
            let arc = tmp.path().join(format!("l-{}-{:?}{}", kind.label(), level, kind.ext()));
            compress_with(&inputs, &arc, kind, &CompressOptions { level, ..Default::default() }).unwrap();
            fs::metadata(&arc).unwrap().len()
        };
        let (fast, best) = (size(Level::Fast), size(Level::Best));
        assert!(best <= fast, "{kind:?}: 최대 {best} > 빠르게 {fast}");
    }
}

#[test]
fn test_archive_detects_corruption() {
    let tmp = tempdir().unwrap();
    let inputs = big_tree(tmp.path());
    for kind in ArchiveKind::ALL {
        let arc = tmp.path().join(format!("t{}", kind.ext()));
        compress(&inputs, &arc, kind, None).unwrap();
        let s = test_archive(&arc, &Options::default()).unwrap();
        assert_eq!(s.files, 3, "{kind:?}");
        // 검사는 아무것도 만들지 않음
        assert!(!tmp.path().join("t").exists());

        // 중간 바이트를 망가뜨리면 검사가 실패해야 함
        let mut bytes = fs::read(&arc).unwrap();
        let mid = bytes.len() / 2;
        for b in &mut bytes[mid..mid + 16] {
            *b ^= 0xFF;
        }
        let bad = tmp.path().join(format!("bad{}", kind.ext()));
        fs::write(&bad, bytes).unwrap();
        assert!(test_archive(&bad, &Options::default()).is_err(), "{kind:?}: 손상을 못 찾음");
    }
    #[cfg(feature = "rar")]
    {
        let rar = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rar/version.rar");
        assert_eq!(test_archive(&rar, &Options::default()).unwrap().files, 1);
    }
}

#[test]
fn split_volumes() {
    let tmp = tempdir().unwrap();
    let inputs = big_tree(tmp.path());
    for kind in [ArchiveKind::Zip, ArchiveKind::SevenZ, ArchiveKind::TarGz] {
        let whole = tmp.path().join(format!("whole{}", kind.ext()));
        compress(&inputs, &whole, kind, None).unwrap();
        // 7-Zip "분할" 방식처럼 그냥 잘라서 이름.확장자.001, .002 … 로 저장
        let bytes = fs::read(&whole).unwrap();
        let name = format!("나눔{}", kind.ext());
        for (i, chunk) in bytes.chunks(bytes.len() / 3 + 1).enumerate() {
            fs::write(tmp.path().join(format!("{name}.{:03}", i + 1)), chunk).unwrap();
        }
        let first = tmp.path().join(format!("{name}.001"));
        assert_eq!(unpack::base_name(&first), "나눔");
        assert_eq!(unpack::archive_parts(&first).len(), 3);
        let out = tmp.path().join(format!("split-{}", kind.label()));
        extract(&first, Some(&out), &Options::default()).unwrap();
        assert_eq!(fs::read_to_string(out.join("자료/하위/b.txt")).unwrap(), "B", "{kind:?}");
    }
}

#[test]
fn bomb_guard() {
    let tmp = tempdir().unwrap();
    // 0으로 가득 찬 4 MB → 압축하면 수 KB (압축률 수백 배)
    fs::write(tmp.path().join("zeros.bin"), vec![0u8; 4 << 20]).unwrap();
    let inputs = vec![tmp.path().join("zeros.bin")];
    let strict = Limits { ratio: 50, min_size: 1 << 20 };
    for kind in ArchiveKind::ALL {
        let arc = tmp.path().join(format!("bomb{}", kind.ext()));
        compress(&inputs, &arc, kind, None).unwrap();
        let out = tmp.path().join(format!("bomb-{}", kind.label()));
        let e = extract(&arc, Some(&out), &Options { limits: strict, ..Default::default() }).unwrap_err();
        assert!(is_too_large(&e), "{kind:?}: {e:#}");
        assert!(!out.join("zeros.bin").exists(), "{kind:?}: 반쯤 쓴 파일이 남음");
        // 확인하고 풀기
        extract(&arc, Some(&out), &Options { limits: strict, allow_large: true, ..Default::default() }).unwrap();
        assert_eq!(fs::metadata(out.join("zeros.bin")).unwrap().len(), 4 << 20);
    }
    // 기본 기준(1 GiB 이상)에서는 정상 파일로 봄
    let arc = tmp.path().join("bomb.zip");
    extract(&arc, Some(&tmp.path().join("ok")), &Options::default()).unwrap();
}

#[test]
fn trash_after_extract() {
    let tmp = tempdir().unwrap();
    let arc = tmp.path().join("지울것.zip");
    zip_with(&arc, &[("a.txt", "A")]);
    extract(&arc, Some(&tmp.path().join("out")), &Options::default()).unwrap();
    match unpack::move_to_trash(&arc) {
        Ok(()) => assert!(!arc.exists()),
        // 휴지통이 없는 환경(컨테이너 등)에서는 실패를 알려야 하고 파일은 그대로여야 함
        Err(e) => {
            eprintln!("휴지통 사용 불가: {e:#}");
            assert!(arc.exists());
        }
    }
}

#[test]
fn incompressible_files_are_stored_and_still_extract() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    // 무작위에 가까운 데이터(압축 안 됨), 이미 압축된 형식, 잘 줄어드는 글
    let mut x: u64 = 0x1234_5678;
    let noise: Vec<u8> = (0..200_000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    fs::write(src.join("noise.bin"), &noise).unwrap();
    fs::write(src.join("photo.jpg"), b"not really a jpeg but named so".repeat(100)).unwrap();
    fs::write(src.join("text.txt"), "가나다라 ".repeat(20_000)).unwrap();
    assert!(unpack::is_incompressible(&src.join("noise.bin")));
    assert!(unpack::is_incompressible(&src.join("photo.jpg")));
    assert!(!unpack::is_incompressible(&src.join("text.txt")));

    for kind in [ArchiveKind::Zip, ArchiveKind::SevenZ] {
        let out = tmp.path().join(format!("smart{}", kind.ext()));
        let s = compress(std::slice::from_ref(&src), &out, kind, None).unwrap();
        assert_eq!(s.stored, 2, "{kind:?}");
        let dest = tmp.path().join(format!("out-{}", kind.label()));
        extract(&out, Some(&dest), &Options::default()).unwrap();
        assert_eq!(fs::read(dest.join("src/noise.bin")).unwrap(), noise);
        assert_eq!(fs::read_to_string(dest.join("src/text.txt")).unwrap(), "가나다라 ".repeat(20_000));
    }
    // zip: 그대로 넣은 파일은 Stored, 글은 Deflated
    let mut z = zip::ZipArchive::new(File::open(tmp.path().join("smart.zip")).unwrap()).unwrap();
    assert_eq!(z.by_name("src/noise.bin").unwrap().compression(), zip::CompressionMethod::Stored);
    assert_eq!(z.by_name("src/text.txt").unwrap().compression(), zip::CompressionMethod::Deflated);
    // 항상 압축 옵션
    let out = tmp.path().join("always.zip");
    let s = compress_with(std::slice::from_ref(&src), &out, ArchiveKind::Zip, &CompressOptions { always_compress: true, ..Default::default() })
        .unwrap();
    assert_eq!(s.stored, 0);
}

#[test]
fn split_output_makes_numbered_parts_that_extract() {
    let tmp = tempdir().unwrap();
    let inputs = big_tree(&tmp.path().join("in"));
    for kind in ArchiveKind::ALL {
        let out = tmp.path().join(format!("split{}", kind.ext()));
        let opts = CompressOptions { split: Some(16 * 1024), always_compress: true, ..Default::default() };
        let s = compress_with(&inputs, &out, kind, &opts).unwrap();
        assert!(s.parts >= 2, "{kind:?}: {} parts", s.parts);
        assert!(!out.exists());
        assert_eq!(s.output, tmp.path().join(format!("split{}.001", kind.ext())));
        let total: u64 = (1..=s.parts)
            .map(|i| fs::metadata(tmp.path().join(format!("split{}.{i:03}", kind.ext()))).unwrap().len())
            .sum();
        assert_eq!(total, s.size);
        let dest = tmp.path().join(format!("x-{}", kind.label()));
        let st = extract(&s.output, Some(&dest), &Options::default()).unwrap();
        assert!(st.files > 0, "{kind:?}");
    }
    assert_eq!(unpack::parse_size("700m"), Some(700 << 20));
    assert_eq!(unpack::parse_size("1.5G"), Some(3 << 29));
    assert_eq!(unpack::parse_size("100 KB"), Some(100 << 10));
    assert_eq!(unpack::parse_size("0"), None);
    assert_eq!(unpack::parse_size("abc"), None);
}

#[test]
fn edit_archives_add_replace_and_remove() {
    use unpack::{EditOptions, edit_archive, editable_kind};
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join("문서/하위")).unwrap();
    fs::write(src.join("문서/a.txt"), "A").unwrap();
    fs::write(src.join("문서/하위/b.txt"), "B").unwrap();
    fs::write(src.join("문서/c.txt"), "C").unwrap();
    let new = tmp.path().join("new");
    fs::create_dir_all(new.join("사진")).unwrap();
    fs::write(new.join("a.txt"), "새 A").unwrap();
    fs::write(new.join("사진/p.txt"), "P").unwrap();

    for kind in ArchiveKind::ALL {
        let arc = tmp.path().join(format!("e{}", kind.ext()));
        compress(std::slice::from_ref(&src.join("문서")), &arc, kind, None).unwrap();
        assert_eq!(editable_kind(&arc), Some(kind));
        let s = edit_archive(
            &arc,
            &["문서/하위".to_string(), "문서/c.txt".to_string()],
            &[new.join("a.txt"), new.join("사진")],
            "문서",
            &EditOptions::default(),
        )
        .unwrap();
        assert_eq!((s.added, s.removed), (2, 2), "{kind:?}");
        let mut names: Vec<String> = list(&arc, &Options::default()).unwrap().into_iter().filter(|e| !e.is_dir).map(|e| e.name).collect();
        names.sort();
        assert_eq!(names, ["문서/a.txt", "문서/사진/p.txt"], "{kind:?}");
        let out = tmp.path().join(format!("o-{}", kind.label()));
        extract(&arc, Some(&out), &Options::default()).unwrap();
        assert_eq!(fs::read_to_string(out.join("문서/a.txt")).unwrap(), "새 A", "{kind:?}");
        assert!(!arc.with_file_name(format!("e{}.part", kind.ext())).exists());
        // 모두 지우기는 거절, 원본은 그대로
        if kind != ArchiveKind::Zip {
            assert!(edit_archive(&arc, &["문서".into()], &[], "", &EditOptions::default()).is_err());
            assert_eq!(list(&arc, &Options::default()).unwrap().iter().filter(|e| !e.is_dir).count(), 2);
        }
    }
    // 암호 zip: 남긴 항목은 암호 그대로, 새 항목도 암호
    let arc = tmp.path().join("pw.zip");
    compress(std::slice::from_ref(&src.join("문서")), &arc, ArchiveKind::Zip, Some("비번")).unwrap();
    edit_archive(&arc, &[], &[new.join("a.txt")], "", &EditOptions { password: Some("비번".into()), ..Default::default() }).unwrap();
    assert!(unpack::is_password_error(&extract(&arc, Some(&tmp.path().join("pw-x")), &Options::default()).unwrap_err()));
    let o = Options { password: Some("비번".into()), ..Default::default() };
    let st = extract(&arc, Some(&tmp.path().join("pw-ok")), &o).unwrap();
    assert_eq!(st.files, 4);
    assert_eq!(fs::read_to_string(tmp.path().join("pw-ok/a.txt")).unwrap(), "새 A");
}
