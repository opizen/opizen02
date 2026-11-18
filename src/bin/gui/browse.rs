//! 압축 파일 내용 보기 — 창 전체에 폴더 구조로 보여 줍니다.
//!
//! - 폴더 더블클릭(Enter): 들어가기 / Backspace: 위로
//! - 파일 더블클릭: 임시 폴더에 풀어서 기본 프로그램으로 열기
//! - Ctrl·Shift + 클릭: 여러 개 선택 → 도구 모음의 압축 풀기가 "선택 항목 풀기"가 됨

use crate::icons::{self, Icon};
use crate::style::ACCENT;
use crate::table::{self, Col};
use eframe::egui;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use unpack::{EntryInfo, Options, Selection, detect, is_password_error, list};

/// 현재 폴더 안의 한 항목
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: String,
    pub is_dir: bool,
    /// 파일 크기, 폴더면 안에 든 파일 크기 합
    pub size: u64,
    /// 폴더 안 파일 수 (파일이면 0)
    pub files: usize,
    /// 수정 시간 (폴더면 안에서 가장 최근)
    pub modified: Option<SystemTime>,
}

/// `entries` 중 `cur` 폴더 바로 아래 항목들 (폴더 먼저, 이름순)
pub fn children(entries: &[EntryInfo], cur: &[String]) -> Vec<Node> {
    let mut map: BTreeMap<String, Node> = BTreeMap::new();
    for e in entries {
        let name = e.name.replace('\\', "/");
        let segs: Vec<&str> = name.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
        if segs.len() <= cur.len() || segs[..cur.len()].iter().zip(cur).any(|(a, b)| a != b) {
            continue;
        }
        let child = segs[cur.len()].to_string();
        let deeper = segs.len() > cur.len() + 1;
        let is_dir = deeper || e.is_dir;
        let node = map.entry(child.clone()).or_insert(Node { name: child, is_dir, size: 0, files: 0, modified: None });
        node.is_dir |= is_dir;
        node.modified = node.modified.max(e.modified);
        if !e.is_dir {
            node.size += e.size;
            if node.is_dir {
                node.files += 1;
            }
        }
    }
    let mut v: Vec<Node> = map.into_values().collect();
    v.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    v
}

pub enum Contents {
    Ok(Vec<EntryInfo>),
    NeedPassword { wrong: bool },
    Failed(String),
}

/// 표에서 일어난 일 (Home이 처리)
pub enum ViewAction {
    None,
    /// 암호를 넣고 다시 열기
    Retry,
    /// 이 파일(압축 파일 안 전체 경로)을 기본 프로그램으로 열기
    Open(String),
    /// 선택한 항목 풀기 (`true`면 위치 고르기)
    ExtractSelected { choose: bool },
    /// 최상위에서 뒤로 → 압축 파일 밖(폴더)으로
    Exit,
}

pub struct ArchiveView {
    pub path: PathBuf,
    format: String,
    file_size: u64,
    pub contents: Contents,
    cur: Vec<String>,
    /// 암호 칸에 한 번만 포커스 주기
    focus_password: bool,
    /// 현재 폴더에서 고른 항목 이름들
    selected: BTreeSet<String>,
    /// Shift 선택 기준
    anchor: Option<String>,
    sort: table::Sort,
}

impl ArchiveView {
    pub fn open(path: &Path, password: Option<String>) -> Self {
        let format = detect(path).map(|f| f.to_string().to_uppercase()).unwrap_or_default();
        let file_size = unpack::archive_parts(path).iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        let mut v = Self {
            path: path.to_path_buf(),
            format,
            file_size,
            contents: Contents::Ok(Vec::new()),
            cur: Vec::new(),
            focus_password: false,
            selected: BTreeSet::new(),
            anchor: None,
            sort: (0, true),
        };
        v.reload(password);
        v
    }

    pub fn reload(&mut self, password: Option<String>) {
        let wrong = password.is_some();
        self.contents = match list(&self.path, &Options { password, ..Default::default() }) {
            Ok(e) => Contents::Ok(e),
            Err(e) if is_password_error(&e) => {
                self.focus_password = true;
                Contents::NeedPassword { wrong }
            }
            Err(e) => Contents::Failed(format!("{e:#}")),
        };
    }

    pub fn title(&self) -> String {
        crate::file_name(&self.path)
    }

    pub fn at_root(&self) -> bool {
        self.cur.is_empty()
    }

    pub fn go_up(&mut self) {
        self.cur.pop();
        self.clear_selection();
    }

    fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    pub fn has_selection(&self) -> bool {
        !self.selected.is_empty()
    }

    fn full(&self, name: &str) -> String {
        let mut segs = self.cur.clone();
        segs.push(name.to_string());
        segs.join("/")
    }

