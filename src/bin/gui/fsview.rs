//! 폴더 보기 — 지금 폴더의 파일·폴더를 표로 보여 줍니다 (파일 탐색기).
//!
//! - 더블클릭(Enter): 폴더는 들어가고, 압축 파일은 안을 보고, 그 밖의 파일은 기본 프로그램으로 엶
//! - Ctrl·Shift + 클릭, Ctrl+A: 여러 개 선택 / Esc: 선택 해제 / Backspace: 위 폴더 / F5: 새로 고침

use crate::icons::Icon;
use crate::table::{self, Col};
use eframe::egui;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct FsEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub archive: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// 표에서 일어난 일 (Home이 처리)
pub enum FsAction {
    None,
    /// 더블클릭·Enter: 폴더 들어가기 / 압축 파일 보기 / 파일 열기
    Open(PathBuf),
    /// 위 폴더로
    Up,
    /// 우클릭 메뉴 명령
    Command(FsCommand),
}

#[derive(Clone, Copy, PartialEq)]
pub enum FsCommand {
    Extract,
    ExtractHere,
    ExtractNewFolder,
    ExtractTo,
    Test,
    Compress,
    ShowInSystem,
}

pub struct FolderView {
    pub dir: PathBuf,
    entries: Vec<FsEntry>,
    pub error: Option<String>,
    pub selected: BTreeSet<PathBuf>,
    anchor: Option<PathBuf>,
    sort: table::Sort,
}

fn is_hidden(path: &Path, name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        if let Ok(m) = std::fs::symlink_metadata(path) {
            return m.file_attributes() & (HIDDEN | SYSTEM) != 0;
        }
    }
    let _ = path;
    false
}

impl FolderView {
    pub fn new(dir: PathBuf, show_hidden: bool) -> Self {
        let mut v = Self { dir, entries: Vec::new(), error: None, selected: BTreeSet::new(), anchor: None, sort: (0, true) };
        v.reload(show_hidden);
        v
    }

