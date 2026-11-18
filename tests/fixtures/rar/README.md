이 폴더의 RAR 파일은 `unrar` 크레이트(https://github.com/muja/unrar.rs, MIT 또는 Apache-2.0)의
테스트 데이터(`data/`)에서 가져왔습니다. RAR 압축 프로그램은 상용이라 여기서 직접 만들 수 없습니다.

| 파일 | 내용 |
|---|---|
| version.rar | `VERSION` (11바이트) |
| unicode.rar | 유니코드 이름 `te…―st✌` |
| crypted.rar | 암호 `unrar`, `.gitignore` (이름은 암호화 안 됨) |
| comment-hpw-password.rar | 암호 `password`, 이름까지 암호화(-hp) |
