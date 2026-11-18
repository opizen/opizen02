//! 설정 저장 — 다시 열어도 저장 위치·형식·수준·옵션이 그대로 남도록.
//!
//! - Windows: `%APPDATA%\opizenAI_Unpack\settings.ini`
//! - Linux 등: `$XDG_CONFIG_HOME/opizenAI_Unpack/settings.ini` (없으면 `~/.config/...`)
//!
//! 암호는 저장하지 않습니다.

use std::path::PathBuf;
use unpack::{ArchiveKind, Level};

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// `None`이면 압축 파일과 같은 위치
    pub out_dir: Option<PathBuf>,
    pub kind: ArchiveKind,
    pub level: Level,
    /// 문제없이 풀리면 압축 파일을 휴지통으로
    pub trash_after: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { out_dir: None, kind: ArchiveKind::Zip, level: Level::Normal, trash_after: false }
    }
}

fn path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join(unpack::APP_NAME).join("settings.ini"))
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return s };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "out_dir" if !v.is_empty() => s.out_dir = Some(PathBuf::from(v)).filter(|p| p.is_dir()),
                "kind" => s.kind = ArchiveKind::ALL.into_iter().find(|k| k.label() == v).unwrap_or(s.kind),
                "level" => s.level = Level::ALL.into_iter().find(|l| format!("{l:?}") == v).unwrap_or(s.level),
                "trash_after" => s.trash_after = v == "true",
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let out = self.out_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default();
        let text = format!(
            "# {} 설정\nout_dir={out}\nkind={}\nlevel={:?}\ntrash_after={}\n",
            unpack::APP_NAME,
            self.kind.label(),
            self.level,
            self.trash_after
        );
        let _ = std::fs::write(p, text);
    }
}
