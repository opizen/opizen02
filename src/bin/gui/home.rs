//! 메인 창 — 파일 탐색기.
//!
//! 왼쪽 사이드바(즐겨찾기·드라이브) / 위 도구 모음 + 주소 줄 / 가운데 폴더 표 / 아래 상태 표시줄.
//! 압축 파일을 더블클릭하면 그 안으로 들어가고(내용 보기), 압축 풀기·압축하기·검사는
//! 고른 항목을 대상으로 하는 하위 기능입니다.

use crate::browse::{ArchiveView, Contents, ViewAction};
use crate::fsview::{FolderView, FsAction, FsCommand};
use crate::icons::{self, Icon};
use crate::places::{self, Place};
use crate::settings::Settings;
use crate::style::{self, ACCENT, OK, WARN};
use eframe::egui::{self, Margin, RichText};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use unpack::{
    CompressOptions, Level, Mode, Options, Progress, Selection, compress_with, default_archive_path, extract, extract_auto,
    is_cancelled, is_password_error, is_too_large, move_to_trash, test_archive,
};

/// 한 압축 파일의 작업 결과
#[derive(PartialEq, Clone)]
enum Status {
    /// 결과 경로, 휴지통으로 보냈는지
    Done(PathBuf, bool),
    Tested(String),
    NeedPassword,
    TooLarge(String),
    Cancelled,
    Failed(String),
}

enum Msg {
    Finished(PathBuf, Status),
    Step(usize),
    Packed(Result<PathBuf, String>),
    Opened(Result<PathBuf, String>),
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

#[derive(Clone, Copy, PartialEq)]
enum Job {
    Extract(Mode),
    Test,
}

pub struct Home {
    folder: FolderView,
    /// 압축 파일 안을 보는 중이면 Some
    view: Option<ArchiveView>,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// 주소 줄에 경로를 직접 입력하는 중
    editing_path: Option<String>,
    favorites: Vec<Place>,
    drives: Vec<Place>,

    worker: Option<Receiver<Msg>>,
    progress: Option<Arc<Progress>>,
    step: (usize, usize),
    results: Vec<(PathBuf, Status)>,
    /// 압축 폭탄 경고로 멈춘 파일 ("그래도 풀기"용)
    held: Vec<(PathBuf, Mode)>,
    notice: Option<Notice>,

    show_settings: bool,
    settings: Settings,
    saved: Settings,
    password: String,
}

fn status_of(r: anyhow::Result<unpack::Stats>, test: bool) -> (Status, Option<unpack::Stats>) {
    match r {
        Ok(s) if test => (Status::Tested(format!("이상 없음 · 파일 {}개", s.files)), Some(s)),
        Ok(s) => (Status::Done(s.output.clone(), false), Some(s)),
        Err(e) if is_cancelled(&e) => (Status::Cancelled, None),
        Err(e) if is_password_error(&e) => (Status::NeedPassword, None),
        Err(e) if is_too_large(&e) => (Status::TooLarge(format!("{e:#}")), None),
        Err(e) => (Status::Failed(format!("{e:#}")), None),
    }
}

impl Home {
    pub fn new(ctx: &egui::Context, files: Vec<PathBuf>, auto: bool) -> Self {
        let settings = Settings::load();
        // 시작 폴더: 넘겨받은 파일이 있으면 그 위치, 없으면 마지막 폴더 → 홈
        let start = match files.first() {
            Some(p) if p.is_dir() && files.len() == 1 => p.clone(),
            Some(p) => p.parent().map(Path::to_path_buf).unwrap_or_else(places::home),
            None => settings.last_dir.clone().unwrap_or_else(places::home),
        };
        let mut app = Self {
            folder: FolderView::new(start, settings.show_hidden),
            view: None,
            back: Vec::new(),
            forward: Vec::new(),
            editing_path: None,
            favorites: places::favorites(),
            drives: places::drives(),
            worker: None,
            progress: None,
            step: (0, 0),
            results: Vec::new(),
            held: Vec::new(),
            notice: None,
            show_settings: false,
            saved: settings.clone(),
            settings,
            password: String::new(),
        };
        let given: Vec<PathBuf> = files.iter().filter(|p| !p.is_dir() || files.len() > 1).cloned().collect();
        app.folder.select_only(&given);
        if auto {
            let archives: Vec<PathBuf> = given.into_iter().filter(|p| unpack::is_archive_name(p)).collect();
            app.run_archives(ctx, archives, Job::Extract(Mode::Smart), None, false);
        } else if let [one] = given.as_slice()
            && unpack::is_archive_name(one)
        {
            // 압축 파일을 더블클릭해서 열면 바로 안을 보여 줌
            app.open_archive(ctx, one.clone());
        }
        app.update_title(ctx);
        app
    }