    /// 폴더 내용을 다시 읽음 (고른 항목 중 아직 있는 것은 유지)
    pub fn reload(&mut self, show_hidden: bool) {
        self.entries.clear();
        self.error = None;
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let path = e.path();
                    let name = e.file_name().to_string_lossy().into_owned();
                    if !show_hidden && is_hidden(&path, &name) {
                        continue;
                    }
                    // 심볼릭 링크는 가리키는 대상 기준 (깨진 링크는 링크 자체)
                    let meta = std::fs::metadata(&path).or_else(|_| std::fs::symlink_metadata(&path));
                    let (is_dir, size, modified) = match &meta {
                        Ok(m) => (m.is_dir(), if m.is_dir() { 0 } else { m.len() }, m.modified().ok()),
                        Err(_) => (false, 0, None),
                    };
                    let archive = !is_dir && unpack::is_archive_name(&path);
                    self.entries.push(FsEntry { path, name, is_dir, archive, size, modified });
                }
            }
            Err(e) => self.error = Some(format!("폴더를 열 수 없습니다: {e}")),
        }
        let paths: BTreeSet<PathBuf> = self.entries.iter().map(|e| e.path.clone()).collect();
        self.selected.retain(|p| paths.contains(p));
        self.sort_entries();
    }

    fn sort_entries(&mut self) {
        let (col, asc) = self.sort;
        self.entries.sort_by(|a, b| {
            let dirs = b.is_dir.cmp(&a.is_dir);
            let ord = match col {
                1 => a.modified.cmp(&b.modified),
                2 => kind(a).cmp(&kind(b)),
                3 => a.size.cmp(&b.size),
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            };
            dirs.then(if asc { ord } else { ord.reverse() })
        });
    }

    /// 고른 항목 (표에 보이는 순서)
    pub fn selection(&self) -> Vec<&FsEntry> {
        self.entries.iter().filter(|e| self.selected.contains(&e.path)).collect()
    }

    pub fn select_only(&mut self, paths: &[PathBuf]) {
        self.selected = paths.iter().filter(|p| self.entries.iter().any(|e| &e.path == *p)).cloned().collect();
        self.anchor = self.selected.iter().next().cloned();
    }

    pub fn status(&self) -> String {
        let dirs = self.entries.iter().filter(|e| e.is_dir).count();
        let files = self.entries.len() - dirs;
        let mut s = format!("폴더 {dirs}개 · 파일 {files}개");
        let sel = self.selection();
        if !sel.is_empty() {
            let size: u64 = sel.iter().map(|e| e.size).sum();
            s += &format!(" · {}개 선택", sel.len());
            if size > 0 {
                s += &format!(" ({})", crate::human_size(size));
            }
        }
        s
    }

    pub fn table_ui(&mut self, ui: &mut egui::Ui) -> FsAction {
        if let Some(e) = &self.error {
            let rect = ui.available_rect_before_wrap();
            table::empty_state(ui, rect, Icon::Folder, "폴더를 열 수 없습니다", e);
            return FsAction::None;
        }
        let mut action = FsAction::None;
        let cols = [
            Col { title: "이름", width: 0.0, right: false },
            Col { title: "수정한 날짜", width: 140.0, right: false },
            Col { title: "종류", width: 120.0, right: false },
            Col { title: "크기", width: 90.0, right: true },
        ];
        let before = self.sort;
        table::header(ui, &cols, &mut self.sort);
        if before != self.sort {
            self.sort_entries();
        }
        let mods = ui.input(|i| i.modifiers);
        let mut clicked: Option<PathBuf> = None;
        let mut menu_target: Option<PathBuf> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if self.entries.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| ui.weak("비어 있는 폴더입니다"));
            }
            for (i, e) in self.entries.iter().enumerate() {
                let icon = if e.is_dir {
                    Icon::Folder
                } else if e.archive {
                    Icon::Archive
                } else {
                    Icon::File
                };
                let date = e.modified.map(crate::fmt_time).unwrap_or_default();
                let size = if e.is_dir { String::new() } else { crate::human_size(e.size) };
                let cells = [(e.name.clone(), None), (date, None), (kind(e), None), (size, None)];
                let sel = self.selected.contains(&e.path);
                let r = table::row(ui, &cols, icon, &cells, sel, i);
                if r.clicked() {
                    clicked = Some(e.path.clone());
                }
                if r.secondary_clicked() && !sel {
                    menu_target = Some(e.path.clone());
                }
                if r.double_clicked() {
                    action = FsAction::Open(e.path.clone());
                }
                let any_archive = e.archive || self.selected.iter().any(|p| self.entries.iter().any(|x| &x.path == p && x.archive));
                let mut chosen: Option<FsCommand> = None;
                let mut open_it = false;
                r.context_menu(|ui| {
                    let mut item = |ui: &mut egui::Ui, label: &str, c: FsCommand| {
                        if ui.button(label).clicked() {
                            chosen = Some(c);
                            ui.close();
                        }
                    };
                    if ui.button(if e.archive { "압축 파일 안 보기" } else { "열기" }).clicked() {
                        open_it = true;
                        ui.close();
                    }
                    if any_archive {
                        ui.separator();
                        item(ui, "알아서 풀기", FsCommand::Extract);
                        item(ui, "여기에 압축 풀기", FsCommand::ExtractHere);
                        item(ui, "새 폴더에 압축 풀기", FsCommand::ExtractNewFolder);
                        item(ui, "다른 위치에 풀기…", FsCommand::ExtractTo);
                        item(ui, "압축 파일 검사", FsCommand::Test);
                    }
                    ui.separator();
                    item(ui, "압축하기", FsCommand::Compress);
                    item(ui, if cfg!(windows) { "탐색기에서 보기" } else { "파일 관리자에서 보기" }, FsCommand::ShowInSystem);
                });
                if open_it {
                    action = FsAction::Open(e.path.clone());
                }
                if let Some(c) = chosen {
                    action = FsAction::Command(c);
                }
            }
        });
        if let Some(p) = menu_target {
            self.selected = [p.clone()].into();
            self.anchor = Some(p);
        }
        if let Some(p) = clicked {
            self.click(&p, mods);
        }

        // 키보드 (글자 입력 중이 아닐 때만)
        if !ui.ctx().egui_wants_keyboard_input() {
            let (enter, back, esc, all) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Backspace),
                    i.key_pressed(egui::Key::Escape),
                    i.modifiers.command && i.key_pressed(egui::Key::A),
                )
            });
            if all {
                self.selected = self.entries.iter().map(|e| e.path.clone()).collect();
            }
            if esc {
                self.selected.clear();
            }
            if back {
                action = FsAction::Up;
            }
            if enter && self.selected.len() == 1 {
                action = FsAction::Open(self.selected.iter().next().cloned().unwrap_or_default());
            }
        }
        action
    }

    fn click(&mut self, path: &Path, mods: egui::Modifiers) {
        if mods.command {
            if !self.selected.remove(path) {
                self.selected.insert(path.to_path_buf());
            }
            self.anchor = Some(path.to_path_buf());
        } else if mods.shift
            && let Some(a) = &self.anchor
        {
            let pos = |p: &Path| self.entries.iter().position(|e| e.path == p);
            if let (Some(i), Some(j)) = (pos(a), pos(path)) {
                let (lo, hi) = (i.min(j), i.max(j));
                self.selected = self.entries[lo..=hi].iter().map(|e| e.path.clone()).collect();
            }
        } else {
            self.selected = [path.to_path_buf()].into();
            self.anchor = Some(path.to_path_buf());
        }
    }
}

fn kind(e: &FsEntry) -> String {
    if e.is_dir {
        "파일 폴더".into()
    } else if e.archive {
        let n = e.name.to_lowercase();
        let ext = ["tar.gz", "tar.bz2", "tar.xz", "tar.zst"].iter().find(|x| n.ends_with(&format!(".{x}"))).map(|x| x.to_string());
        let ext = ext.unwrap_or_else(|| n.rsplit('.').next().unwrap_or("").to_string());
        format!("{} 압축 파일", ext.to_uppercase())
    } else {
        crate::browse::kind_of(&e.name)
    }
}
