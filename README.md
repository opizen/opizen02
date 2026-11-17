# unpack — Rust 압축 해제 프로그램

여러 압축 형식을 하나의 명령으로 안전하게 해제하는 CLI 도구입니다.

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
cargo build --release
```

실행 파일: `target/release/unpack`

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
