//! 메인 창 — 끌어다 놓고 버튼 하나로 압축 풀기 / 압축하기.

use crate::style::{self, ERR, OK, WARN};
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use unpack::{
    ArchiveKind, EntryInfo, Mode, Options, compress, default_archive_path, detect, extract_auto,
    is_password_error, list,
};

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

/// 하단에 한 줄로 보여 주는 결과
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
    viewer: Option<(String, Result<Vec<EntryInfo>, String>)>,
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
            viewer: None,
        };
        app.add(files);
        if auto {
            app.extract(ctx);
        } else if let [one] = app.items.as_slice()
            && one.archive
        {
            // 압축 파일을 더블클릭해서 열면 내용을 바로 보여 줌
            let entries = list(&one.path, &app.options()).map_err(|e| format!("{e:#}"));
            app.viewer = Some((crate::file_name(&one.path), entries));
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
            let archive = path.is_file() && detect(&path).is_ok();
            self.items.push(Item { path, archive, status: Status::Idle });
        }
        self.notice = None;
    }

    fn options(&self) -> Options {
        Options { password: (!self.password.is_empty()).then(|| self.password.clone()), ..Default::default() }
    }

    fn extract(&mut self, ctx: &egui::Context) {
        let jobs: Vec<(usize, PathBuf)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.archive && !matches!(it.status, Status::Done(_)))
            .map(|(i, it)| (i, it.path.clone()))
            .collect();
        if jobs.is_empty() {
            return;
        }
        let opts = self.options();
        let out_dir = self.out_dir.clone();
        self.run(ctx, move |tx| {
            for (i, path) in jobs {
                tx.send(Msg::Started(i));
                let parent = out_dir.clone().unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_default());
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
                // 지정 폴더에서도 이름이 겹치지 않게
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
                Ok(p) => Notice {
                    ok: true,
                    text: format!("{} 만들었습니다", crate::file_name(&p)),
                    open: Some(p),
                },
                Err(e) => Notice { ok: false, text: e, open: None },
            });
        }
        if done {
            self.worker = None;
            if self.items.iter().any(|i| i.status == Status::NeedPassword) {
                self.show_settings = true;
                self.notice = Some(Notice { ok: false, text: "암호가 필요한 파일이 있습니다. 암호를 입력하고 다시 누르세요.".into(), open: None });
            }
        }
    }

    fn pick_files(&mut self) {
        if let Some(files) = rfd::FileDialog::new().set_title("파일 선택").pick_files() {
            self.add(files);
        }
    }

    fn drop_zone(&mut self, ui: &mut egui::Ui, hovering: bool) {
        let stroke = if hovering {
            egui::Stroke::new(2.0, style::ACCENT)
        } else {
            egui::Stroke::new(1.5, ui.visuals().widgets.noninteractive.bg_stroke.color)
        };
        let size = ui.available_size();
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
        ui.painter().rect_stroke(rect, 12.0, stroke, egui::StrokeKind::Inside);
        let c = rect.center();
        let painter = ui.painter();
        let text = ui.visuals().text_color();
        let weak = ui.visuals().weak_text_color();
        painter.text(c - egui::vec2(0.0, 22.0), egui::Align2::CENTER_CENTER, "여기로 끌어다 놓으세요", egui::FontId::proportional(20.0), text);
        painter.text(c + egui::vec2(0.0, 8.0), egui::Align2::CENTER_CENTER, "압축 파일은 풀고, 파일·폴더는 압축합니다", egui::FontId::proportional(14.0), weak);
        painter.text(c + egui::vec2(0.0, 40.0), egui::Align2::CENTER_CENTER, "또는 클릭해서 파일 선택", egui::FontId::proportional(14.0), style::ACCENT);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.pick_files();
        }
    }

    fn list_ui(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        let mut view = None;
        let height = ui.available_height() - 44.0 - if self.show_settings { 172.0 } else { 0.0 };
        egui::ScrollArea::vertical().max_height(height.max(80.0)).auto_shrink([false, false]).show(ui, |ui| {
            for (i, it) in self.items.iter().enumerate() {
                let row = ui.horizontal(|ui| {
                    let name = crate::file_name(&it.path);
                    let label = if it.path.is_dir() { format!("{name}/") } else { name };
                    ui.label(RichText::new(label).strong()).on_hover_text(it.path.display().to_string());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !self.busy() && ui.add(egui::Button::new("✕").frame(false)).on_hover_text("목록에서 빼기").clicked() {
                            remove = Some(i);
                        }
                        match &it.status {
                            Status::Idle => {
                                let kind = if it.archive {
                                    "압축 파일".to_string()
                                } else if it.path.is_dir() {
                                    "폴더".to_string()
                                } else {
                                    std::fs::metadata(&it.path).map(|m| crate::human_size(m.len())).unwrap_or_default()
                                };
                                ui.weak(kind);
                            }
                            Status::Working => {
                                ui.spinner();
                            }
                            Status::Done(out) => {
                                if ui.link("열기").clicked() {
                                    crate::open_in_explorer(out);
                                }
                                ui.colored_label(OK, "완료");
                            }
                            Status::NeedPassword => {
                                ui.colored_label(WARN, "암호 필요");
                            }
                            Status::Failed(e) => {
                                ui.colored_label(ERR, "실패").on_hover_text(e);
                            }
                        }
                    });
                });
                if it.archive {
                    row.response.context_menu(|ui| {
                        if ui.button("내용 보기").clicked() {
                            view = Some(i);
                            ui.close();
                        }
                    });
                }
                ui.separator();
            }
        });
        if let Some(i) = view {
            let it = &self.items[i];
            self.viewer = Some((crate::file_name(&it.path), list(&it.path, &self.options()).map_err(|e| format!("{e:#}"))));
        }
        if let Some(i) = remove {
            self.items.remove(i);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.add_enabled_ui(!self.busy(), |ui| {
                if ui.link("+ 추가").clicked() {
                    self.pick_files();
                }
                if ui.link("모두 지우기").clicked() {
                    self.items.clear();
                    self.notice = None;
                }
            });
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("settings").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                ui.weak("저장 위치");
                ui.horizontal(|ui| {
                    let label = match &self.out_dir {
                        None => "원래 위치".to_string(),
                        Some(d) => d.display().to_string(),
                    };
                    ui.label(label);
                    if ui.link("변경").clicked()
                        && let Some(d) = rfd::FileDialog::new().set_title("저장 위치").pick_folder()
                    {
                        self.out_dir = Some(d);
                    }
                    if self.out_dir.is_some() && ui.link("원래 위치로").clicked() {
                        self.out_dir = None;
                    }
                });
                ui.end_row();

                ui.weak("압축 형식");
                ui.horizontal(|ui| {
                    for k in ArchiveKind::ALL {
                        ui.selectable_value(&mut self.kind, k, k.label());
                    }
                });
                ui.end_row();

                ui.weak("암호");
                ui.add(
                    egui::TextEdit::singleline(&mut self.password)
                        .password(true)
                        .hint_text("없으면 비워 두세요")
                        .desired_width(220.0),
                );
                ui.end_row();
            });
        });
    }
}