    /// 고른 항목 → 라이브러리 선택 (지금 보는 폴더 기준 경로로 풀림)
    pub fn selection(&self) -> Option<Selection> {
        self.has_selection().then(|| Selection {
            prefix: self.cur.join("/"),
            names: self.selected.iter().map(|n| self.full(n)).collect(),
        })
    }

    /// 상태 표시줄 문구
    pub fn status(&self) -> String {
        match &self.contents {
            Contents::Ok(e) => {
                let files = e.iter().filter(|x| !x.is_dir).count();
                let total: u64 = e.iter().map(|x| x.size).sum();
                let sel = if self.selected.is_empty() { String::new() } else { format!(" · {}개 선택", self.selected.len()) };
                format!(
                    "{} · 파일 {}개 · 풀면 {} · 압축 파일 {}{sel}",
                    self.format,
                    files,
                    crate::human_size(total),
                    crate::human_size(self.file_size)
                )
            }
            _ => format!("{} · {}", self.format, crate::human_size(self.file_size)),
        }
    }

    /// 주소 줄: [압축파일] › 폴더 › 폴더
    pub fn crumbs_ui(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = 2.0;
        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        icons::paint(ui.painter(), r, Icon::Archive, ACCENT);
        let mut go = None;
        let root = self.title();
        if self.cur.is_empty() {
            ui.strong(root);
        } else if ui.add(egui::Button::new(root).frame(false)).clicked() {
            go = Some(0);
        }
        for (i, seg) in self.cur.iter().enumerate() {
            ui.weak(" › ");
            if i + 1 == self.cur.len() {
                ui.strong(seg);
            } else if ui.add(egui::Button::new(seg.as_str()).frame(false)).clicked() {
                go = Some(i + 1);
            }
        }
        if let Some(n) = go {
            self.cur.truncate(n);
            self.clear_selection();
        }
    }

