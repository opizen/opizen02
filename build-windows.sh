#!/usr/bin/env bash
# Linux에서 Windows 설치 파일을 만듭니다.
# 필요: rustup target add x86_64-pc-windows-gnu, mingw-w64, nsis
set -euo pipefail
cd "$(dirname "$0")"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
cargo build --release --target x86_64-pc-windows-gnu --features gui
mkdir -p dist
makensis -V2 -DVERSION="$VERSION" installer/unpack.nsi
echo "생성됨: dist/UnpackSetup-$VERSION-x64.exe"
