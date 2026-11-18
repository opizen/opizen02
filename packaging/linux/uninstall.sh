#!/usr/bin/env bash
# opizenAI_Unpack 제거 (install.sh로 설치한 경우)
set -euo pipefail
if [ -z "${PREFIX:-}" ]; then
  if [ "$(id -u)" = 0 ]; then PREFIX=/usr/local; else PREFIX="$HOME/.local"; fi
fi
SHARE="$PREFIX/share"
rm -f "$PREFIX/bin/opizenAI_Unpack" "$PREFIX/bin/unpack" \
  "$SHARE/applications/opizenAI_Unpack.desktop" \
  "$SHARE/icons/hicolor/256x256/apps/opizenai-unpack.png" \
  "$SHARE/kio/servicemenus/opizenai-unpack-extract.desktop" \
  "$SHARE/kio/servicemenus/opizenai-unpack-compress.desktop" \
  "$SHARE/nautilus-python/extensions/opizenai_unpack.py"
rm -f "$SHARE"/nemo/actions/opizenai-unpack-*.nemo_action
if [ "$PREFIX" = "$HOME/.local" ] && [ -d "$HOME/.local/share/nautilus/scripts/opizenAI_Unpack" ]; then
  rm -r "$HOME/.local/share/nautilus/scripts/opizenAI_Unpack"
fi
command -v update-desktop-database >/dev/null && update-desktop-database -q "$SHARE/applications" || true
echo "제거 완료"
