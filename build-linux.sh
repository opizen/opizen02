#!/usr/bin/env bash
# Linux 배포 파일을 만듭니다: dist/*.deb (Debian/Ubuntu) 와 dist/*-linux-x86_64.tar.gz (그 외 배포판)
# 필요: Rust, dpkg-deb
set -euo pipefail
cd "$(dirname "$0")"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
NAME=opizenAI_Unpack
cargo build --release --features gui
REL=target/release
L=packaging/linux

# 공통 파일 배치 (prefix 기준 상대 경로)
stage() {
  local root=$1
  install -Dm755 "$REL/$NAME" "$root/bin/$NAME"
  install -Dm755 "$REL/unpack" "$root/bin/unpack"
  install -Dm644 "$L/$NAME.desktop" "$root/share/applications/$NAME.desktop"
  install -Dm644 assets/unpack.png "$root/share/icons/hicolor/256x256/apps/opizenai-unpack.png"
  install -Dm755 "$L/kde-extract.desktop" "$root/share/kio/servicemenus/opizenai-unpack-extract.desktop"
  install -Dm755 "$L/kde-compress.desktop" "$root/share/kio/servicemenus/opizenai-unpack-compress.desktop"
  for f in "$L"/nemo/*.nemo_action; do install -Dm644 "$f" "$root/share/nemo/actions/$(basename "$f")"; done
  install -Dm644 "$L/nautilus/opizenai_unpack.py" "$root/share/nautilus-python/extensions/opizenai_unpack.py"
  install -Dm644 README.md "$root/share/doc/opizenai-unpack/README.md"
  install -Dm644 LICENSE "$root/share/doc/opizenai-unpack/copyright"
  install -Dm644 THIRD-PARTY-NOTICES.md "$root/share/doc/opizenai-unpack/THIRD-PARTY-NOTICES.md"
}

rm -rf target/pkg && mkdir -p dist target/pkg

# --- .deb ---
DEB=target/pkg/deb
stage "$DEB/usr"
mkdir -p "$DEB/DEBIAN"
SIZE=$(du -sk "$DEB/usr" | cut -f1)
GLIBC=$(ldd --version | head -1 | grep -oE '[0-9]+\.[0-9]+$')
cat > "$DEB/DEBIAN/control" <<CTRL
Package: opizenai-unpack
Version: $VERSION
Section: utils
Priority: optional
Architecture: amd64
Installed-Size: $SIZE
Maintainer: opizen <noreply@github.com>
Depends: libc6 (>= $GLIBC), libgcc-s1, libxkbcommon-x11-0, libegl1 | libgl1, libxcursor1, libxrandr2, libxi6
Recommends: xdg-desktop-portal, libvulkan1, mesa-vulkan-drivers | vulkan-icd
Suggests: python3-nautilus
Homepage: https://github.com/opizen/opizen02
Description: opizenAI_Unpack - 압축 풀기 / 압축하기 (GUI + CLI)
 zip, 7z, rar, tar, tar.gz, tar.bz2, tar.xz, tar.zst, gz, bz2, xz, zst 압축 풀기와
 zip, 7z, tar.gz, tar.xz 압축하기를 지원합니다. 파일 관리자(Nautilus, Dolphin,
 Nemo) 우클릭 메뉴를 함께 설치합니다.
CTRL
dpkg-deb --root-owner-group --build "$DEB" "dist/opizenai-unpack_${VERSION}_amd64.deb" >/dev/null

# --- tar.gz (설치 스크립트 포함) ---
TGZ_DIR="$NAME-$VERSION-linux-x86_64"
T="target/pkg/$TGZ_DIR"
stage "$T"
rm -rf "$T/share/doc"
cp README.md LICENSE THIRD-PARTY-NOTICES.md "$T/"
install -m755 "$L/install.sh" "$L/uninstall.sh" "$T/"
tar -C target/pkg -czf "dist/$TGZ_DIR.tar.gz" "$TGZ_DIR"

echo "생성됨: dist/opizenai-unpack_${VERSION}_amd64.deb"
echo "생성됨: dist/$TGZ_DIR.tar.gz"