    fn busy(&self) -> bool {
        self.worker.is_some()
    }

    fn options(&self) -> Options {
        Options { password: (!self.password.is_empty()).then(|| self.password.clone()), ..Default::default() }
    }

    fn update_title(&self, ctx: &egui::Context) {
        let here = match &self.view {
            Some(v) => v.title(),
            None => crate::file_name(&self.folder.dir),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{here} - {}", unpack::APP_NAME)));
    }

    // ----- 이동 -----

    fn navigate(&mut self, ctx: &egui::Context, dir: PathBuf) {
        if dir == self.folder.dir && self.view.is_none() {
            return;
        }
        self.back.push(self.folder.dir.clone());
        self.forward.clear();
        self.go_to(ctx, dir);
    }

    fn go_to(&mut self, ctx: &egui::Context, dir: PathBuf) {
        self.view = None;
        self.folder = FolderView::new(dir.clone(), self.settings.show_hidden);
        self.settings.last_dir = Some(dir);
        self.editing_path = None;
        self.update_title(ctx);
    }

    fn go_back(&mut self, ctx: &egui::Context) {
        if self.view.is_some() {
            self.exit_archive(ctx);
            return;
        }
        if let Some(d) = self.back.pop() {
            self.forward.push(self.folder.dir.clone());
            self.go_to(ctx, d);
        }
    }

    fn go_forward(&mut self, ctx: &egui::Context) {
        if let Some(d) = self.forward.pop() {
            self.back.push(self.folder.dir.clone());
            self.go_to(ctx, d);
        }
    }

    fn go_up(&mut self, ctx: &egui::Context) {
        if let Some(v) = self.view.as_mut() {
            if v.at_root() {
                self.exit_archive(ctx);
            } else {
                v.go_up();
            }
            return;
        }
        let cur = self.folder.dir.clone();
        if let Some(parent) = cur.parent() {
            self.navigate(ctx, parent.to_path_buf());
            self.folder.select_only(&[cur]);
        }
    }

    fn open_archive(&mut self, ctx: &egui::Context, path: PathBuf) {
        if let Some(parent) = path.parent()
            && parent != self.folder.dir
        {
            self.navigate(ctx, parent.to_path_buf());
        }
        self.folder.select_only(std::slice::from_ref(&path));
        self.view = Some(ArchiveView::open(&path, self.options().password));
        self.update_title(ctx);
    }

    fn exit_archive(&mut self, ctx: &egui::Context) {
        self.view = None;
        self.update_title(ctx);
    }

    /// 더블클릭·Enter: 폴더 들어가기 / 압축 파일 안 보기 / 기본 프로그램으로 열기
    fn open_path(&mut self, ctx: &egui::Context, path: PathBuf) {
        if path.is_dir() {
            self.navigate(ctx, path);
        } else if unpack::is_archive_name(&path) {
            self.open_archive(ctx, path);
        } else {
            crate::open_with_default(&path);
        }
    }

    fn reload(&mut self) {
        self.folder.reload(self.settings.show_hidden);
    }

    // ----- 작업 -----

    fn start_progress(&mut self) -> Arc<Progress> {
        let p = Progress::new();
        self.progress = Some(p.clone());
        p
    }

    fn run(&mut self, ctx: &egui::Context, job: impl FnOnce(&Tx) + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        let tx = Tx { tx, ctx: ctx.clone() };
        thread::spawn(move || {
            job(&tx);
            tx.send(Msg::AllDone);
        });
        self.notice = None;
        self.results.clear();
        self.worker = Some(rx);
    }

    /// 지금 고른 압축 파일들 (압축 파일 안을 보는 중이면 그 파일)
    fn selected_archives(&self) -> Vec<PathBuf> {
        if let Some(v) = &self.view {
            return if matches!(v.contents, Contents::Ok(_)) { vec![v.path.clone()] } else { vec![] };
        }
        self.folder.selection().into_iter().filter(|e| e.archive).map(|e| e.path.clone()).collect()
    }

    /// 압축 파일 여러 개를 풀거나 검사
    fn run_archives(&mut self, ctx: &egui::Context, archives: Vec<PathBuf>, job: Job, to: Option<PathBuf>, allow_large: bool) {
        if archives.is_empty() {
            return;
        }
        let parent = to.or_else(|| self.settings.out_dir.clone());
        let progress = self.start_progress();
        let opts = Options { progress: Some(progress.clone()), allow_large, ..self.options() };
        let trash = self.settings.trash_after && matches!(job, Job::Extract(_));
        self.step = (0, archives.len());
        self.run(ctx, move |tx| {
            for (n, path) in archives.into_iter().enumerate() {
                if progress.is_cancelled() {
                    tx.send(Msg::Finished(path, Status::Cancelled));
                    continue;
                }
                tx.send(Msg::Step(n + 1));
                let dest = parent.clone().unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_default());
                let r = match job {
                    Job::Test => test_archive(&path, &opts),
                    Job::Extract(mode) => extract_auto(&path, &dest, mode, &opts),
                };
                let (mut status, stats) = status_of(r, job == Job::Test);
                if trash
                    && let (Status::Done(out, _), Some(s)) = (&status, &stats)
                    && s.warnings.is_empty()
                    && s.skipped == 0
                {
                    status = Status::Done(out.clone(), move_to_trash(&path).is_ok());
                }
                // 압축 폭탄 경고면 어떤 방식이었는지 기억 ("그래도 풀기")
                let status = match (status, job) {
                    (Status::TooLarge(m), Job::Extract(mode)) => Status::TooLarge(format!("{}\u{1}{m}", mode as u8)),
                    (s, _) => s,
                };
                tx.send(Msg::Finished(path, status));
            }
        });
    }

