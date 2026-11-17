# Unpack — Rust 압축 해제 프로그램

여러 압축 형식을 안전하게 해제하는 프로그램입니다. **Windows GUI 앱(설치 프로그램 제공)**과 명령줄 도구(CLI)를 함께 제공합니다.

## Windows 설치

1. [Actions](../../actions) 의 최신 "Windows 설치 파일" 실행 결과에서 `UnpackSetup` 아티팩트를 받거나,
   [Releases](../../releases) 에서 `UnpackSetup-<버전>-x64.exe` 를 받습니다.
2. 설치 파일을 실행합니다 (관리자 권한 필요, 64비트 Windows 10/11).
3. 설치 시 선택할 수 있는 항목
   - 시작 메뉴 바로가기 (기본)
   - 탐색기 우클릭 메뉴 **"Unpack으로 압축 풀기"** (Windows 11은 "추가 옵션 표시" 안)
   - 바탕화면 바로가기
4. 제거: Windows 설정 → 앱 → Unpack 제거

> 서명되지 않은 실행 파일이므로 처음 실행 시 SmartScreen 경고가 나올 수 있습니다. "추가 정보 → 실행"을 누르세요.

### GUI 사용법

- 압축 파일을 창에 **끌어다 놓거나** "파일 추가…" 버튼으로 선택
- 출력 위치(압축 파일과 같은 위치 / 지정 폴더), 암호, 덮어쓰기 옵션 지정
- **압축 풀기** 클릭 → 각 파일마다 이름이 같은 폴더에 해제
- "내용" 버튼으로 풀기 전에 목록 확인

### 설치 파일 직접 빌드 (Linux)

```bash
rustup target add x86_64-pc-windows-gnu
sudo apt-get install -y mingw-w64 nsis
./build-windows.sh   # → dist/UnpackSetup-<버전>-x64.exe
```

`v*` 태그를 푸시하면 GitHub Actions가 설치 파일을 빌드해 Release에 자동 첨부합니다.

---

## 명령줄 도구 (CLI)

## 지원 형식

| 형식 | 확장자 |
|------|--------|
| ZIP (암호 지원) | `.zip`, `.jar` |
| 7-Zip (암호 지원) | `.7z` |
| TAR | `.tar` |
| TAR + 압축 | `.tar.gz` `.tgz`, `.tar.bz2` `.tbz2`, `.tar.xz` `.txz`, `.tar.zst` `.tzst` |
| 단일 파일 압축 | `.gz`, `.bz2`, `.xz`, `.zst` |

확장자가 없거나 틀려도 **매직 바이트**로 형식을 감지하며, `.gz`/`.xz` 등의 내용이 tar이면 자동으로 tar로 풉니다.

## 빌드

```bash
cargo build --release                 # CLI: target/release/unpack
cargo build --release --features gui  # GUI: target/release/unpack-gui
```

## 사용법

```bash
unpack archive.zip                 # archive/ 폴더에 해제
unpack data.tar.gz -o out          # out/ 폴더에 해제
unpack a.zip b.7z c.tar.xz -o out  # out/a, out/b, out/c 로 각각 해제
unpack -l archive.7z               # 목록만 보기
unpack -f archive.zip              # 기존 파일 덮어쓰기
unpack -p 비밀번호 secret.zip      # 암호 걸린 zip / 7z
unpack -v archive.tar              # 해제되는 파일 이름 출력
```

| 옵션 | 설명 |
|------|------|
| `-o, --output <DIR>` | 출력 폴더 (기본: 압축 파일 옆, 확장자를 뺀 이름) |
| `-l, --list` | 해제하지 않고 목록 표시 |
| `-f, --overwrite` | 이미 존재하는 파일 덮어쓰기 (기본은 건너뜀) |
| `-p, --password <PW>` | zip / 7z 암호 |
| `-v, --verbose` | 해제되는 항목 출력 |

## 보안

- `../`, 절대 경로, 드라이브 문자(`C:`)가 들어간 항목은 건너뜁니다 (Zip Slip 방지).
- 출력 폴더 밖을 가리키는 심볼릭 링크는 만들지 않습니다.
- 덮어쓸 때 기존 심볼릭 링크를 먼저 지워, 링크를 통해 바깥 파일을 덮어쓰지 않습니다.

## 테스트

```bash
cargo test
```
