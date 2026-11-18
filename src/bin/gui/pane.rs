//! 탐색기 창 하나 (화면을 최대 4개로 나눌 수 있음).
//!
//! 창마다 주소 줄(뒤로·앞으로·위로·경로), 폴더 표 또는 압축 파일 안 보기, 선택이 따로 있습니다.

use crate::browse::{ArchiveView, ViewAction};
use crate::dnd::DragItems;
use crate::fsview::{FolderView, FsAction};
use crate::icons::{self, Icon};
use crate::places;
use crate::style::{self, ACCENT};
use eframe::egui::{self, Margin, RichText};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Pane {
    pub folder: FolderView,
    /// 압축 파일 안을 보는 중이면 Some
    pub view: Option<ArchiveView>,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// 주소 줄에 경로를 직접 입력하는 중
    pub editing_path: Option<String>,
}

/// 창에서 일어난 일 (Home이 처리)
pub struct PaneOut {
    pub fs: FsAction,
    pub view: ViewAction,
    pub close: bool,
    /// 이 창을 눌렀음 (활성 창으로)
    pub activated: bool,
    /// 이 창에 끌어다 놓음: (내용물, 놓을 폴더, Ctrl 누름 여부)
    pub dropped: Option<(Arc<DragItems>, PathBuf, bool)>,
    /// 이동할 곳 (주소 줄에서 입력하거나 누름)
    pub go: Option<PathBuf>,
}

impl Pane {
    pub fn new(dir: PathBuf, show_hidden: bool) -> Self {
        Self { folder: FolderView::new(dir, show_hidden), view: None, back: Vec::new(), forward: Vec::new(), editing_path: None }
    }

    pub fn dir(&self) -> &Path {
        &self.folder.dir
    }

    pub fn title(&self) -> String {
        match &self.view {
            Some(v) => v.title(),
            None => crate::file_name(&self.folder.dir),
        }
    }

    pub fn status(&self) -> String {
        match &self.view {
            Some(v) => v.status(),
            None => self.folder.status(),
        }
    }

    pub fn reload(&mut self, show_hidden: bool) {
        self.folder.reload(show_hidden);
    }

    // ----- 이동 -----

    pub fn navigate(&mut self, dir: PathBuf, show_hidden: bool) {
        if dir == self.folder.dir && self.view.is_none() {
            return;
        }
        self.back.push(self.folder.dir.clone());
        self.forward.clear();
        self.go_to(dir, show_hidden);
    }

    fn go_to(&mut self, dir: PathBuf, show_hidden: bool) {
        self.view = None;
        self.folder = FolderView::new(dir, show_hidden);
        self.editing_path = None;
    }

    pub fn go_back(&mut self, show_hidden: bool) {
        if self.view.is_some() {
            self.view = None;
            return;
        }
        if let Some(d) = self.back.pop() {
            self.forward.push(self.folder.dir.clone());
            self.go_to(d, show_hidden);
        }
    }

    pub fn go_forward(&mut self, show_hidden: bool) {
        if let Some(d) = self.forward.pop() {
            self.back.push(self.folder.dir.clone());
            self.go_to(d, show_hidden);
        }
    }

    pub fn go_up(&mut self, show_hidden: bool) {
        if let Some(v) = self.view.as_mut() {
            if v.at_root() {
                self.view = None;
            } else {
                v.go_up();
            }
            return;
        }
        let cur = self.folder.dir.clone();
        if let Some(parent) = cur.parent() {
            self.navigate(parent.to_path_buf(), show_hidden);
            self.folder.select_only(&[cur]);
        }
    }

    pub fn open_archive(&mut self, path: PathBuf, password: Option<String>, show_hidden: bool) {
        if let Some(parent) = path.parent()
            && parent != self.folder.dir
        {
            self.navigate(parent.to_path_buf(), show_hidden);
        }
        self.folder.select_only(std::slice::from_ref(&path));
        self.view = Some(ArchiveView::open(&path, password));
    }

    /// 지금 고른 압축 파일들 (압축 파일 안을 보는 중이면 그 파일)
    pub fn selected_archives(&self) -> Vec<PathBuf> {
        if let Some(v) = &self.view {
            return if matches!(v.contents, crate::browse::Contents::Ok(_)) { vec![v.path.clone()] } else { vec![] };
        }
        self.folder.selection().into_iter().filter(|e| e.archive).map(|e| e.path.clone()).collect()
    }

    // ----- 화면 -----