    fn extract(&mut self, ctx: &egui::Context, mode: Mode, to: Option<PathBuf>) {
        // 압축 파일 안에서 고른 항목이 있으면 그것만
        if self.view.as_ref().is_some_and(|v| v.has_selection()) {
            self.extract_selection(ctx, to);
            return;
        }
        let archives = self.selected_archives();
        self.run_archives(ctx, archives, Job::Extract(mode), to, false);
    }

    fn test(&mut self, ctx: &egui::Context) {
        let archives = self.selected_archives();
        self.run_archives(ctx, archives, Job::Test, None, true);
    }

    /// 압축 파일 안에서 고른 항목만, 압축 파일 옆(또는 `to`)에 그대로 풀기
    fn extract_selection(&mut self, ctx: &egui::Context, to: Option<PathBuf>) {
        let Some(v) = &self.view else { return };
        let Some(sel) = v.selection() else { return };
        let path = v.path.clone();
        let dest = to
            .or_else(|| self.settings.out_dir.clone())
            .unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_default());
        let progress = self.start_progress();
        let opts = Options { progress: Some(progress), select: Some(Arc::new(sel)), ..self.options() };
        self.step = (0, 1);
        self.run(ctx, move |tx| {
            let r = extract(&path, Some(&dest), &opts).map(|mut s| {
                s.output = dest.clone();
                s
            });
            tx.send(Msg::Finished(path, status_of(r, false).0));
        });
    }

    /// 압축 파일 안 파일 하나를 임시 폴더에 풀어 기본 프로그램으로 열기
    fn open_entry(&mut self, ctx: &egui::Context, full: String) {
        let Some(v) = &self.view else { return };
        let path = v.path.clone();
        let dir = crate::open_temp_dir();
        let name = full.rsplit('/').next().unwrap_or(&full).to_string();
        let prefix = full.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
        let progress = self.start_progress();
        let opts = Options {
            progress: Some(progress),
            select: Some(Arc::new(Selection { prefix, names: vec![full] })),
            ..self.options()
        };
        self.step = (0, 0);
        self.run(ctx, move |tx| {
            let r = extract(&path, Some(&dir), &opts).map(|_| dir.join(&name)).map_err(|e| {
                if is_password_error(&e) { "암호가 필요합니다. 설정에 암호를 넣고 다시 여세요.".to_string() } else { format!("{e:#}") }
            });
            tx.send(Msg::Opened(r));
        });
    }

