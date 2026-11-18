//! 탐색기 창 하나 (화면을 최대 4개로 나눌 수 있음). 창마다 탭을 여러 개 열 수 있습니다.
//!
//! 탭마다 주소 줄(뒤로·앞으로·위로·경로), 폴더 표(목록·아이콘) 또는 압축 파일 안 보기, 검색 결과,
//! 선택이 따로 있습니다. 창 왼쪽에는 바로가기(즐겨찾기·드라이브)가 있습니다.

use crate::browse::{ArchiveView, ViewAction};
use crate::dnd::DragItems;
use crate::fsview::{FolderView, FsAction, ViewMode};
use crate::icons::{self, Icon};
use crate::places::{self, Place};
use crate::search::{SearchAction, SearchView};
use crate::style::{self, ACCENT};
use eframe::egui::{self, Margin, RichText};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 탭 하나
pub struct Tab {
    pub folder: FolderView,
    /// 압축 파일 안을 보는 중이면 Some
    pub view: Option<ArchiveView>,
    /// 검색 결과를 보는 중이면 Some
    pub search: Option<SearchView>,
    /// 검색 칸 글자
    pub search_text: String,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// 주소 줄에 경로를 직접 입력하는 중
    pub editing_path: Option<String>,
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub cur: usize,
    /// 왼쪽 바로가기(즐겨찾기·드라이브) 목록을 보이는지
    pub show_places: bool,
    /// 다음 화면에서 검색 칸에 포커스 (Ctrl+F)
    pub focus_search: bool,
    /// 좁은 창에서 검색 칸을 펼쳤는지 (좁으면 돋보기 단추만 보임)
    search_open: bool,
}

impl Deref for Pane {
    type Target = Tab;
    fn deref(&self) -> &Tab {
        &self.tabs[self.cur]
    }
}

impl DerefMut for Pane {
    fn deref_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.cur]
    }
}

/// 왼쪽 바로가기 목록: 즐겨찾기와 드라이브
pub struct Places<'a> {
    pub favorites: &'a [Place],
    pub drives: &'a [Place],
}

/// 창에서 일어난 일 (Home이 처리)
pub struct PaneOut {
    pub fs: FsAction,
    pub view: ViewAction,
    pub search: SearchAction,
    pub close: bool,
    /// 이 창을 눌렀음 (활성 창으로)
    pub activated: bool,
    /// 이 창에 끌어다 놓음: (내용물, 놓을 폴더, Ctrl 누름 여부)
    pub dropped: Option<(Arc<DragItems>, PathBuf, bool)>,
    /// 압축 파일 안을 보는 탭에 파일을 끌어다 놓음: 넣을 파일들
    pub add_to_archive: Option<Vec<PathBuf>>,
    /// 이동할 곳 (주소 줄에서 입력하거나 누름)
    pub go: Option<PathBuf>,
    /// 즐겨찾기에 추가 / 빼기
    pub pin: Option<Vec<PathBuf>>,
    pub unpin: Option<PathBuf>,
    /// 탭을 바꾸거나 닫음 (제목 다시 쓰기)
    pub tab_changed: bool,
}

