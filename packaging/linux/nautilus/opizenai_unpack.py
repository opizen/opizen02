# opizenAI_Unpack — GNOME 파일(Nautilus) 우클릭 메뉴
#
# 필요: python3-nautilus (nautilus-python)
# 설치 위치: /usr/share/nautilus-python/extensions/ 또는 ~/.local/share/nautilus-python/extensions/
# 적용: nautilus -q 로 파일 관리자를 다시 시작

import shutil
import subprocess
from urllib.parse import unquote, urlparse

from gi.repository import GObject, Nautilus

APP = "opizenAI_Unpack"
ARCHIVE_EXT = (
    ".zip", ".jar", ".7z", ".tar", ".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tbz",
    ".tar.xz", ".txz", ".tar.zst", ".tzst", ".gz", ".bz2", ".xz", ".zst",
)


def _paths(files):
    out = []
    for f in files:
        uri = f.get_uri()
        if not uri.startswith("file://"):
            return []  # 로컬 파일만 지원
        out.append(unquote(urlparse(uri).path))
    return out


def _run(args):
    exe = shutil.which(APP) or APP
    subprocess.Popen([exe, *args], start_new_session=True)


class OpizenAIUnpackMenu(GObject.GObject, Nautilus.MenuProvider):
    def _item(self, name, label, args, paths):
        item = Nautilus.MenuItem(name=f"OpizenAIUnpack::{name}", label=label)
        item.connect("activate", lambda *_: _run([*args, *paths]))
        return item

    # Nautilus 43+ 는 (files), 이전 버전은 (window, files) 로 호출합니다.
    def get_file_items(self, *args):
        files = args[-1]
        paths = _paths(files)
        if not paths:
            return []

        top = Nautilus.MenuItem(name="OpizenAIUnpack::Top", label=APP)
        sub = Nautilus.Menu()
        top.set_submenu(sub)

        if all(p.lower().endswith(ARCHIVE_EXT) for p in paths):
            sub.append_item(self._item("Smart", "알아서 풀기", ["--smart"], paths))
            sub.append_item(self._item("Here", "여기에 압축 풀기", ["--here"], paths))
            sub.append_item(self._item("Folder", "새 폴더에 압축 풀기", ["--folder"], paths))
        sub.append_item(self._item("Zip", "ZIP으로 압축", ["--zip"], paths))
        sub.append_item(self._item("SevenZ", "7Z로 압축", ["--7z"], paths))
        sub.append_item(self._item("Ask", "압축하기...", ["--compress"], paths))
        return [top]
