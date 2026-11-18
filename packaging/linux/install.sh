#!/usr/bin/env bash
# opizenAI_Unpack 설치 (tar.gz 배포판용)
#   사용자 설치:   ./install.sh            → ~/.local
#   시스템 설치:   sudo ./install.sh       → /usr/local
#   다른 위치:     PREFIX=/opt/x ./install.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
if [ -z "${PREFIX:-}" ]; then
  if [ "$(id -u)" = 0 ]; then PREFIX=/usr/local; else PREFIX="$HOME/.local"; fi
fi
BIN="$PREFIX/bin"; SHARE="$PREFIX/share"

install -Dm755 "$HERE/bin/opizenAI_Unpack" "$BIN/opizenAI_Unpack"
install -Dm755 "$HERE/bin/unpack" "$BIN/unpack"
install -Dm644 "$HERE/share/applications/opizenAI_Unpack.desktop" "$SHARE/applications/opizenAI_Unpack.desktop"
install -Dm644 "$HERE/share/icons/hicolor/256x256/apps/opizenai-unpack.png" "$SHARE/icons/hicolor/256x256/apps/opizenai-unpack.png"
# KDE Dolphin (서비스 메뉴는 실행 권한이 있어야 함)
install -Dm755 "$HERE/share/kio/servicemenus/opizenai-unpack-extract.desktop" "$SHARE/kio/servicemenus/opizenai-unpack-extract.desktop"
install -Dm755 "$HERE/share/kio/servicemenus/opizenai-unpack-compress.desktop" "$SHARE/kio/servicemenus/opizenai-unpack-compress.desktop"
# Cinnamon Nemo
for f in "$HERE"/share/nemo/actions/*.nemo_action; do
  install -Dm644 "$f" "$SHARE/nemo/actions/$(basename "$f")"
done
# GNOME Nautilus (python3-nautilus 필요)
install -Dm644 "$HERE/share/nautilus-python/extensions/opizenai_unpack.py" "$SHARE/nautilus-python/extensions/opizenai_unpack.py"

# 사용자 설치이면 Nautilus "스크립트" 메뉴도 추가 (python3-nautilus 없이도 동작)
if [ "$PREFIX" = "$HOME/.local" ]; then
  SCRIPTS="$HOME/.local/share/nautilus/scripts/opizenAI_Unpack"
  mkdir -p "$SCRIPTS"
  mk() { printf '#!/bin/sh\nexec "%s" %s "$@"\n' "$BIN/opizenAI_Unpack" "$2" > "$SCRIPTS/$1"; chmod 755 "$SCRIPTS/$1"; }
  mk "1 알아서 풀기" --smart
  mk "2 여기에 압축 풀기" --here
  mk "3 새 폴더에 압축 풀기" --folder
  mk "4 ZIP으로 압축" --zip
  mk "5 7Z로 압축" --7z
  mk "6 압축하기..." --compress
fi

command -v update-desktop-database >/dev/null && update-desktop-database -q "$SHARE/applications" || true

# zip 파일을 opizenAI_Unpack으로 열도록 연결 (사용자 설치일 때, NO_ASSOC=1 이면 건너뜀)
if [ "$PREFIX" = "$HOME/.local" ] && [ -z "${NO_ASSOC:-}" ] && command -v xdg-mime >/dev/null; then
  xdg-mime default opizenAI_Unpack.desktop application/zip && echo "zip 파일 연결: opizenAI_Unpack"
fi
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$SHARE/icons/hicolor" 2>/dev/null || true

echo "설치 완료: $BIN/opizenAI_Unpack"
case ":$PATH:" in *":$BIN:"*) ;; *) echo "참고: $BIN 이 PATH에 없습니다. 셸 설정에 추가하세요." ;; esac
echo "파일 관리자 우클릭 메뉴를 적용하려면 파일 관리자를 다시 시작하세요 (예: nautilus -q)."