impl Tab {
    pub fn new(dir: PathBuf, show_hidden: bool) -> Self {
        Self {
            folder: FolderView::new(dir, show_hidden),
            view: None,
            search: None,
            search_text: String::new(),
            back: Vec::new(),
            forward: Vec::new(),
            editing_path: None,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.folder.dir
    }

    pub fn title(&self) -> String {
        if let Some(s) = &self.search {
            return format!("‘{}’ 검색", s.query);
        }
        match &self.view {
            Some(v) => v.title(),
            None => crate::file_name(&self.folder.dir),
        }
    }

    pub fn status(&self) -> String {
        if let Some(s) = &self.search {
            return s.status();
        }
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
        if dir == self.folder.dir && self.view.is_none() && self.search.is_none() {
            return;
        }
        if dir != self.folder.dir {
            self.back.push(self.folder.dir.clone());
            self.forward.clear();
        }
        self.go_to(dir, show_hidden);
    }

    fn go_to(&mut self, dir: PathBuf, show_hidden: bool) {
        let mode = self.folder.mode;
        self.view = None;
        self.search = None;
        self.search_text.clear();
        self.folder = FolderView::new(dir, show_hidden);
        self.folder.mode = mode;
        self.editing_path = None;
    }

    pub fn go_back(&mut self, show_hidden: bool) {
        if self.search.is_some() {
            self.search = None;
            return;
        }
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
        if self.search.is_some() {
            self.search = None;
            return;
        }
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
            && (parent != self.folder.dir || self.search.is_some())
        {
            self.navigate(parent.to_path_buf(), show_hidden);
        }
        self.folder.select_only(std::slice::from_ref(&path));
        self.view = Some(ArchiveView::open(&path, password));
    }

    /// 파일이 들어 있는 폴더로 가서 그 파일을 선택
    pub fn reveal(&mut self, path: &Path, show_hidden: bool) {
        if let Some(parent) = path.parent() {
            self.navigate(parent.to_path_buf(), show_hidden);
            self.folder.select_only(&[path.to_path_buf()]);
        }
    }

    /// 지금 고른 압축 파일들 (압축 파일 안을 보는 중이면 그 파일)
    pub fn selected_archives(&self) -> Vec<PathBuf> {
        if self.search.is_some() {
            return vec![];
        }
        if let Some(v) = &self.view {
            return if matches!(v.contents, crate::browse::Contents::Ok(_)) { vec![v.path.clone()] } else { vec![] };
        }
        self.folder.selection().into_iter().filter(|e| e.archive).map(|e| e.path.clone()).collect()
    }

    /// 지금 고른 실제 파일·폴더 (검색 결과에서도)
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        match (&self.search, &self.view) {
            (Some(s), _) => s.selected_paths(),
            (None, Some(_)) => vec![],
            (None, None) => self.folder.selected_paths(),
        }
    }
}

impl Pane {
    pub fn new(dir: PathBuf, show_hidden: bool) -> Self {
        Self { tabs: vec![Tab::new(dir, show_hidden)], cur: 0, show_places: true, focus_search: false, search_open: false }
    }

    pub fn new_tab(&mut self, dir: PathBuf, show_hidden: bool) {
        let mode = self.folder.mode;
        let mut t = Tab::new(dir, show_hidden);
        t.folder.mode = mode;
        self.cur += 1;
        self.tabs.insert(self.cur, t);
    }

