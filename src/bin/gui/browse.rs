//! 압축 파일 내용 보기 — 창 전체에 폴더 구조로 보여 줍니다.

use crate::icons::{self, Icon};
use crate::style::ACCENT;
use crate::table::{self, Col};
use eframe::egui;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use unpack::{EntryInfo, Options, detect, is_password_error, list};

/// 현재 폴더 안의 한 항목
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: String,
    pub is_dir: bool,
    /// 파일 크기, 폴더면 안에 든 파일 크기 합
    pub size: u64,
    /// 폴더 안 파일 수 (파일이면 0)
    pub files: usize,
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
        let node = map.entry(child.clone()).or_insert(Node { name: child, is_dir, size: 0, files: 0 });
        node.is_dir |= is_dir;
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

pub struct ArchiveView {
    /// Home 목록에서의 위치
    pub idx: usize,
    pub path: PathBuf,
    format: String,
    file_size: u64,
    pub contents: Contents,
    cur: Vec<String>,
    /// 암호 칸에 한 번만 포커스 주기
    focus_password: bool,
    selected: Option<String>,
    sort: table::Sort,
}

impl ArchiveView {
    pub fn open(idx: usize, path: &Path, password: Option<String>) -> Self {
        let format = detect(path).map(|f| f.to_string().to_uppercase()).unwrap_or_default();
        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let mut v = Self { idx, path: path.to_path_buf(), format, file_size, contents: Contents::Ok(Vec::new()), cur: Vec::new(), focus_password: false, selected: None, sort: (0, true) };
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
        self.selected = None;
    }

    /// 상태 표시줄 문구
    pub fn status(&self) -> String {
        match &self.contents {
            Contents::Ok(e) => {
                let files = e.iter().filter(|x| !x.is_dir).count();
                let total: u64 = e.iter().map(|x| x.size).sum();
                format!(
                    "{} · 파일 {}개 · 풀면 {} · 압축 파일 {}",
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
            self.selected = None;
        }
    }

    /// 내용 표. 암호가 필요하면 입력 화면을 보여 주고, 다시 열어야 하면 `true`.
    pub fn table_ui(&mut self, ui: &mut egui::Ui, password: &mut String) -> bool {
        let mut retry = false;
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
                        retry = true;
                    }
                });
            }
            Contents::Ok(entries) => {
                let mut nodes = children(entries, &self.cur);
                sort_nodes(&mut nodes, self.sort);
                let cols = [
                    Col { title: "이름", width: 0.0, right: false },
                    Col { title: "크기", width: 110.0, right: true },
                    Col { title: "종류", width: 120.0, right: false },
                ];
                table::header(ui, &cols, &mut self.sort);
                let mut enter = None;
                let mut up = false;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if !self.cur.is_empty() {
                        let r = table::row(ui, &cols, Icon::Up, &[("상위 폴더".into(), Some(ACCENT))], false, 0);
                        if r.double_clicked() || r.clicked() {
                            up = true;
                        }
                    }
                    for (i, n) in nodes.iter().enumerate() {
                        let (icon, size, kind) = if n.is_dir {
                            (Icon::Folder, crate::human_size(n.size), format!("폴더 · {}개", n.files))
                        } else {
                            (Icon::File, crate::human_size(n.size), kind_of(&n.name))
                        };
                        let sel = self.selected.as_deref() == Some(n.name.as_str());
                        let r = table::row(ui, &cols, icon, &[(n.name.clone(), None), (size, None), (kind, None)], sel, i + usize::from(!self.cur.is_empty()));
                        if r.clicked() {
                            self.selected = Some(n.name.clone());
                        }
                        if r.double_clicked() && n.is_dir {
                            enter = Some(n.name.clone());
                        }
                    }
                    if nodes.is_empty() {
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| ui.weak("비어 있는 폴더입니다"));
                    }
                });
                // 키보드: Backspace 위로, Enter 선택한 폴더 열기
                let (back, open) = ui.input(|i| (i.key_pressed(egui::Key::Backspace), i.key_pressed(egui::Key::Enter)));
                if back && !self.cur.is_empty() {
                    up = true;
                }
                if open
                    && let Some(sel) = &self.selected
                    && nodes.iter().any(|n| &n.name == sel && n.is_dir)
                {
                    enter = Some(sel.clone());
                }
                if up {
                    self.go_up();
                }
                if let Some(d) = enter {
                    self.cur.push(d);
                    self.selected = None;
                }
            }
        }
        retry
    }
}

fn sort_nodes(nodes: &mut [Node], (col, asc): table::Sort) {
    nodes.sort_by(|a, b| {
        let dirs = b.is_dir.cmp(&a.is_dir); // 폴더는 항상 위
        let ord = match col {
            1 => a.size.cmp(&b.size),
            2 => kind_of(&a.name).cmp(&kind_of(&b.name)),
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
        EntryInfo { name: name.into(), size, is_dir }
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
