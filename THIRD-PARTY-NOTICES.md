# 제3자 구성 요소 고지 (Third-party notices)

opizenAI_Unpack 자체는 MIT 라이선스입니다 (`LICENSE`). 아래 구성 요소는 각자의 라이선스를 따릅니다.

## UnRAR (RAR 풀기)

RAR 풀기 기능은 Alexander Roshal의 UnRAR 소스 코드(`vendor/unrar_sys/vendor/unrar`)를 사용합니다.
UnRAR는 **RAR 파일을 푸는 용도로만** 사용하며, RAR 파일을 만드는(압축하는) 기능은 없습니다.

- Rust 연결 코드 `unrar` / `unrar_sys` 크레이트: MIT 또는 Apache-2.0 (https://github.com/muja/unrar.rs)
- `vendor/unrar_sys/build.rs` 만 교차 빌드를 위해 고쳤으며, UnRAR C++ 소스는 수정하지 않았습니다.
- RAR 지원 없이 빌드하려면: `cargo build --no-default-features --features gui`

UnRAR 라이선스 전문:

```
 ******    *****   ******   UnRAR - free utility for RAR archives
 **   **  **   **  **   **  ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
 ******   *******  ******    License for use and distribution of
 **   **  **   **  **   **   ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
 **   **  **   **  **   **         FREE portable version
                                   ~~~~~~~~~~~~~~~~~~~~~

      The source code of UnRAR utility is freeware. This means:

   1. All copyrights to RAR and the utility UnRAR are exclusively
      owned by the author - Alexander Roshal.

   2. UnRAR source code may be used in any software to handle
      RAR archives without limitations free of charge, but cannot be
      used to develop RAR (WinRAR) compatible archiver and to
      re-create RAR compression algorithm, which is proprietary.
      Distribution of modified UnRAR source code in separate form
      or as a part of other software is permitted, provided that
      full text of this paragraph, starting from "UnRAR source code"
      words, is included in license, or in documentation if license
      is not available, and in source code comments of resulting package.

   3. The UnRAR utility may be freely distributed. It is allowed
      to distribute UnRAR inside of other software packages.

   4. THE RAR ARCHIVER AND THE UnRAR UTILITY ARE DISTRIBUTED "AS IS".
      NO WARRANTY OF ANY KIND IS EXPRESSED OR IMPLIED.  YOU USE AT 
      YOUR OWN RISK. THE AUTHOR WILL NOT BE LIABLE FOR DATA LOSS, 
      DAMAGES, LOSS OF PROFITS OR ANY OTHER KIND OF LOSS WHILE USING
      OR MISUSING THIS SOFTWARE.

   5. Installing and using the UnRAR utility signifies acceptance of
      these terms and conditions of the license.

   6. If you don't agree with terms of the license you must remove
      UnRAR files from your storage devices and cease to use the
      utility.

      Thank you for your interest in RAR and UnRAR.


                                            Alexander L. Roshal
```