    /// 내용 표
    pub fn table_ui(&mut self, ui: &mut egui::Ui, password: &mut String) -> ViewAction {
        let mut action = ViewAction::None;
        match &self.contents {
            Contents::Failed(e) => {
                let rect = ui.available_rect_before_wrap();
                table::empty_state(ui, rect, Icon::File, "내용을 읽을 수 없습니다", e);
            }
            Contents::NeedPassword { wrong } => {
                let wrong = *wrong;
                let rect = ui.available_rect_before_wrap();
                let top = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), rect.height() * 0.62));
                table::empty_state(
                    ui,
                    top,
                    Icon::Lock,
                    "암호가 걸린 압축 파일입니다",
                    if wrong { "암호가 올바르지 않습니다. 다시 입력하세요." } else { "암호를 입력하면 내용을 볼 수 있습니다" },
                );
                ui.add_space(top.height() - 10.0);
                ui.vertical_centered(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(password).password(true).hint_text("암호").desired_width(240.0));
                    if std::mem::take(&mut self.focus_password) {
                        r.request_focus();
                    }
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(4.0);
                    if (ui.add_enabled(!password.is_empty(), crate::style::primary("열기")).clicked() || enter)
                        && !password.is_empty()
                    {
                        action = ViewAction::Retry;
                    }
                });
            }
            Contents::Ok(entries) => {
                let mut nodes = children(entries, &self.cur);
                sort_nodes(&mut nodes, self.sort);
                // 폴더 보기와 같은 순서: 이름 / 수정한 날짜 / 종류 / 크기
                let cols = [
                    Col { title: "이름", width: 0.0, right: false },
                    Col { title: "수정한 날짜", width: 140.0, right: false },
                    Col { title: "종류", width: 120.0, right: false },
                    Col { title: "크기", width: 90.0, right: true },
                ];
                table::header(ui, &cols, &mut self.sort);
                let mut enter = None;
                let mut up = false;
                let mods = ui.input(|i| i.modifiers);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if !self.cur.is_empty() {
                        let r = table::row(ui, &cols, Icon::Up, &[("상위 폴더".into(), Some(ACCENT))], false, 0);
                        if r.double_clicked() || r.clicked() {
                            up = true;
                        }
                    }
                    for (i, n) in nodes.iter().enumerate() {
                        let (icon, kind) = if n.is_dir {
                            (Icon::Folder, format!("폴더 · {}개", n.files))
                        } else {
                            (Icon::File, kind_of(&n.name))
                        };
                        let date = n.modified.map(crate::fmt_time).unwrap_or_default();
                        let sel = self.selected.contains(&n.name);
                        let cells = [(n.name.clone(), None), (date, None), (kind, None), (crate::human_size(n.size), None)];
                        let r = table::row(ui, &cols, icon, &cells, sel, i + usize::from(!self.cur.is_empty()));
                        if r.clicked() {
                            self.click(&nodes, &n.name, mods);
                        }
                        if r.secondary_clicked() && !sel {
                            self.clear_selection();
                            self.selected.insert(n.name.clone());
                            self.anchor = Some(n.name.clone());
                        }
                        if r.double_clicked() {
                            if n.is_dir {
                                enter = Some(n.name.clone());
                            } else {
                                action = ViewAction::Open(self.full(&n.name));
                            }
                        }
                        r.context_menu(|ui| {
                            if !n.is_dir && ui.button("열기").clicked() {
                                action = ViewAction::Open(self.full(&n.name));
                                ui.close();
                            }
                            if n.is_dir && ui.button("폴더 열기").clicked() {
                                enter = Some(n.name.clone());
                                ui.close();
                            }
                            ui.separator();
                            let label = format!("선택한 {}개 풀기", self.selected.len().max(1));
                            if ui.button(label).clicked() {
                                action = ViewAction::ExtractSelected { choose: false };
                                ui.close();
                            }
                            if ui.button("선택한 항목을 다른 위치에 풀기…").clicked() {
                                action = ViewAction::ExtractSelected { choose: true };
                                ui.close();
                            }
                        });
                    }
                    if nodes.is_empty() {
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| ui.weak("비어 있는 폴더입니다"));
                    }
                });
                // 키보드: Backspace 위로, Enter 열기, Esc 선택 해제, Ctrl+A 모두 선택
                let (back, open, esc, all) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::Backspace),
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::Escape),
                        i.modifiers.command && i.key_pressed(egui::Key::A),
                    )
                });
                if back {
                    if self.cur.is_empty() {
                        action = ViewAction::Exit;
                    } else {
                        up = true;
                    }
                }
                if esc {
                    self.clear_selection();
                }
                if all {
                    self.selected = nodes.iter().map(|n| n.name.clone()).collect();
                }
                if open && self.selected.len() == 1 {
                    let name = self.selected.iter().next().cloned().unwrap_or_default();
                    match nodes.iter().find(|n| n.name == name) {
                        Some(n) if n.is_dir => enter = Some(name),
                        Some(_) => action = ViewAction::Open(self.full(&name)),
                        None => {}
                    }
                }
                if up {
                    self.go_up();
                }
                if let Some(d) = enter {
                    self.cur.push(d);
                    self.clear_selection();
                }
            }
        }
        action
    }

    /// 클릭: 그냥 누르면 하나만, Ctrl은 추가/빼기, Shift는 범위
    fn click(&mut self, nodes: &[Node], name: &str, mods: egui::Modifiers) {
        if mods.command {
            if !self.selected.remove(name) {
                self.selected.insert(name.to_string());
            }
            self.anchor = Some(name.to_string());
        } else if mods.shift
            && let Some(a) = &self.anchor
        {
            let pos = |n: &str| nodes.iter().position(|x| x.name == n);
            if let (Some(i), Some(j)) = (pos(a), pos(name)) {
                let (lo, hi) = (i.min(j), i.max(j));
                self.selected = nodes[lo..=hi].iter().map(|n| n.name.clone()).collect();
            }
        } else {
            self.clear_selection();
            self.selected.insert(name.to_string());
            self.anchor = Some(name.to_string());
        }
    }
}

fn sort_nodes(nodes: &mut [Node], (col, asc): table::Sort) {
    nodes.sort_by(|a, b| {
        let dirs = b.is_dir.cmp(&a.is_dir); // 폴더는 항상 위
        let ord = match col {
            1 => a.modified.cmp(&b.modified),
            2 => kind_of(&a.name).cmp(&kind_of(&b.name)),
            3 => a.size.cmp(&b.size),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        dirs.then(if asc { ord } else { ord.reverse() })
    });
}

/// 확장자로 보여 줄 종류 이름
pub fn kind_of(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && ext.len() <= 6 => format!("{} 파일", ext.to_uppercase()),
        _ => "파일".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, size: u64, is_dir: bool) -> EntryInfo {
        EntryInfo { name: name.into(), size, is_dir, modified: None }
    }

    #[test]
    fn builds_tree_levels() {
        let entries = vec![
            e("project/", 0, true),
            e("project/a.txt", 10, false),
            e("project/sub/b.txt", 5, false),
            e("c.txt", 3, false),
            e("implicit/x.bin", 7, false), // 폴더 항목이 따로 없는 경우
        ];
        let top = children(&entries, &[]);
        let names: Vec<_> = top.iter().map(|n| (n.name.as_str(), n.is_dir, n.size, n.files)).collect();
        assert_eq!(names, vec![("implicit", true, 7, 1), ("project", true, 15, 2), ("c.txt", false, 3, 0)]);

        let inner = children(&entries, &["project".into()]);
        let names: Vec<_> = inner.iter().map(|n| (n.name.as_str(), n.is_dir)).collect();
        assert_eq!(names, vec![("sub", true), ("a.txt", false)]);
        assert!(children(&entries, &["nope".into()]).is_empty());
    }
}
