//! 폴더 보기 — 지금 폴더의 파일·폴더를 표로 보여 줍니다 (파일 탐색기).
//!
//! - 더블클릭(Enter): 폴더는 들어가고, 압축 파일은 안을 보고, 그 밖의 파일은 기본 프로그램으로 엶
//! - Ctrl·Shift + 클릭, Ctrl+A: 여러 개 선택 / Esc: 선택 해제 / Backspace: 위 폴더
//! - 끌어서 다른 창(또는 폴더 줄)에 놓으면 이동, Ctrl을 누르고 놓으면 복사

use crate::dnd::DragItems;
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
    /// 우클릭 메뉴·단축키 명령
    Command(FsCommand),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FsCommand {
    Extract,
    ExtractHere,
    ExtractNewFolder,
    ExtractTo,
    Test,
    Compress,
    ShowInSystem,
    Copy,
    Cut,
    Paste,
    Rename,
    NewFolder,
    Trash,
}

pub struct FolderView {
    pub dir: PathBuf,
    entries: Vec<FsEntry>,
    pub error: Option<String>,
    pub selected: BTreeSet<PathBuf>,
    anchor: Option<PathBuf>,
    sort: table::Sort,
    /// 끄는 중에 마우스가 올라간 폴더 줄 (놓으면 그 폴더로)
    pub drop_folder: Option<PathBuf>,
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
        let mut v = Self {
            dir,
            entries: Vec::new(),
            error: None,
            selected: BTreeSet::new(),
            anchor: None,
            sort: (0, true),
            drop_folder: None,
        };
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

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selection().into_iter().map(|e| e.path.clone()).collect()
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

