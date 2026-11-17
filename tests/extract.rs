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