    fn nav_button(ui: &mut egui::Ui, icon: Icon, enabled: bool, tip: &str) -> bool {
        let (r, resp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
        if enabled && resp.hovered() {
            ui.painter().rect_filled(r, 6.0, style::hover_fill(ui));
        }
        let color = if enabled { ui.visuals().text_color() } else { style::weak(ui).gamma_multiply(0.5) };
        icons::paint(ui.painter(), r.shrink(7.0), icon, color);
        enabled && resp.on_hover_text(tip).clicked()
    }

    /// 창 하나 그리기. `rect`는 이 창이 차지할 자리.
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        active: bool,
        split: bool,
        password: &mut String,
        show_hidden: bool,
        can_paste: bool,
    ) -> PaneOut {
        let mut out = PaneOut { fs: FsAction::None, view: ViewAction::None, close: false, activated: false, dropped: None, go: None };
        let rect = ui.max_rect();
        // 나뉜 상태에서는 테두리로 구분하고, 활성 창은 파란 테두리
        if split {
            let color = if active { ACCENT } else { style::border(ui) };
            ui.painter().rect_stroke(rect.shrink(0.5), 8.0, egui::Stroke::new(if active { 1.5 } else { 1.0 }, color), egui::StrokeKind::Inside);
        }
        let inner = if split { rect.shrink(4.0) } else { rect };
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
        // 옆 창으로 넘쳐 그려지지 않도록
        ui.set_clip_rect(inner);
        let ui = &mut ui;

        // --- 주소 줄 ---
        let (mut back, mut fwd, mut up) = (false, false, false);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            back = Self::nav_button(ui, Icon::Back, !self.back.is_empty() || self.view.is_some(), "뒤로 (Alt+←)");
            fwd = Self::nav_button(ui, Icon::Forward, !self.forward.is_empty() && self.view.is_none(), "앞으로 (Alt+→)");
            up = Self::nav_button(ui, Icon::Up, self.view.is_some() || self.folder.dir.parent().is_some(), "위로 (Alt+↑)");
            ui.add_space(4.0);
            let close_w = if split { 30.0 } else { 0.0 };
            let field_w = (ui.available_width() - close_w - 18.0).max(80.0);
            let field = egui::Frame::NONE
                .fill(ui.visuals().extreme_bg_color)
                .stroke(egui::Stroke::new(1.0, style::border(ui)))
                .corner_radius(6)
                .inner_margin(Margin { left: 8, right: 8, top: 2, bottom: 2 });
            field.show(ui, |ui| {
                ui.set_width(field_w);
                ui.set_max_width(field_w);
                if let Some(text) = self.editing_path.as_mut() {
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).frame(egui::Frame::NONE));
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    if r.lost_focus() {
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            out.go = Some(PathBuf::from(text.trim()));
                        }
                        self.editing_path = None;
                    }
                    return;
                }
                let resp = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r.shrink2(egui::vec2(0.0, 2.0)), Icon::Folder, ACCENT);
                        ui.add_space(4.0);
                        let crumbs = places::crumbs(&self.folder.dir);
                        // 뒤쪽부터 들어가는 만큼만 (글자 폭을 대략 계산)
                        let budget = (field_w - 50.0 - if self.view.is_some() { 160.0 } else { 0.0 }).max(60.0);
                        let mut used = 0.0;
                        let mut fit = 0;
                        for (label, _) in crumbs.iter().rev() {
                            used += label.chars().map(|c| if c.is_ascii() { 7.5 } else { 14.0 }).sum::<f32>() + 18.0;
                            if used > budget && fit > 0 {
                                break;
                            }
                            fit += 1;
                        }
                        let skip = crumbs.len() - fit;
                        if skip > 0 {
                            ui.weak("… › ");
                        }
                        let last = crumbs.len().saturating_sub(1);
                        for (i, (label, path)) in crumbs.into_iter().enumerate().skip(skip) {
                            let is_last = i == last && self.view.is_none();
                            let text = if is_last { RichText::new(label).strong() } else { RichText::new(label) };
                            if ui.add(egui::Button::new(text).frame(false)).clicked() {
                                out.go = Some(path);
                            }
                            if i != last || self.view.is_some() {
                                ui.weak(" › ");
                            }
                        }
                        if let Some(v) = self.view.as_mut() {
                            v.crumbs_ui(ui);
                        }
                        // 나머지 빈 곳을 누르면 경로 입력
                        ui.allocate_response(egui::vec2(ui.available_width(), 22.0), egui::Sense::click())
                    })
                    .inner;
                if resp.on_hover_cursor(egui::CursorIcon::Text).clicked() {
                    self.editing_path = Some(self.folder.dir.display().to_string());
                }
            });
            if split && Self::nav_button(ui, Icon::Close, true, "이 창 닫기") {
                out.close = true;
            }
        });
        ui.add_space(2.0);
        if back {
            self.go_back(show_hidden);
        } else if fwd {
            self.go_forward(show_hidden);
        } else if up {
            self.go_up(show_hidden);
        }

        // --- 표 ---
        if let Some(view) = self.view.as_mut() {
            out.view = view.table_ui(ui, password, idx, active);
        } else {
            out.fs = self.folder.table_ui(ui, idx, active, can_paste);
        }

        // --- 활성 창 / 끌어다 놓기 ---
        let pointer = ui.ctx().pointer_latest_pos();
        let inside = pointer.is_some_and(|p| rect.contains(p));
        if inside && ui.input(|i| i.pointer.any_pressed()) {
            out.activated = true;
        }
        if let Some(payload) = egui::DragAndDrop::payload::<DragItems>(ui.ctx())
            && inside
        {
            let from_here = payload.pane() == idx;
            let target = self.folder.drop_folder.clone();
            // 같은 창의 빈 곳에 놓는 것은 아무 일도 아님 (폴더 줄에 놓을 때만 의미)
            let meaningful = self.view.is_none() && (!from_here || target.is_some());
            if meaningful && target.is_none() {
                crate::table::drop_highlight(ui, inner);
            }
            if meaningful && ui.input(|i| i.pointer.any_released()) {
                let ctrl = ui.input(|i| i.modifiers.command);
                if let Some(p) = egui::DragAndDrop::take_payload::<DragItems>(ui.ctx()) {
                    out.dropped = Some((p, target.unwrap_or_else(|| self.folder.dir.clone()), ctrl));
                }
            }
        }
        out
    }
}
