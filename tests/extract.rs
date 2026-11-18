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

use unpack::{ArchiveKind, compress, default_archive_path};

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
