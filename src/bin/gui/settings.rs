//! 설정 저장 — 다시 열어도 저장 위치·형식·수준·옵션이 그대로 남도록.
//!
//! - Windows: `%APPDATA%\opizenAI_Unpack\settings.ini`
//! - Linux 등: `$XDG_CONFIG_HOME/opizenAI_Unpack/settings.ini` (없으면 `~/.config/...`)
//!
//! 암호는 저장하지 않습니다.

use std::path::{Path, PathBuf};
use unpack::{ArchiveKind, Level};

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// `None`이면 압축 파일과 같은 위치
    pub out_dir: Option<PathBuf>,
    pub kind: ArchiveKind,
    pub level: Level,
    /// 문제없이 풀리면 압축 파일을 휴지통으로
    pub trash_after: bool,
    /// 마지막으로 보던 폴더
    pub last_dir: Option<PathBuf>,
    /// 숨김 파일 표시
    pub show_hidden: bool,
    /// 이미 압축된 파일도 다시 압축 (기본: 그대로 넣음)
    pub always_compress: bool,
    /// 분할 크기 (`700m` 등, 비어 있으면 나누지 않음)
    pub split: String,
    /// 사용자가 추가한 즐겨찾기 폴더
    pub pins: Vec<PathBuf>,
    /// 창 배치 (다음에 열 때 되살림)
    pub panes: Vec<PaneState>,
    pub active: usize,
    /// 창 경계선 위치 (0~1)
    pub split_x: f32,
    pub split_y: f32,
    /// 미리보기 칸
    pub show_preview: bool,
}

/// 창 하나의 저장된 상태
#[derive(Debug, Clone, PartialEq)]
pub struct PaneState {
    pub show_places: bool,
    pub icons: bool,
    /// 탭마다 폴더
    pub tabs: Vec<PathBuf>,
    pub cur: usize,
}

impl PaneState {
    /// `pane=바로가기(1/0);아이콘(1/0);현재탭;폴더<TAB>폴더…`
    fn to_line(&self) -> String {
        let tabs: Vec<String> = self.tabs.iter().map(|t| t.display().to_string()).collect();
        format!("{};{};{};{}", u8::from(self.show_places), u8::from(self.icons), self.cur, tabs.join("\t"))
    }

    fn parse(v: &str) -> Option<Self> {
        let mut it = v.splitn(4, ';');
        let show_places = it.next()? == "1";
        let icons = it.next()? == "1";
        let cur: usize = it.next()?.parse().ok()?;
        let tabs: Vec<PathBuf> = it.next()?.split('\t').map(PathBuf::from).filter(|p| p.is_dir()).collect();
        if tabs.is_empty() {
            return None;
        }
        let cur = cur.min(tabs.len() - 1);
        Some(Self { show_places, icons, tabs, cur })
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            out_dir: None, kind: ArchiveKind::Zip, level: Level::Normal, trash_after: false, last_dir: None, show_hidden: false, always_compress: false,
            split: String::new(),
            pins: Vec::new(),
            panes: Vec::new(),
            active: 0,
            split_x: 0.5,
            split_y: 0.5,
            show_preview: false,
        }
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
        s.panes.clear();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "out_dir" if !v.is_empty() => s.out_dir = Some(PathBuf::from(v)).filter(|p| p.is_dir()),
                "kind" => s.kind = ArchiveKind::ALL.into_iter().find(|k| k.label() == v).unwrap_or(s.kind),
                "level" => s.level = Level::ALL.into_iter().find(|l| format!("{l:?}") == v).unwrap_or(s.level),
                "trash_after" => s.trash_after = v == "true",
                "last_dir" if !v.is_empty() => s.last_dir = Some(PathBuf::from(v)).filter(|p| p.is_dir()),
                "show_hidden" => s.show_hidden = v == "true",
                "always_compress" => s.always_compress = v == "true",
                "split" if unpack::parse_size(v).is_some() => s.split = v.to_string(),
                "pin" if Path::new(v).is_dir() => s.pins.push(PathBuf::from(v)),
                "pane" => s.panes.extend(PaneState::parse(v)),
                "active" => s.active = v.parse().unwrap_or(0),
                "split_x" => s.split_x = v.parse::<f32>().unwrap_or(0.5).clamp(0.15, 0.85),
                "split_y" => s.split_y = v.parse::<f32>().unwrap_or(0.5).clamp(0.15, 0.85),
                "show_preview" => s.show_preview = v == "true",
                _ => {}
            }
        }
        s.active = s.active.min(s.panes.len().saturating_sub(1));
        s
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let out = self.out_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default();
        let text = format!(
            "# {} 설정\nout_dir={out}\nkind={}\nlevel={:?}\ntrash_after={}\nlast_dir={}\nshow_hidden={}\nalways_compress={}\nsplit={}\n",
            unpack::APP_NAME,
            self.kind.label(),
            self.level,
            self.trash_after,
            self.last_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default(),
            self.show_hidden,
            self.always_compress,
            self.split
        );
        let mut text = text;
        for pin in &self.pins {
            text += &format!("pin={}\n", pin.display());
        }
        for pane in &self.panes {
            text += &format!("pane={}\n", pane.to_line());
        }
        text += &format!("active={}\nsplit_x={:.3}\nsplit_y={:.3}\nshow_preview={}\n", self.active, self.split_x, self.split_y, self.show_preview);
        let _ = std::fs::write(p, text);
    }
}