impl eframe::App for Home {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll();

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() && !self.busy() {
            self.add(dropped);
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        // 하단 막대: 결과 한 줄 + 설정 + 버튼 두 개
        let bar_frame = style::panel(&ctx).inner_margin(egui::Margin { left: 20, right: 20, top: 8, bottom: 16 });
        egui::Panel::bottom("bar").frame(bar_frame).show_separator_line(false).show(ui, |ui| {
            if let Some(n) = &self.notice {
                let open = n.open.clone();
                ui.horizontal(|ui| {
                    ui.colored_label(if n.ok { OK } else { WARN }, &n.text);
                    if let Some(p) = open
                        && ui.link("폴더에서 보기").clicked()
                    {
                        crate::open_in_explorer(&p);
                    }
                });
            }
            ui.horizontal(|ui| {
                let settings = if self.show_settings { "설정 닫기" } else { "설정" };
                if ui.add(egui::Button::new(settings).frame(false)).clicked() {
                    self.show_settings = !self.show_settings;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let any = !self.items.is_empty() && !self.busy();
                    let archives = self.items.iter().any(|i| i.archive && !matches!(i.status, Status::Done(_)));
                    if ui
                        .add_enabled(any, style::secondary(&format!("압축하기 ({})", self.kind.label())))
                        .clicked()
                    {
                        self.pack(&ctx);
                    }
                    if ui.add_enabled(any && archives, style::primary("압축 풀기")).clicked() {
                        self.extract(&ctx);
                    }
                    if self.busy() {
                        ui.spinner();
                    }
                });
            });
        });

        let main_frame = style::panel(&ctx).inner_margin(egui::Margin { left: 20, right: 20, top: 20, bottom: 12 });
        egui::CentralPanel::default().frame(main_frame).show(ui, |ui| {
            if self.items.is_empty() && !self.show_settings {
                self.drop_zone(ui, hovering);
            } else if !self.items.is_empty() {
                self.list_ui(ui);
            }
            if self.show_settings {
                self.settings_ui(ui);
            }
        });

        let mut close = false;
        if let Some((name, entries)) = &self.viewer {
            let mut open = true;
            egui::Window::new(name.as_str())
                .open(&mut open)
                .collapsible(false)
                .default_size([420.0, 320.0])
                .show(&ctx, |ui| match entries {
                    Err(e) => {
                        ui.colored_label(ERR, e);
                    }
                    Ok(list) => {
                        ui.weak(format!("{}개 항목", list.len()));
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for e in list {
                                ui.horizontal(|ui| {
                                    ui.label(&e.name);
                                    if !e.is_dir {
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.weak(crate::human_size(e.size));
                                        });
                                    }
                                });
                            }
                        });
                    }
                });
            close = !open;
        }
        if close {
            self.viewer = None;
        }
    }
}