    /// 탭 닫기. 마지막 탭이면 false (창을 닫아야 함)
    pub fn close_tab(&mut self, i: usize) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        self.tabs.remove(i);
        if self.cur > i || self.cur >= self.tabs.len() {
            self.cur = self.cur.saturating_sub(1);
        }
        true
    }

    pub fn reload(&mut self, show_hidden: bool) {
        for t in &mut self.tabs {
            t.reload(show_hidden);
        }
    }

    // ----- 화면 -----

    fn nav_button(ui: &mut egui::Ui, icon: Icon, enabled: bool, tip: &str) -> bool {
        Self::nav_button_on(ui, icon, enabled, false, tip)
    }

    fn nav_button_on(ui: &mut egui::Ui, icon: Icon, enabled: bool, on: bool, tip: &str) -> bool {
        let (r, resp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
        if on {
            ui.painter().rect_filled(r, 6.0, style::selection_fill(ui));
        } else if enabled && resp.hovered() {
            ui.painter().rect_filled(r, 6.0, style::hover_fill(ui));
        }
        let color = if enabled { ui.visuals().text_color() } else { style::weak(ui).gamma_multiply(0.5) };
        icons::paint(ui.painter(), r.shrink(7.0), icon, color);
        enabled && resp.on_hover_text(tip).clicked()
    }

    /// 탭 줄 (탭이 2개 이상일 때)
    fn tabs_ui(&mut self, ui: &mut egui::Ui, out: &mut PaneOut, show_hidden: bool) {
        let mut select = None;
        let mut close = None;
        let mut add = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let n = self.tabs.len();
            let avail = ui.available_width() - 32.0;
            let w = (avail / n as f32).clamp(70.0, 190.0);
            for (i, t) in self.tabs.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(w - 2.0, 26.0), egui::Sense::click());
                let on = i == self.cur;
                let fill = if on { ui.visuals().extreme_bg_color } else if resp.hovered() { style::hover_fill(ui) } else { style::bar_fill(ui) };
                ui.painter().rect_filled(rect, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, fill);
                if on {
                    ui.painter().hline(rect.x_range(), rect.top() + 0.5, egui::Stroke::new(2.0, ACCENT));
                }
                let x_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 12.0, rect.center().y), egui::vec2(16.0, 16.0));
                let text_clip = egui::Rect::from_min_max(rect.min + egui::vec2(8.0, 0.0), egui::pos2(x_rect.left() - 2.0, rect.bottom()));
                ui.painter().with_clip_rect(text_clip.intersect(ui.clip_rect())).text(
                    egui::pos2(rect.left() + 8.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    t.title(),
                    egui::FontId::proportional(12.5),
                    if on { ui.visuals().strong_text_color() } else { ui.visuals().text_color() },
                );
                let x_resp = ui.interact(x_rect, ui.scope_id().with(("tab_x", i)), egui::Sense::click());
                if x_resp.hovered() {
                    ui.painter().rect_filled(x_rect, 3.0, style::hover_fill(ui));
                }
                icons::paint(ui.painter(), x_rect.shrink(3.5), Icon::Close, style::weak(ui));
                if x_resp.clicked() || resp.middle_clicked() {
                    close = Some(i);
                } else if resp.clicked() {
                    select = Some(i);
                }
                resp.on_hover_text(t.dir().display().to_string());
            }
            if Self::nav_button(ui, Icon::Plus, true, "새 탭 (Ctrl+T)") {
                add = true;
            }
        });
        if let Some(i) = close {
            self.close_tab(i);
            out.tab_changed = true;
        } else if let Some(i) = select {
            self.cur = i;
            out.tab_changed = true;
        }
        if add {
            let d = self.dir().to_path_buf();
            self.new_tab(d, show_hidden);
            out.tab_changed = true;
        }
    }

    /// 왼쪽 바로가기 목록.
    fn places_ui(&self, ui: &mut egui::Ui, idx: usize, places: &Places, out: &mut PaneOut) -> Option<PathBuf> {
        let mut drop = None;
        let payload = egui::DragAndDrop::payload::<DragItems>(ui.ctx());
        let pointer = ui.ctx().pointer_latest_pos();
        let current = if self.view.is_none() && self.search.is_none() { Some(self.folder.dir.as_path()) } else { None };
        let mut pin_zone = None;
        egui::ScrollArea::vertical().id_salt(("places_scroll", idx)).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let section = |ui: &mut egui::Ui, title: &str| {
                ui.add_space(4.0);
                let r = ui.label(RichText::new(title).size(11.5).color(style::weak(ui)));
                ui.add_space(2.0);
                r.rect
            };
            let mut row = |ui: &mut egui::Ui, p: &Place| {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
                let hovered_drop = payload.is_some() && pointer.is_some_and(|q| rect.contains(q));
                if hovered_drop {
                    drop = Some(p.path.clone());
                    crate::table::drop_highlight(ui, rect);
                } else if current == Some(p.path.as_path()) {
                    ui.painter().rect_filled(rect, 6.0, style::selection_fill(ui));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, style::hover_fill(ui));
                }
                let ir = egui::Rect::from_center_size(rect.left_center() + egui::vec2(14.0, 0.0), egui::vec2(16.0, 16.0));
                icons::paint(ui.painter(), ir, if p.drive { Icon::Drive } else { Icon::Folder }, style::weak(ui));
                let clip = egui::Rect::from_x_y_ranges((rect.left() + 28.0)..=(rect.right() - 4.0), rect.y_range());
                ui.painter().with_clip_rect(clip.intersect(ui.clip_rect())).text(
                    rect.left_center() + egui::vec2(28.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &p.label,
                    egui::FontId::proportional(13.0),
                    ui.visuals().text_color(),
                );
                let resp = resp.on_hover_text(p.path.display().to_string());
                if resp.clicked() {
                    out.go = Some(p.path.clone());
                }
                resp.context_menu(|ui| {
                    if ui.button("열기").clicked() {
                        out.go = Some(p.path.clone());
                        ui.close();
                    }
                    if p.pinned && ui.button("즐겨찾기에서 빼기").clicked() {
                        out.unpin = Some(p.path.clone());
                        ui.close();
                    }
                });
            };
            let head = section(ui, "즐겨찾기");
            for p in places.favorites {
                row(ui, p);
            }
            // 폴더를 "즐겨찾기" 글자나 그 아래 빈 줄에 놓으면 즐겨찾기에 추가
            let (zone, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.0), egui::Sense::hover());
            pin_zone = Some((head, zone));
            ui.add_space(4.0);
            section(ui, "드라이브");
            for p in places.drives {
                row(ui, p);
            }
        });
        if let (Some(DragItems::Files { paths, .. }), Some((head, zone)), Some(q)) = (payload.as_deref(), pin_zone, pointer)
            && (head.expand2(egui::vec2(60.0, 3.0)).contains(q) || zone.expand2(egui::vec2(60.0, 4.0)).contains(q))
            && paths.iter().any(|p| p.is_dir())
        {
            let line = zone.shrink2(egui::vec2(4.0, 3.0));
            ui.painter().rect_filled(line, 2.0, ACCENT);
            drop = None;
            if ui.input(|i| i.pointer.any_released()) {
                out.pin = Some(paths.iter().filter(|p| p.is_dir()).cloned().collect());
                egui::DragAndDrop::clear_payload(ui.ctx());
            }
        }
        drop
    }

    /// 창 하나 그리기. `ui.max_rect()`가 이 창이 차지할 자리.
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
        places: &Places,
    ) -> PaneOut {
        let mut out = PaneOut {
            fs: FsAction::None,
            view: ViewAction::None,
            search: SearchAction::None,
            close: false,
            activated: false,
            dropped: None,
            add_to_archive: None,
            go: None,
            pin: None,
            unpin: None,
            tab_changed: false,
        };
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

        if self.tabs.len() > 1 {
            self.tabs_ui(ui, &mut out, show_hidden);
        }

        // --- 주소 줄 ---
        let (mut back, mut fwd, mut up) = (false, false, false);
        let mut start_search = None;
        let mut open_now = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if Self::nav_button_on(ui, Icon::Sidebar, true, self.show_places, if self.show_places { "바로가기 숨기기" } else { "바로가기 보이기" }) {
                self.show_places = !self.show_places;
            }
            let in_sub = self.view.is_some() || self.search.is_some();
            // 아주 좁은 창은 앞으로·위로·보기 단추를 숨김 (Alt+→, Alt+↑, Backspace는 그대로)
            let row_w = ui.available_width() + 30.0;
            let tiny = row_w < 340.0;
            back = Self::nav_button(ui, Icon::Back, !self.back.is_empty() || in_sub, "뒤로 (Alt+←)");
            if !tiny {
                fwd = Self::nav_button(ui, Icon::Forward, !self.forward.is_empty() && !in_sub, "앞으로 (Alt+→)");
                up = Self::nav_button(ui, Icon::Up, in_sub || self.folder.dir.parent().is_some(), "위로 (Alt+↑)");
            }
            ui.add_space(4.0);
            // 오른쪽: 검색 칸 · 보기 방식 · (나뉜 상태면) 닫기
            let avail = ui.available_width();
            // 좁은 창: 검색 칸 대신 돋보기 단추 (누르거나 Ctrl+F면 펼침)
            let compact = avail < 330.0;
            let show_box = !compact || self.search_open || self.focus_search || !self.search_text.is_empty() || self.search.is_some();
            let view_w = if tiny { 0.0 } else { 30.0 };
            let fixed = view_w + if split { 30.0 } else { 0.0 } + 8.0 + 16.0 + 4.0 + 16.0 + 40.0;
            let search_w: f32 = if avail > 420.0 { 150.0 } else if compact { 100.0 } else { 96.0 };
            // 아주 좁으면 주소 칸(최소 40)과 단추가 들어가도록 검색 칸을 줄임
            let search_w = search_w.min(avail - fixed).max(40.0);
            let box_w = if show_box { search_w + 12.0 + 4.0 } else { 30.0 };
            // 검색 칸 + 보기 단추 30 + 닫기 30 + 사이 간격
            let right_w = box_w + view_w + if split { 30.0 } else { 0.0 } + 8.0;
            // 주소 칸 테두리 안쪽 여백(좌우 16)도 빼야 넘치지 않음
            let field_w = (avail - right_w - 16.0 - 4.0).max(40.0);
            // 너비를 정해 두고 그 안에만 그림 (경로가 길어도 오른쪽 단추를 밀어내지 않음)
            let (frect, _) = ui.allocate_exact_size(egui::vec2(field_w + 16.0, 26.0), egui::Sense::hover());
            ui.painter().rect(frect, 6.0, ui.visuals().extreme_bg_color, egui::Stroke::new(1.0, style::border(ui)), egui::StrokeKind::Inside);
            let mut fui = ui.new_child(
                egui::UiBuilder::new().max_rect(frect.shrink2(egui::vec2(8.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            fui.set_clip_rect(frect.shrink(1.0).intersect(ui.clip_rect()));
            (|ui: &mut egui::Ui| {
                let dir = self.folder.dir.clone();
                let tab = &mut **self;
                if let Some(text) = tab.editing_path.as_mut() {
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).frame(egui::Frame::NONE));
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    if r.lost_focus() {
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            out.go = Some(PathBuf::from(text.trim()));
                        }
                        tab.editing_path = None;
                    }
                    return;
                }
                let resp = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r.shrink2(egui::vec2(0.0, 2.0)), if tab.search.is_some() { Icon::Search } else { Icon::Folder }, ACCENT);
                        ui.add_space(4.0);
                        let crumbs = places::crumbs(&dir);
                        // 뒤쪽부터 들어가는 만큼만 (실제 글꼴로 너비 계산)
                        let extra = if tab.view.is_some() || tab.search.is_some() { 160.0 } else { 0.0 };
                        let budget = (field_w - 50.0 - extra).max(60.0);
                        let mut used = 0.0;
                        let mut fit = 0;
                        let font = egui::TextStyle::Button.resolve(ui.style());
                        let sep = ui.painter().layout_no_wrap(" › ".into(), font.clone(), egui::Color32::WHITE).size().x;
                        for (label, _) in crumbs.iter().rev() {
                            used += ui.painter().layout_no_wrap(label.clone(), font.clone(), egui::Color32::WHITE).size().x + sep + 2.0;
                            if used > budget && fit > 0 {
                                break;
                            }
                            fit += 1;
                        }
                        let skip = crumbs.len() - fit;
                        if skip > 0 {
                            ui.weak("… › ");
                        }
                        let tail = tab.view.is_some() || tab.search.is_some();
                        let last = crumbs.len().saturating_sub(1);
                        for (i, (label, path)) in crumbs.into_iter().enumerate().skip(skip) {
                            let is_last = i == last && !tail;
                            let text = if is_last { RichText::new(label).strong() } else { RichText::new(label) };
                            if ui.add(egui::Button::new(text).frame(false)).clicked() {
                                out.go = Some(path);
                            }
                            if i != last || tail {
                                ui.weak(" › ");
                            }
                        }
                        if let Some(s) = &tab.search {
                            ui.strong(format!("‘{}’ 검색 결과", s.query));
                        } else if let Some(v) = tab.view.as_mut() {
                            v.crumbs_ui(ui);
                        }
                        // 나머지 빈 곳을 누르면 경로 입력
                        ui.allocate_response(egui::vec2(ui.available_width(), 22.0), egui::Sense::click())
                    })
                    .inner;
                if resp.on_hover_cursor(egui::CursorIcon::Text).clicked() {
                    tab.editing_path = Some(dir.display().to_string());
                }
            })(&mut fui);
            // 검색 칸
            let sframe = egui::Frame::NONE
                .fill(ui.visuals().extreme_bg_color)
                .stroke(egui::Stroke::new(1.0, style::border(ui)))
                .corner_radius(6)
                .inner_margin(Margin { left: 6, right: 6, top: 2, bottom: 2 });
            if !show_box {
                if Self::nav_button(ui, Icon::Search, true, "검색 (Ctrl+F)") {
                    self.search_open = true;
                    self.focus_search = true;
                }
            } else {
            ui.add_space(4.0);
            sframe.show(ui, |ui| {
                ui.set_width(search_w);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 22.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r.shrink2(egui::vec2(0.0, 3.0)), Icon::Search, style::weak(ui));
                    let focus = std::mem::take(&mut self.focus_search);
                    let tab = &mut **self;
                    let te = ui.add(
                        egui::TextEdit::singleline(&mut tab.search_text)
                            .hint_text("검색 (Ctrl+F)")
                            .desired_width(search_w - 22.0)
                            .frame(egui::Frame::NONE),
                    );
                    if focus {
                        te.request_focus();
                        open_now = true;
                    }
                    if te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        start_search = Some(tab.search_text.trim().to_string());
                    }
                    if te.has_focus() {
                        open_now = true;
                    }
                });
            });
            }
            // 보기 방식
            let icons_mode = self.folder.mode == ViewMode::Icons;
            let (ic, tip) = if icons_mode { (Icon::List, "목록으로 보기") } else { (Icon::Grid, "아이콘으로 보기 (사진은 미리보기)") };
            if !tiny && Self::nav_button(ui, ic, self.view.is_none() && self.search.is_none(), tip) {
                self.folder.mode = if icons_mode { ViewMode::List } else { ViewMode::Icons };
            }
            if split && Self::nav_button(ui, Icon::Close, true, "이 창 닫기") {
                out.close = true;
            }
        });
        ui.add_space(2.0);
        // 좁은 창에서 펼친 검색 칸은 비어 있고 포커스가 없으면 다시 접음
        if open_now {
            self.search_open = true;
        } else if self.search_open && !self.focus_search && self.search_text.is_empty() {
            self.search_open = false;
        }
        if let Some(q) = start_search {
            if q.is_empty() {
                self.search = None;
            } else {
                let root = self.folder.dir.clone();
                self.view = None;
                self.search = Some(SearchView::start(ui.ctx(), root, q, show_hidden));
            }
            out.tab_changed = true;
        }
        if back {
            self.go_back(show_hidden);
        } else if fwd {
            self.go_forward(show_hidden);
        } else if up {
            self.go_up(show_hidden);
        }

        // --- 바로가기 + 표 ---
        // 주소 줄이 넓어져도 표는 창 안에서만
        let body = egui::Rect::from_min_max(egui::pos2(inner.left(), ui.cursor().top()), inner.max);
        let mut place_drop = None;
        let table_rect = if self.show_places {
            // 창이 좁으면 바로가기 목록도 좁게
            let w = if body.width() >= 640.0 { 160.0 } else { 118.0 };
            let side = egui::Rect::from_min_max(body.min, egui::pos2(body.left() + w, body.bottom()));
            let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(side).id_salt(("places", idx)));
            sui.set_clip_rect(side.intersect(inner));
            place_drop = self.places_ui(&mut sui, idx, places, &mut out);
            ui.painter().vline(side.right() + 3.0, side.y_range(), egui::Stroke::new(1.0, style::border(ui)));
            egui::Rect::from_min_max(egui::pos2(side.right() + 7.0, body.top()), body.max)
        } else {
            body
        };
        let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(table_rect).layout(egui::Layout::top_down(egui::Align::Min)).id_salt(("tab", idx, self.cur)));
        tui.set_clip_rect(table_rect.intersect(inner));
        let tab = &mut **self;
        if let Some(s) = tab.search.as_mut() {
            out.search = s.table_ui(&mut tui, idx, active);
        } else if let Some(view) = tab.view.as_mut() {
            out.view = view.table_ui(&mut tui, password, idx, active);
        } else {
            out.fs = tab.folder.table_ui(&mut tui, idx, active, can_paste);
        }
        ui.advance_cursor_after_rect(body);

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
            let on_place = place_drop.is_some();
            let in_archive = self.view.as_ref().is_some_and(|v| v.editable()) && self.search.is_none();
            let in_folder = self.view.is_none() && self.search.is_none();
            let target = place_drop.or_else(|| if in_folder { self.folder.drop_folder.clone() } else { None });
            let files = matches!(&*payload, DragItems::Files { .. });
            // 같은 창의 빈 곳에 놓는 것은 아무 일도 아님 (폴더 줄·바로가기에 놓을 때만 의미)
            let meaningful = on_place || (in_folder && (!from_here || target.is_some())) || (in_archive && files && !on_place);
            if meaningful && target.is_none() {
                crate::table::drop_highlight(ui, table_rect.intersect(inner));
            }
            if meaningful && ui.input(|i| i.pointer.any_released()) {
                let ctrl = ui.input(|i| i.modifiers.command);
                if let Some(p) = egui::DragAndDrop::take_payload::<DragItems>(ui.ctx()) {
                    if in_archive && !on_place {
                        if let DragItems::Files { paths, .. } = &*p {
                            out.add_to_archive = Some(paths.clone());
                        }
                    } else {
                        out.dropped = Some((p, target.unwrap_or_else(|| self.folder.dir.clone()), ctrl));
                    }
                }
            }
        }
        out
    }
}