    /// 고른 파일·폴더를 압축 (설정의 형식·수준, 같은 폴더 또는 설정 위치에)
    fn pack(&mut self, ctx: &egui::Context) {
        let inputs: Vec<PathBuf> = self.folder.selection().into_iter().map(|e| e.path.clone()).collect();
        if inputs.is_empty() {
            return;
        }
        let kind = self.settings.kind;
        let mut out = default_archive_path(&inputs, kind);
        if let Some(dir) = &self.settings.out_dir {
            let name = out.file_name().unwrap_or_default().to_owned();
            out = dir.join(name);
            if out.exists() {
                let stem = out.file_name().unwrap_or_default().to_string_lossy().trim_end_matches(kind.ext()).to_string();
                out = (2..).map(|n| dir.join(format!("{stem} ({n}){}", kind.ext()))).find(|p| !p.exists()).unwrap();
            }
        }
        let progress = self.start_progress();
        let opts = CompressOptions {
            password: (kind.supports_password() && !self.password.is_empty()).then(|| self.password.clone()),
            level: self.settings.level,
            progress: Some(progress),
        };
        self.step = (0, 0);
        self.run(ctx, move |tx| {
            let r = compress_with(&inputs, &out, kind, &opts)
                .map(|s| s.output)
                .map_err(|e| if is_cancelled(&e) { "취소했습니다".to_string() } else { format!("{e:#}") });
            tx.send(Msg::Packed(r));
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.worker else { return };
        let mut done = false;
        let mut packed = None;
        let mut opened = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Step(n) => self.step.0 = n,
                Msg::Finished(p, s) => self.results.push((p, s)),
                Msg::Packed(r) => packed = Some(r),
                Msg::Opened(r) => opened = Some(r),
                Msg::AllDone => done = true,
            }
        }
        if !done {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        self.worker = None;
        self.progress = None;
        if let Some(r) = opened {
            match r {
                Ok(p) => crate::open_with_default(&p),
                Err(e) => self.notice = Some(Notice { ok: false, text: e, open: None }),
            }
            return;
        }
        self.reload();
        if let Some(r) = packed {
            self.notice = Some(match r {
                Ok(p) => {
                    self.folder.select_only(std::slice::from_ref(&p));
                    Notice { ok: true, text: format!("{} 을(를) 만들었습니다", crate::file_name(&p)), open: Some(p) }
                }
                Err(e) => Notice { ok: false, text: e, open: None },
            });
            return;
        }
        self.summarize(ctx);
    }

    /// 작업이 끝나면 상태 표시줄에 결과 한 줄 + 결과를 표에서 선택
    fn summarize(&mut self, ctx: &egui::Context) {
        self.held = self
            .results
            .iter()
            .filter_map(|(p, s)| match s {
                Status::TooLarge(m) => {
                    let mode = match m.split('\u{1}').next() {
                        Some("1") => Mode::Here,
                        Some("2") => Mode::NewFolder,
                        _ => Mode::Smart,
                    };
                    Some((p.clone(), mode))
                }
                _ => None,
            })
            .collect();
        let find = |f: &dyn Fn(&Status) -> bool| self.results.iter().find(|(_, s)| f(s));
        self.notice = if find(&|s| *s == Status::NeedPassword).is_some() {
            self.show_settings = true;
            Some(Notice { ok: false, text: "암호가 필요합니다. 오른쪽 설정에 암호를 넣고 다시 누르세요.".into(), open: None })
        } else if let Some((_, Status::TooLarge(m))) = find(&|s| matches!(s, Status::TooLarge(_))) {
            Some(Notice { ok: false, text: m.split('\u{1}').next_back().unwrap_or(m).to_string(), open: None })
        } else if let Some((p, Status::Failed(e))) = find(&|s| matches!(s, Status::Failed(_))) {
            Some(Notice { ok: false, text: format!("{}: {e}", crate::file_name(p)), open: None })
        } else if find(&|s| *s == Status::Cancelled).is_some() {
            Some(Notice { ok: false, text: "취소했습니다".into(), open: None })
        } else if let Some((p, Status::Tested(t))) = self.results.iter().rev().find(|(_, s)| matches!(s, Status::Tested(_))) {
            let n = self.results.len();
            let who = if n > 1 { format!("{n}개 모두") } else { crate::file_name(p) };
            Some(Notice { ok: true, text: format!("{who}: {t}"), open: None })
        } else if let Some((_, Status::Done(out, trashed))) = self.results.iter().rev().find(|(_, s)| matches!(s, Status::Done(..))) {
            let extra = if *trashed { " · 압축 파일은 휴지통으로" } else { "" };
            Some(Notice { ok: true, text: format!("압축을 풀었습니다 → {}{extra}", crate::file_name(out)), open: Some(out.clone()) })
        } else {
            None
        };
        // 풀린 결과가 지금 폴더에 있으면 선택해서 보여 줌
        let outs: Vec<PathBuf> = self.results.iter().filter_map(|(_, s)| if let Status::Done(o, _) = s { Some(o.clone()) } else { None }).collect();
        if self.view.is_none() && !outs.is_empty() {
            self.folder.select_only(&outs);
        }
        // 휴지통으로 보낸 압축 파일을 보고 있었다면 폴더로
        if let Some(v) = &self.view
            && !v.path.exists()
        {
            self.exit_archive(ctx);
        }
    }

    fn command(&mut self, ctx: &egui::Context, c: FsCommand) {
        match c {
            FsCommand::Extract => self.extract(ctx, Mode::Smart, None),
            FsCommand::ExtractHere => self.extract(ctx, Mode::Here, None),
            FsCommand::ExtractNewFolder => self.extract(ctx, Mode::NewFolder, None),
            FsCommand::ExtractTo => {
                if let Some(dir) = rfd::FileDialog::new().set_title("풀 위치").set_directory(&self.folder.dir).pick_folder() {
                    self.extract(ctx, Mode::Smart, Some(dir));
                }
            }
            FsCommand::Test => self.test(ctx),
            FsCommand::Compress => self.pack(ctx),
            FsCommand::ShowInSystem => {
                if let Some(e) = self.folder.selection().first() {
                    crate::open_in_explorer(&e.path);
                }
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
                let in_archive = self.view.is_some();
                let has_sel = !self.folder.selection().is_empty();
                let archives = !self.selected_archives().is_empty();
                let selecting = self.view.as_ref().is_some_and(|v| v.has_selection());

                let open_target = if in_archive { None } else { self.folder.selection().first().map(|e| e.path.clone()) };
                if style::tool_button(ui, Icon::Open, "열기", idle && open_target.is_some())
                    .on_hover_text("폴더에 들어가거나, 압축 파일 안을 보거나, 파일을 엽니다")
                    .clicked()
                    && let Some(p) = open_target
                {
                    self.open_path(ctx, p);
                }
                style::vsep(ui);
                let (label, tip) = if selecting {
                    ("선택 풀기", "고른 항목만 압축 파일 옆에 풀기")
                } else {
                    ("압축 풀기", "알아서 풀기: 폴더가 하나면 그대로, 아니면 파일 이름 폴더로")
                };
                if style::tool_button(ui, Icon::Extract, label, idle && archives).on_hover_text(tip).clicked() {
                    self.extract(ctx, Mode::Smart, None);
                }
                if style::tool_button(ui, Icon::ExtractTo, "다른 위치에", idle && archives).on_hover_text("풀 위치를 골라서 풀기").clicked() {
                    self.command(ctx, FsCommand::ExtractTo);
                }
                if style::tool_button(ui, Icon::Check, "검사", idle && archives).on_hover_text("풀지 않고 손상 여부 확인").clicked() {
                    self.test(ctx);
                }
                if style::tool_button(ui, Icon::Compress, "압축하기", idle && !in_archive && has_sel)
                    .on_hover_text(format!(
                        "고른 항목을 {} 하나로 압축 ({})",
                        self.settings.kind.label(),
                        self.settings.level.label()
                    ))
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

    fn nav_button(ui: &mut egui::Ui, icon: Icon, enabled: bool, tip: &str) -> bool {
        let (r, resp) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
        if enabled && resp.hovered() {
            ui.painter().rect_filled(r, 6.0, style::hover_fill(ui));
        }
        let color = if enabled { ui.visuals().text_color() } else { style::weak(ui).gamma_multiply(0.5) };
        icons::paint(ui.painter(), r.shrink(7.0), icon, color);
        enabled && resp.on_hover_text(tip).clicked()
    }

    fn address_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let frame = egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(Margin { left: 8, right: 10, top: 8, bottom: 6 });
        let mut go: Option<PathBuf> = None;
        let (mut back, mut fwd, mut up) = (false, false, false);
        egui::Panel::top("address").frame(frame).show_separator_line(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                back = Self::nav_button(ui, Icon::Back, !self.back.is_empty() || self.view.is_some(), "뒤로 (Alt+←)");
                fwd = Self::nav_button(ui, Icon::Forward, !self.forward.is_empty() && self.view.is_none(), "앞으로 (Alt+→)");
                up = Self::nav_button(ui, Icon::Up, self.view.is_some() || self.folder.dir.parent().is_some(), "위로 (Alt+↑)");
                ui.add_space(6.0);
                let field = egui::Frame::NONE
                    .fill(ui.visuals().extreme_bg_color)
                    .stroke(egui::Stroke::new(1.0, style::border(ui)))
                    .corner_radius(6)
                    .inner_margin(Margin { left: 8, right: 8, top: 3, bottom: 3 });
                let field_w = ui.available_width() - 18.0;
                field.show(ui, |ui| {
                    ui.set_width(field_w);
                    ui.set_max_width(field_w);
                    if let Some(text) = self.editing_path.as_mut() {
                        // 경로 직접 입력
                        let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).frame(egui::Frame::NONE));
                        if !r.has_focus() && !r.lost_focus() {
                            r.request_focus();
                        }
                        if r.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                go = Some(PathBuf::from(text.trim()));
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
                            // 뒤쪽부터 들어가는 만큼만 (글자 폭을 대략 7.5px로 계산)
                            let budget = (field_w - 60.0 - if self.view.is_some() { 220.0 } else { 0.0 }).max(80.0);
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
                                    go = Some(path);
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
            });
        });
        if back {
            self.go_back(ctx);
        } else if fwd {
            self.go_forward(ctx);
        } else if up {
            self.go_up(ctx);
        }
        if let Some(p) = go {
            if p.is_dir() {
                self.navigate(ctx, p);
            } else if p.is_file() {
                self.open_path(ctx, p);
            } else {
                self.notice = Some(Notice { ok: false, text: format!("찾을 수 없는 경로: {}", p.display()), open: None });
            }
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let frame = style::bar(ui, Margin { left: 8, right: 8, top: 10, bottom: 10 });
        let mut go = None;
        egui::Panel::left("places").frame(frame).resizable(false).exact_size(190.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let section = |ui: &mut egui::Ui, title: &str| {
                    ui.add_space(4.0);
                    ui.label(RichText::new(title).size(12.0).color(style::weak(ui)));
                };
                let mut row = |ui: &mut egui::Ui, p: &Place| {
                    let current = self.view.is_none() && self.folder.dir == p.path;
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::click());
                    if current {
                        ui.painter().rect_filled(rect, 6.0, style::selection_fill(ui));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 6.0, style::hover_fill(ui));
                    }
                    let ir = egui::Rect::from_center_size(rect.left_center() + egui::vec2(16.0, 0.0), egui::vec2(18.0, 18.0));
                    icons::paint(ui.painter(), ir, if p.drive { Icon::Drive } else { Icon::Folder }, style::weak(ui));
                    ui.painter().text(
                        rect.left_center() + egui::vec2(32.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        &p.label,
                        egui::FontId::proportional(13.5),
                        ui.visuals().text_color(),
                    );
                    if resp.on_hover_text(p.path.display().to_string()).clicked() {
                        go = Some(p.path.clone());
                    }
                };
                section(ui, "즐겨찾기");
                for p in &self.favorites {
                    row(ui, p);
                }
                ui.add_space(10.0);
                section(ui, "드라이브");
                for p in &self.drives {
                    row(ui, p);
                }
            });
        });
        if let Some(p) = go {
            self.navigate(ctx, p);
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        let frame = style::bar(ui, Margin::same(16));
        let hidden_before = self.settings.show_hidden;
        egui::Panel::right("settings").frame(frame).resizable(false).exact_size(260.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("설정");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("✕").frame(false)).clicked() {
                            self.show_settings = false;
                        }
                    });
                });
                ui.add_space(10.0);

                ui.weak("압축 풀 위치");
                let here = self.settings.out_dir.is_none();
                if ui.radio(here, "압축 파일과 같은 폴더").clicked() {
                    self.settings.out_dir = None;
                }
                let label = self.settings.out_dir.as_ref().map(|d| crate::file_name(d)).unwrap_or_else(|| "다른 폴더…".into());
                let hover = self.settings.out_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "폴더 고르기".into());
                if ui.radio(!here, label).on_hover_text(hover).clicked()
                    && let Some(d) = rfd::FileDialog::new().set_title("저장 위치").pick_folder()
                {
                    self.settings.out_dir = Some(d);
                }
                ui.add_space(12.0);

                ui.weak("압축 형식");
                ui.horizontal_wrapped(|ui| {
                    for k in unpack::ArchiveKind::ALL {
                        ui.selectable_value(&mut self.settings.kind, k, k.label());
                    }
                });
                ui.add_space(8.0);
                ui.weak("압축 수준");
                ui.horizontal(|ui| {
                    for l in Level::ALL {
                        ui.selectable_value(&mut self.settings.level, l, l.label());
                    }
                });
                ui.add_space(12.0);

                ui.weak("암호 (풀기·압축 공통, 저장 안 함)");
                ui.add(
                    egui::TextEdit::singleline(&mut self.password).password(true).hint_text("없으면 비워 두세요").desired_width(f32::INFINITY),
                );
                if !self.settings.kind.supports_password() && !self.password.is_empty() {
                    ui.colored_label(WARN, format!("{}는 암호를 지원하지 않습니다", self.settings.kind.label()));
                }
                ui.add_space(12.0);

                ui.weak("풀고 나서");
                ui.checkbox(&mut self.settings.trash_after, "압축 파일을 휴지통으로 보내기")
                    .on_hover_text("경고 없이 모두 풀렸을 때만 보냅니다. 휴지통에서 되살릴 수 있습니다.");
                ui.add_space(12.0);

                ui.weak("보기");
                ui.checkbox(&mut self.settings.show_hidden, "숨김 파일 표시");
            });
        });
        if hidden_before != self.settings.show_hidden {
            self.reload();
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let frame = style::bar(ui, Margin { left: 14, right: 14, top: 6, bottom: 6 });
        let mut retry_large = false;
        egui::Panel::bottom("status").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if let Some(p) = self.progress.clone() {
                    let (n, total) = self.step;
                    let step = if total > 1 { format!("{n}/{total} · ") } else { String::new() };
                    let pct = p.fraction().map(|f| format!("{}%", (f * 100.0) as u32)).unwrap_or_default();
                    ui.add(
                        egui::ProgressBar::new(p.fraction().unwrap_or(0.0))
                            .desired_width(220.0)
                            .desired_height(8.0)
                            .animate(p.fraction().is_none()),
                    );
                    ui.label(RichText::new(format!("{step}{pct}")).size(12.5).color(ACCENT));
                    let cur: String = p.current().chars().take(40).collect();
                    ui.label(RichText::new(cur).size(12.5).color(style::weak(ui)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if p.is_cancelled() { "취소하는 중…" } else { "취소" };
                        if ui.add_enabled(!p.is_cancelled(), egui::Button::new(label).small()).clicked() {
                            p.cancel();
                        }
                    });
                    return;
                }
                let info = match &self.view {
                    Some(v) => v.status(),
                    None => self.folder.status(),
                };
                ui.label(RichText::new(info).size(12.5).color(style::weak(ui)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(n) = &self.notice {
                        if let Some(p) = n.open.clone()
                            && ui.link(RichText::new("폴더 열기").size(12.5)).clicked()
                        {
                            crate::open_in_explorer(&p);
                        }
                        if !self.held.is_empty() && ui.button(RichText::new("그래도 풀기").size(12.5)).clicked() {
                            retry_large = true;
                        }
                        let short: String = if n.text.chars().count() > 46 {
                            n.text.chars().take(45).chain(std::iter::once('…')).collect()
                        } else {
                            n.text.clone()
                        };
                        ui.label(RichText::new(short).size(12.5).color(if n.ok { OK } else { WARN })).on_hover_text(&n.text);
                    }
                });
            });
        });
        if retry_large {
            let held = std::mem::take(&mut self.held);
            // 같은 방식으로 다시 (보통은 모두 같은 방식)
            let mode = held.first().map(|(_, m)| *m).unwrap_or(Mode::Smart);
            let paths = held.into_iter().map(|(p, _)| p).collect();
            self.run_archives(ctx, paths, Job::Extract(mode), None, true);
        }
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (alt_left, alt_right, alt_up, f5, ctrl_l) = ctx.input(|i| {
            (
                i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft),
                i.modifiers.alt && i.key_pressed(egui::Key::ArrowRight),
                i.modifiers.alt && i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::F5),
                i.modifiers.command && i.key_pressed(egui::Key::L),
            )
        });
        if alt_left {
            self.go_back(ctx);
        }
        if alt_right {
            self.go_forward(ctx);
        }
        if alt_up {
            self.go_up(ctx);
        }
        if f5 {
            self.reload();
            self.drives = places::drives();
        }
        if ctrl_l {
            self.editing_path = Some(self.folder.dir.display().to_string());
        }
    }
}

