//! 메인 창 — 도구 모음 / (주소 줄) / 표 / 상태 표시줄, 오른쪽에 설정 패널.

use crate::browse::{ArchiveView, Contents, kind_of};
use crate::icons::Icon;
use crate::style::{self, ACCENT, ERR, OK, WARN};
use crate::table::{self, Col};
use eframe::egui::{self, Margin, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use unpack::{ArchiveKind, Mode, Options, compress, default_archive_path, detect, extract_auto, is_password_error};

#[derive(PartialEq)]
enum Status {
    Idle,
    Working,
    Done(PathBuf),
    NeedPassword,
    Failed(String),
}

struct Item {
    path: PathBuf,
    archive: bool,
    is_dir: bool,
    size: u64,
    status: Status,
}

enum Msg {
    Started(usize),
    Finished(usize, Status),
    Packed(Result<PathBuf, String>),
    AllDone,
}

/// 메시지를 보내고 화면을 다시 그리게 함
struct Tx {
    tx: mpsc::Sender<Msg>,
    ctx: egui::Context,
}

impl Tx {
    fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }
}

/// 상태 표시줄에 보여 주는 결과
struct Notice {
    ok: bool,
    text: String,
    open: Option<PathBuf>,
}

pub struct Home {
    items: Vec<Item>,
    worker: Option<Receiver<Msg>>,
    notice: Option<Notice>,
    show_settings: bool,
    /// `None`이면 원래 위치
    out_dir: Option<PathBuf>,
    kind: ArchiveKind,
    password: String,
    /// 압축 파일 내용 보기 (있으면 표 대신 표시)
    view: Option<ArchiveView>,
    selected: Option<usize>,
    sort: table::Sort,
}

impl Home {
    pub fn new(ctx: &egui::Context, files: Vec<PathBuf>, auto: bool) -> Self {
        let mut app = Self {
            items: Vec::new(),
            worker: None,
            notice: None,
            show_settings: false,
            out_dir: None,
            kind: ArchiveKind::Zip,
            password: String::new(),
            view: None,
            selected: None,
            sort: (0, true),
        };
        app.add(files);
        if auto {
            app.extract(ctx, None);
        } else if let [one] = app.items.as_slice()
            && one.archive
        {
            // 압축 파일을 더블클릭해서 열면 내용을 바로 보여 줌
            app.open_view(ctx, 0);
        }
        app
    }

    fn busy(&self) -> bool {
        self.worker.is_some()
    }

    fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if self.items.iter().any(|i| i.path == path) || !path.exists() {
                continue;
            }
            let is_dir = path.is_dir();
            let archive = !is_dir && detect(&path).is_ok();
            let size = if is_dir { 0 } else { std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) };
            self.items.push(Item { path, archive, is_dir, size, status: Status::Idle });
        }
        self.notice = None;
    }

    fn options(&self) -> Options {
        Options { password: (!self.password.is_empty()).then(|| self.password.clone()), ..Default::default() }
    }

    fn open_view(&mut self, ctx: &egui::Context, idx: usize) {
        let v = ArchiveView::open(idx, &self.items[idx].path, self.options().password);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{} - {}", v.title(), unpack::APP_NAME)));
        self.view = Some(v);
    }

    fn close_view(&mut self, ctx: &egui::Context) {
        self.view = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(unpack::APP_NAME.into()));
    }

    /// 풀 대상: 내용 보기 중이면 그 파일, 아니면 목록의 (아직 안 푼) 압축 파일 전부
    fn extract_targets(&self) -> Vec<usize> {
        if let Some(v) = &self.view {
            return if matches!(v.contents, Contents::Ok(_)) { vec![v.idx] } else { vec![] };
        }
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.archive && !matches!(it.status, Status::Done(_)))
            .map(|(i, _)| i)
            .collect()
    }

    /// 알아서 풀기. `to`가 없으면 설정의 저장 위치 또는 각 압축 파일 위치.
    fn extract(&mut self, ctx: &egui::Context, to: Option<PathBuf>) {
        let ids = self.extract_targets();
        let parent = to.or_else(|| self.out_dir.clone());
        let jobs: Vec<(usize, PathBuf)> = ids.into_iter().map(|i| (i, self.items[i].path.clone())).collect();
        if jobs.is_empty() {
            return;
        }
        let opts = self.options();
        self.run(ctx, move |tx| {
            for (i, path) in jobs {
                tx.send(Msg::Started(i));
                let parent = parent.clone().unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_default());
                let status = match extract_auto(&path, &parent, Mode::Smart, &opts) {
                    Ok(s) => Status::Done(s.output),
                    Err(e) if is_password_error(&e) => Status::NeedPassword,
                    Err(e) => Status::Failed(format!("{e:#}")),
                };
                tx.send(Msg::Finished(i, status));
            }
        });
    }

    fn pack(&mut self, ctx: &egui::Context) {
        let inputs: Vec<PathBuf> = self.items.iter().map(|i| i.path.clone()).collect();
        if inputs.is_empty() {
            return;
        }
        let kind = self.kind;
        let mut out = default_archive_path(&inputs, kind);
        if let Some(dir) = &self.out_dir {
            let name = out.file_name().unwrap_or_default().to_owned();
            out = dir.join(name);
            if out.exists() {
                let stem = out.file_name().unwrap_or_default().to_string_lossy().trim_end_matches(kind.ext()).to_string();
                out = (2..).map(|n| dir.join(format!("{stem} ({n}){}", kind.ext()))).find(|p| !p.exists()).unwrap();
            }
        }
        let pw = (kind.supports_password() && !self.password.is_empty()).then(|| self.password.clone());
        self.run(ctx, move |tx| {
            let r = compress(&inputs, &out, kind, pw.as_deref()).map(|s| s.output).map_err(|e| format!("{e:#}"));
            tx.send(Msg::Packed(r));
        });
    }

    fn run(&mut self, ctx: &egui::Context, job: impl FnOnce(&Tx) + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        let tx = Tx { tx, ctx: ctx.clone() };
        thread::spawn(move || {
            job(&tx);
            tx.send(Msg::AllDone);
        });
        self.notice = None;
        self.worker = Some(rx);
    }

    fn poll(&mut self) {
        let Some(rx) = &self.worker else { return };
        let mut done = false;
        let mut packed = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Started(i) => self.items[i].status = Status::Working,
                Msg::Finished(i, s) => self.items[i].status = s,
                Msg::Packed(r) => packed = Some(r),
                Msg::AllDone => done = true,
            }
        }
        if let Some(r) = packed {
            self.notice = Some(match r {
                Ok(p) => Notice { ok: true, text: format!("{} 을(를) 만들었습니다", crate::file_name(&p)), open: Some(p) },
                Err(e) => Notice { ok: false, text: e, open: None },
            });
        }
        if done {
            self.worker = None;
            let finished: Vec<PathBuf> =
                self.items.iter().filter_map(|i| if let Status::Done(p) = &i.status { Some(p.clone()) } else { None }).collect();
            if self.items.iter().any(|i| i.status == Status::NeedPassword) {
                self.show_settings = true;
                self.notice = Some(Notice {
                    ok: false,
                    text: "암호가 필요합니다. 오른쪽 설정에 암호를 넣고 다시 압축 풀기를 누르세요.".into(),
                    open: None,
                });
            } else if let Some(it) = self.items.iter().find(|i| matches!(i.status, Status::Failed(_))) {
                if let Status::Failed(e) = &it.status {
                    self.notice = Some(Notice { ok: false, text: format!("{}: {e}", crate::file_name(&it.path)), open: None });
                }
            } else if self.notice.is_none()
                && let Some(last) = finished.last()
            {
                self.notice =
                    Some(Notice { ok: true, text: format!("압축을 풀었습니다 → {}", crate::file_name(last)), open: Some(last.clone()) });
            }
        }
    }

    fn pick_files(&mut self, ctx: &egui::Context) {
        if let Some(files) = rfd::FileDialog::new().set_title("파일 열기").pick_files() {
            let first_new = self.items.len();
            self.add(files);
            // 압축 파일 하나만 열었으면 내용을 바로 보여 줌
            if self.items.len() == first_new + 1 && self.items[first_new].archive {
                self.open_view(ctx, first_new);
            } else if self.view.is_some() {
                self.close_view(ctx);
            }
        }
    }

    // ----- 화면 -----

    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let frame = style::bar(ui, Margin { left: 10, right: 10, top: 6, bottom: 6 });
        egui::Panel::top("toolbar").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let idle = !self.busy();
                if style::tool_button(ui, Icon::Open, "열기", idle).on_hover_text("압축 파일이나 파일·폴더 추가").clicked() {
                    self.pick_files(ctx);
                }
                style::vsep(ui);
                let can_extract = idle && !self.extract_targets().is_empty();
                if style::tool_button(ui, Icon::Extract, "압축 풀기", can_extract)
                    .on_hover_text("알아서 풀기: 폴더가 하나면 그대로, 아니면 파일 이름 폴더로")
                    .clicked()
                {
                    self.extract(ctx, None);
                }
                if style::tool_button(ui, Icon::ExtractTo, "다른 위치에", can_extract).on_hover_text("풀 위치를 골라서 풀기").clicked()
                    && let Some(dir) = rfd::FileDialog::new().set_title("풀 위치").pick_folder()
                {
                    self.extract(ctx, Some(dir));
                }
                let can_pack = idle && self.view.is_none() && !self.items.is_empty();
                if style::tool_button(ui, Icon::Compress, "압축하기", can_pack)
                    .on_hover_text(format!("목록 전체를 {} 하나로 압축", self.kind.label()))
                    .clicked()
                {
                    self.pack(ctx);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::tool_button(ui, Icon::Settings, "설정", true).clicked() {
                        self.show_settings = !self.show_settings;
                    }
                });
            });
        });
    }

    fn address_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let frame = egui::Frame::NONE
            .fill(ui.visuals().panel_fill)
            .inner_margin(Margin { left: 10, right: 10, top: 8, bottom: 6 });
        let mut back = false;
        egui::Panel::top("address").frame(frame).show_separator_line(false).show(ui, |ui| {
            let field = egui::Frame::NONE
                .fill(ui.visuals().extreme_bg_color)
                .stroke(egui::Stroke::new(1.0, style::border(ui)))
                .corner_radius(6)
                .inner_margin(Margin { left: 4, right: 10, top: 2, bottom: 2 });
            field.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, 6.0, style::hover_fill(ui));
                }
                crate::icons::paint(ui.painter(), r.shrink(6.0), Icon::Back, ui.visuals().text_color());
                if resp.on_hover_text("상위 폴더 / 목록으로").clicked() {
                    back = true;
                }
                if let Some(v) = self.view.as_mut() {
                    v.crumbs_ui(ui);
                }
            });
            });
        });
        if back {
            match self.view.as_mut() {
                Some(v) if !v.at_root() => v.go_up(),
                _ => self.close_view(ctx),
            }
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        let frame = style::bar(ui, Margin::same(16));
        egui::Panel::right("settings").frame(frame).resizable(false).exact_size(250.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("설정");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("✕").frame(false)).clicked() {
                        self.show_settings = false;
                    }
                });
            });
            ui.add_space(10.0);

            ui.weak("저장 위치");
            let here = self.out_dir.is_none();
            if ui.radio(here, "원래 위치 (압축 파일 옆)").clicked() {
                self.out_dir = None;
            }
            let label = self.out_dir.as_ref().map(|d| crate::file_name(d)).unwrap_or_else(|| "다른 폴더…".into());
            let hover = self.out_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "폴더 고르기".into());
            if ui.radio(!here, label).on_hover_text(hover).clicked()
                && let Some(d) = rfd::FileDialog::new().set_title("저장 위치").pick_folder()
            {
                self.out_dir = Some(d);
            }
            ui.add_space(14.0);

            ui.weak("압축 형식");
            ui.horizontal_wrapped(|ui| {
                for k in ArchiveKind::ALL {
                    ui.selectable_value(&mut self.kind, k, k.label());
                }
            });
            ui.add_space(14.0);

            ui.weak("암호 (풀기·압축 공통)");
            ui.add(
                egui::TextEdit::singleline(&mut self.password)
                    .password(true)
                    .hint_text("없으면 비워 두세요")
                    .desired_width(f32::INFINITY),
            );
            if !self.kind.supports_password() && !self.password.is_empty() {
                ui.colored_label(WARN, format!("{}는 암호를 지원하지 않습니다", self.kind.label()));
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let frame = style::bar(ui, Margin { left: 14, right: 14, top: 6, bottom: 6 });
        egui::Panel::bottom("status").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let info = match &self.view {
                    Some(v) => v.status(),
                    None if self.items.is_empty() => format!("{} — 압축 풀기 · 압축하기", unpack::APP_NAME),
                    None => {
                        let archives = self.items.iter().filter(|i| i.archive).count();
                        format!("항목 {}개 · 압축 파일 {}개", self.items.len(), archives)
                    }
                };
                ui.label(RichText::new(info).size(12.5).color(style::weak(ui)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.busy() {
                        ui.label(RichText::new("작업 중…").size(12.5).color(ACCENT));
                        ui.spinner();
                    } else if let Some(n) = &self.notice {
                        if let Some(p) = n.open.clone()
                            && ui.link(RichText::new("폴더 열기").size(12.5)).clicked()
                        {
                            crate::open_in_explorer(&p);
                        }
                        ui.label(RichText::new(&n.text).size(12.5).color(if n.ok { OK } else { WARN }));
                    }
                });
            });
        });
    }

    fn empty_state(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, hovering: bool) {
        let rect = ui.available_rect_before_wrap().shrink(24.0);
        let resp = ui.interact(rect, ui.scope_id().with("drop"), egui::Sense::click());
        let stroke_color = if hovering || resp.hovered() { ACCENT } else { style::border(ui) };
        let p = ui.painter();
        if hovering {
            p.rect_filled(rect, 14.0, style::selection_fill(ui));
        }
        // 점선 테두리
        let pts = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
        for w in pts.windows(2) {
            p.extend(egui::Shape::dashed_line(&[w[0], w[1]], egui::Stroke::new(1.5, stroke_color), 8.0, 6.0));
        }
        table::empty_state(
            ui,
            rect,
            Icon::Archive,
            "압축 파일이나 폴더를 여기로 끌어다 놓으세요",
            "압축 파일은 내용을 보여 주고, 파일·폴더는 압축할 수 있습니다",
        );
        let btn = egui::Rect::from_center_size(rect.center() + egui::vec2(0.0, 84.0), egui::vec2(130.0, 34.0));
        let clicked = ui.put(btn, style::primary("파일 열기")).clicked();
        if clicked || resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.pick_files(ctx);
        }
    }

    fn list_table(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let cols = [
            Col { title: "이름", width: 0.0, right: false },
            Col { title: "종류", width: 130.0, right: false },
            Col { title: "크기", width: 100.0, right: true },
            Col { title: "상태", width: 100.0, right: false },
        ];
        table::header(ui, &cols, &mut self.sort);
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        let (col, asc) = self.sort;
        order.sort_by(|&a, &b| {
            let (a, b) = (&self.items[a], &self.items[b]);
            let ord = match col {
                1 => kind(a).cmp(&kind(b)),
                2 => a.size.cmp(&b.size),
                _ => crate::file_name(&a.path).to_lowercase().cmp(&crate::file_name(&b.path).to_lowercase()),
            };
            if asc { ord } else { ord.reverse() }
        });

        let mut open = None;
        let mut remove = None;
        let busy = self.busy();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (row_no, &i) in order.iter().enumerate() {
                let it = &self.items[i];
                let icon = if it.is_dir {
                    Icon::Folder
                } else if it.archive {
                    Icon::Archive
                } else {
                    Icon::File
                };
                let (st, color) = match &it.status {
                    Status::Idle => (String::new(), None),
                    Status::Working => ("푸는 중…".into(), Some(ACCENT)),
                    Status::Done(_) => ("완료".into(), Some(OK)),
                    Status::NeedPassword => ("암호 필요".into(), Some(WARN)),
                    Status::Failed(_) => ("실패".into(), Some(ERR)),
                };
                let size = if it.is_dir { String::new() } else { crate::human_size(it.size) };
                let cells = [(crate::file_name(&it.path), None), (kind(it), None), (size, None), (st, color)];
                let r = table::row(ui, &cols, icon, &cells, self.selected == Some(i), row_no);
                let r = match &it.status {
                    Status::Failed(e) => r.on_hover_text(e),
                    _ => r.on_hover_text(it.path.display().to_string()),
                };
                if r.clicked() {
                    self.selected = Some(i);
                }
                if r.double_clicked() {
                    match &it.status {
                        Status::Done(out) => crate::open_in_explorer(out),
                        _ if it.archive => open = Some(i),
                        _ => {}
                    }
                }
                r.context_menu(|ui| {
                    if it.archive && ui.button("내용 보기").clicked() {
                        open = Some(i);
                        ui.close();
                    }
                    if let Status::Done(out) = &it.status
                        && ui.button("결과 폴더 열기").clicked()
                    {
                        crate::open_in_explorer(out);
                        ui.close();
                    }
                    if !busy && ui.button("목록에서 빼기").clicked() {
                        remove = Some(i);
                        ui.close();
                    }
                });
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Delete)) && !busy {
            remove = remove.or(self.selected);
        }
        if let Some(i) = open {
            self.open_view(ctx, i);
        }
        if let Some(i) = remove {
            self.items.remove(i);
            self.selected = None;
        }
    }
}

fn kind(it: &Item) -> String {
    if it.is_dir {
        "폴더".into()
    } else if it.archive {
        detect(&it.path).map(|f| format!("{} 압축 파일", f.to_string().to_uppercase())).unwrap_or_default()
    } else {
        kind_of(&crate::file_name(&it.path))
    }
}

impl eframe::App for Home {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll();

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() && !self.busy() {
            self.add(dropped);
            if self.view.is_some() {
                self.close_view(&ctx);
            }
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        self.toolbar(ui, &ctx);
        self.status_bar(ui);
        if self.show_settings {
            self.settings_panel(ui);
        }
        if self.view.is_some() {
            self.address_bar(ui, &ctx);
        }

        let frame = egui::Frame::central_panel(&ctx.global_style()).inner_margin(Margin { left: 8, right: 8, top: 4, bottom: 4 });
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if let Some(view) = self.view.as_mut() {
                if view.table_ui(ui, &mut self.password) {
                    let pw = Some(self.password.clone());
                    view.reload(pw);
                }
            } else if self.items.is_empty() {
                self.empty_state(ui, &ctx, hovering);
            } else {
                self.list_table(ui, &ctx);
            }
        });
    }
}