    /// `pane`: 이 표가 있는 창 번호 (끌기용), `focused`: 키보드를 받을 창인지
    pub fn table_ui(&mut self, ui: &mut egui::Ui, pane: usize, focused: bool, can_paste: bool) -> FsAction {
        self.drop_folder = None;
        if let Some(e) = &self.error {
            let rect = ui.available_rect_before_wrap();
            table::empty_state(ui, rect, Icon::Folder, "폴더를 열 수 없습니다", e);
            return FsAction::None;
        }
        let mut action = FsAction::None;
        let all = [
            Col { id: 0, title: "이름", width: 0.0, right: false },
            Col { id: 1, title: "수정한 날짜", width: 130.0, right: false },
            Col { id: 2, title: "종류", width: 110.0, right: false },
            Col { id: 3, title: "크기", width: 80.0, right: true },
        ];
        let shown = table::fit_columns(&all, ui.available_width());
        let cols: Vec<Col> = shown.iter().map(|&i| Col { ..all[i] }).collect();
        let before = self.sort;
        table::header(ui, &cols, &mut self.sort);
        if before != self.sort {
            self.sort_entries();
        }
        let mods = ui.input(|i| i.modifiers);
        let dragging: Option<std::sync::Arc<DragItems>> = egui::DragAndDrop::payload::<DragItems>(ui.ctx());
        let mut clicked: Option<PathBuf> = None;
        let mut menu_target: Option<PathBuf> = None;
        let mut start_drag: Option<PathBuf> = None;
        let mut chosen: Option<FsCommand> = None;
        let mut open_it: Option<PathBuf> = None;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if self.entries.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| ui.weak("비어 있는 폴더입니다"));
            }
            let any_archive_selected = self.selection().iter().any(|e| e.archive);
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
                let full = [(e.name.clone(), None), (date, None), (kind(e), None), (size, None)];
                let cells: Vec<_> = shown.iter().map(|&i| full[i].clone()).collect();
                let sel = self.selected.contains(&e.path);
                let r = table::row_ex(ui, &cols, icon, &cells, sel, i, true);
                if r.clicked() {
                    clicked = Some(e.path.clone());
                }
                if r.secondary_clicked() && !sel {
                    menu_target = Some(e.path.clone());
                }
                if r.double_clicked() {
                    action = FsAction::Open(e.path.clone());
                }
                if r.drag_started() {
                    start_drag = Some(e.path.clone());
                }
                // 끄는 중에 폴더 위에 있으면 그 폴더가 놓을 곳
                if let Some(d) = &dragging
                    && e.is_dir
                    && r.contains_pointer()
                    && !matches!(&**d, DragItems::Files { paths, .. } if paths.contains(&e.path))
                {
                    table::drop_highlight(ui, r.rect);
                    self.drop_folder = Some(e.path.clone());
                }
                let any_archive = e.archive || any_archive_selected;
                r.context_menu(|ui| {
                    let mut item = |ui: &mut egui::Ui, label: &str, c: FsCommand| {
                        if ui.button(label).clicked() {
                            chosen = Some(c);
                            ui.close();
                        }
                    };
                    if ui.button(if e.archive { "압축 파일 안 보기" } else { "열기" }).clicked() {
                        open_it = Some(e.path.clone());
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
                    ui.separator();
                    item(ui, "잘라내기  (Ctrl+X)", FsCommand::Cut);
                    item(ui, "복사  (Ctrl+C)", FsCommand::Copy);
                    if can_paste {
                        item(ui, "붙여넣기  (Ctrl+V)", FsCommand::Paste);
                    }
                    item(ui, "이름 바꾸기  (F2)", FsCommand::Rename);
                    item(ui, "휴지통으로  (Delete)", FsCommand::Trash);
                    ui.separator();
                    item(ui, if cfg!(windows) { "탐색기에서 보기" } else { "파일 관리자에서 보기" }, FsCommand::ShowInSystem);
                });
            }
            // 빈 곳: 새 폴더 / 붙여넣기
            let rest = ui.available_size_before_wrap();
            let (_, blank) = ui.allocate_exact_size(egui::vec2(rest.x, rest.y.max(40.0)), egui::Sense::click());
            if blank.clicked() && !mods.command && !mods.shift {
                self.selected.clear();
            }
            blank.context_menu(|ui| {
                if ui.button("새 폴더  (Ctrl+Shift+N)").clicked() {
                    chosen = Some(FsCommand::NewFolder);
                    ui.close();
                }
                if ui.add_enabled(can_paste, egui::Button::new("붙여넣기  (Ctrl+V)")).clicked() {
                    chosen = Some(FsCommand::Paste);
                    ui.close();
                }
            });
        });

        if let Some(p) = menu_target {
            self.selected = [p.clone()].into();
            self.anchor = Some(p);
        }
        if let Some(p) = clicked {
            self.click(&p, mods);
        }
        if let Some(p) = start_drag {
            // 고르지 않은 줄을 끌면 그 줄만
            if !self.selected.contains(&p) {
                self.selected = [p.clone()].into();
                self.anchor = Some(p);
            }
            egui::DragAndDrop::set_payload(ui.ctx(), DragItems::Files { pane, paths: self.selected_paths() });
        }
        if let Some(p) = open_it {
            action = FsAction::Open(p);
        }
        if let Some(c) = chosen {
            action = FsAction::Command(c);
        }

        // 키보드 (이 창이 활성이고 글자 입력 중이 아닐 때만)
        if focused && !ui.ctx().egui_wants_keyboard_input() {
            let k = ui.input(|i| {
                let cmd = i.modifiers.command;
                [
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Backspace),
                    i.key_pressed(egui::Key::Escape),
                    cmd && i.key_pressed(egui::Key::A),
                    // Ctrl+C/X/V는 egui가 키가 아니라 복사·잘라내기·붙여넣기 이벤트로 바꿔서 줌
                    i.events.iter().any(|e| matches!(e, egui::Event::Copy)) || (cmd && i.key_pressed(egui::Key::C)),
                    i.events.iter().any(|e| matches!(e, egui::Event::Cut)) || (cmd && i.key_pressed(egui::Key::X)),
                    i.events.iter().any(|e| matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_))) || (cmd && i.key_pressed(egui::Key::V)),
                    i.key_pressed(egui::Key::F2),
                    i.key_pressed(egui::Key::Delete),
                    cmd && i.modifiers.shift && i.key_pressed(egui::Key::N),
                ]
            });
            let [enter, back, esc, all, copy, cut, paste, f2, del, newdir] = k;
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
            let has_sel = !self.selected.is_empty();
            for (pressed, c, need_sel) in [
                (copy, FsCommand::Copy, true),
                (cut, FsCommand::Cut, true),
                (paste, FsCommand::Paste, false),
                (f2, FsCommand::Rename, true),
                (del, FsCommand::Trash, true),
                (newdir, FsCommand::NewFolder, false),
            ] {
                if pressed && (has_sel || !need_sel) {
                    action = FsAction::Command(c);
                }
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
