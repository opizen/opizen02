// Windows 빌드 시 아이콘과 버전 정보를 실행 파일에 넣습니다.
fn main() {
    println!("cargo:rerun-if-changed=assets/unpack.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/unpack.ico")
            .set("ProductName", "Unpack")
            .set("FileDescription", "Unpack 압축 해제 프로그램")
            .set("LegalCopyright", "MIT License");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Windows 리소스 컴파일 실패: {e}");
        }
    }
}
