//! 압축 파일 내용 보기 — 창 전체에 폴더 구조로 보여 줍니다.

use crate::style::{ACCENT, ERR, WARN};
use eframe::egui::{self, RichText};
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
}

impl ArchiveView {
    pub fn open(idx: usize, path: &Path, password: Option<String>) -> Self {
        let format = detect(path).map(|f| f.to_string().to_uppercase()).unwrap_or_default();
        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let mut v = Self { idx, path: path.to_path_buf(), format, file_size, contents: Contents::Ok(Vec::new()), cur: Vec::new(), focus_password: false };
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

    /// 머리글: 이름, 형식·항목 수·크기, 현재 위치
    pub fn header_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(self.title());
        });
        let mut info = format!("{} · {}", self.format, crate::human_size(self.file_size));
        if let Contents::Ok(e) = &self.contents {
            let files = e.iter().filter(|x| !x.is_dir).count();
            let total: u64 = e.iter().map(|x| x.size).sum();
            info = format!("{} · 파일 {}개 · 풀면 {} · 압축 파일 {}", self.format, files, crate::human_size(total), crate::human_size(self.file_size));
        }
        ui.weak(info);
        ui.add_space(4.0);

        // 경로: 최상위 › a › b
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let mut go = None;
            if self.cur.is_empty() {
                ui.strong("최상위");
            } else if ui.link("최상위").clicked() {
                go = Some(0);
            }
            for (i, seg) in self.cur.iter().enumerate() {
                ui.weak("›");
                if i + 1 == self.cur.len() {
                    ui.strong(seg);
                } else if ui.link(seg).clicked() {
                    go = Some(i + 1);
                }
            }
            if let Some(n) = go {
                self.cur.truncate(n);
            }
        });
        ui.separator();
    }

    /// 목록. 암호가 필요하면 `password` 입력 칸을 보여 주고, 다시 열면 `true`.
    pub fn list_ui(&mut self, ui: &mut egui::Ui, password: &mut String) -> bool {
        let mut retry = false;
        match &self.contents {
            Contents::Failed(e) => {
                ui.colored_label(ERR, "내용을 읽을 수 없습니다");
                ui.weak(e);
            }
            Contents::NeedPassword { wrong } => {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("🔒 암호가 걸린 압축 파일입니다").size(17.0));
                    if *wrong {
                        ui.colored_label(WARN, "암호가 올바르지 않습니다");
                    }
                    ui.add_space(6.0);
                    let r = ui.add(egui::TextEdit::singleline(password).password(true).hint_text("암호").desired_width(220.0));
                    if std::mem::take(&mut self.focus_password) {
                        r.request_focus();
                    }
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.add_enabled(!password.is_empty(), crate::style::primary("열기")).clicked() || enter)
                        && !password.is_empty()
                    {
                        retry = true;
                    }
                });
            }
            Contents::Ok(entries) => {
                let nodes = children(entries, &self.cur);
                let mut enter = None;
                let mut up = false;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if !self.cur.is_empty() && row(ui, "⬆", "..", "", true).clicked() {
                        up = true;
                    }
                    if nodes.is_empty() {
                        ui.weak("비어 있습니다");
                    }
                    for n in &nodes {
                        let (icon, right) = if n.is_dir {
                            ("📁", format!("파일 {}개 · {}", n.files, crate::human_size(n.size)))
                        } else {
                            ("📄", crate::human_size(n.size))
                        };
                        if row(ui, icon, &n.name, &right, n.is_dir).clicked() && n.is_dir {
                            enter = Some(n.name.clone());
                        }
                    }
                });
                if up {
                    self.cur.pop();
                }
                if let Some(d) = enter {
                    self.cur.push(d);
                }
            }
        }
        retry
    }
}

/// 한 줄: 아이콘, 이름, 오른쪽 정보. 폴더 줄은 클릭할 수 있음.
fn row(ui: &mut egui::Ui, icon: &str, name: &str, right: &str, clickable: bool) -> egui::Response {
    let h = 30.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::click());
    if clickable && resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let text = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let y = rect.center().y;
    let p = ui.painter();
    p.text(egui::pos2(rect.left() + 6.0, y), egui::Align2::LEFT_CENTER, icon, egui::FontId::proportional(15.0), if clickable { ACCENT } else { weak });
    p.text(egui::pos2(rect.left() + 32.0, y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(15.0), text);
    p.text(egui::pos2(rect.right() - 6.0, y), egui::Align2::RIGHT_CENTER, right, egui::FontId::proportional(13.0), weak);
    if clickable { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
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