impl eframe::App for Home {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);

        // 설정이 바뀌면 저장 (마지막 폴더 포함)
        if self.settings != self.saved {
            self.settings.save();
            self.saved = self.settings.clone();
        }

        // 끌어다 놓기: 폴더 하나면 그 폴더로, 파일이면 그 위치로 가서 선택
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() && !self.busy() {
            match dropped.as_slice() {
                [one] if one.is_dir() => self.navigate(&ctx, one.clone()),
                [one] if unpack::is_archive_name(one) => self.open_archive(&ctx, one.clone()),
                many => {
                    if let Some(parent) = many[0].parent() {
                        self.navigate(&ctx, parent.to_path_buf());
                        self.folder.select_only(many);
                    }
                }
            }
        }
        self.keyboard(&ctx);

        self.toolbar(ui, &ctx);
        self.status_bar(ui, &ctx);
        self.sidebar(ui, &ctx);
        if self.show_settings {
            self.settings_panel(ui);
        }
        self.address_bar(ui, &ctx);

        let frame = egui::Frame::central_panel(&ctx.global_style()).inner_margin(Margin { left: 8, right: 8, top: 2, bottom: 4 });
        let mut fs_action = FsAction::None;
        let mut view_action = ViewAction::None;
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if let Some(view) = self.view.as_mut() {
                view_action = view.table_ui(ui, &mut self.password);
            } else {
                fs_action = self.folder.table_ui(ui);
            }
        });

        if !self.busy() {
            match fs_action {
                FsAction::None => {}
                FsAction::Open(p) => self.open_path(&ctx, p),
                FsAction::Up => self.go_up(&ctx),
                FsAction::Command(c) => self.command(&ctx, c),
            }
        }
        match view_action {
            ViewAction::None => {}
            ViewAction::Retry => {
                let pw = Some(self.password.clone());
                if let Some(v) = self.view.as_mut() {
                    v.reload(pw);
                }
            }
            ViewAction::Exit => self.exit_archive(&ctx),
            ViewAction::Open(full) if !self.busy() => self.open_entry(&ctx, full),
            ViewAction::ExtractSelected { choose } if !self.busy() => {
                let to = if choose { rfd::FileDialog::new().set_title("풀 위치").pick_folder() } else { None };
                if !choose || to.is_some() {
                    self.extract_selection(&ctx, to);
                }
            }
            _ => {}
        }
    }
}
