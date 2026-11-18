//! 메인 창 — 도구 모음 / (주소 줄) / 표 / 상태 표시줄, 오른쪽에 설정 패널.

use crate::browse::{ArchiveView, Contents, ViewAction, kind_of};
use crate::icons::Icon;
use crate::settings::Settings;
use crate::style::{self, ACCENT, ERR, OK, WARN};
use crate::table::{self, Col};
use eframe::egui::{self, Margin, RichText};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use unpack::{
    ArchiveKind, CompressOptions, Level, Mode, Options, Progress, Selection, compress_with, default_archive_path, detect,
    extract, extract_auto, is_cancelled, is_password_error, is_too_large, move_to_trash, test_archive,
};

#[derive(PartialEq, Clone)]
enum Status {
    Idle,
    Working,
    /// 결과 경로, 휴지통으로 보냈는지
    Done(PathBuf, bool),
    /// 검사 결과 (이상 없음)
    Tested(String),
    NeedPassword,
    /// 압축 폭탄 의심 / 디스크 공간 부족 — "그래도 풀기" 가능
    TooLarge(String),
    Cancelled,
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

/// 풀기 작업 종류
#[derive(Clone, Copy, PartialEq)]
enum Job {
    Extract,
    Test,
}

pub struct Home {
    items: Vec<Item>,
    worker: Option<Receiver<Msg>>,
    progress: Option<Arc<Progress>>,
    /// 여러 파일 작업 중 몇 번째인지 (상태 표시줄용)
    step: (usize, usize),
    notice: Option<Notice>,
    show_settings: bool,
    settings: Settings,
    saved: Settings,
    password: String,
    /// 압축 파일 내용 보기 (있으면 표 대신 표시)
    view: Option<ArchiveView>,
    selected: Option<usize>,
    sort: table::Sort,
}

fn status_of(r: anyhow::Result<unpack::Stats>, job: Job) -> (Status, Option<unpack::Stats>) {
    match r {
        Ok(s) if job == Job::Test => (Status::Tested(format!("이상 없음 · 파일 {}개", s.files)), Some(s)),
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
        let mut app = Self {
            items: Vec::new(),
            worker: None,
            progress: None,
            step: (0, 0),
            notice: None,
            show_settings: false,
            saved: settings.clone(),
            settings,
            password: String::new(),
            view: None,
            selected: None,
            sort: (0, true),
        };
        app.add(files);
        if auto {
            app.extract(ctx, None, false);
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

    /// 풀기·검사 대상: 내용 보기 중이면 그 파일, 아니면 목록의 압축 파일
    fn targets(&self, job: Job) -> Vec<usize> {
        if let Some(v) = &self.view {
            return if matches!(v.contents, Contents::Ok(_)) && self.items[v.idx].path.exists() { vec![v.idx] } else { vec![] };
        }
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.archive && it.path.exists() && (job == Job::Test || !matches!(it.status, Status::Done(..))))
            .map(|(i, _)| i)
            .collect()
    }

    fn start_progress(&mut self) -> Arc<Progress> {
        let p = Progress::new();
        self.progress = Some(p.clone());
        p
    }

    /// 알아서 풀기 (또는 검사). `to`가 없으면 설정의 저장 위치 또는 각 압축 파일 위치.
    fn run_archives(&mut self, ctx: &egui::Context, ids: Vec<usize>, to: Option<PathBuf>, job: Job, allow_large: bool) {
        let parent = to.or_else(|| self.settings.out_dir.clone());
        let jobs: Vec<(usize, PathBuf)> = ids.into_iter().map(|i| (i, self.items[i].path.clone())).collect();
        if jobs.is_empty() {
            return;
        }
        let progress = self.start_progress();
        let opts = Options { progress: Some(progress.clone()), allow_large, ..self.options() };
        let trash = self.settings.trash_after && job == Job::Extract;
        self.step = (0, jobs.len());
        self.run(ctx, move |tx| {
            for (i, path) in jobs {
                if progress.is_cancelled() {
                    tx.send(Msg::Finished(i, Status::Cancelled));
                    continue;
                }
                tx.send(Msg::Started(i));
                let parent = parent.clone().unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_default());
                let r = match job {
                    Job::Test => test_archive(&path, &opts),
                    Job::Extract => extract_auto(&path, &parent, Mode::Smart, &opts),
                };
                let (mut status, stats) = status_of(r, job);
                // 문제없이 풀렸으면 (설정에 따라) 압축 파일을 휴지통으로
                if trash
                    && let (Status::Done(out, _), Some(s)) = (&status, &stats)
                    && s.warnings.is_empty()
                    && s.skipped == 0
                {
                    status = Status::Done(out.clone(), move_to_trash(&path).is_ok());
                }
                tx.send(Msg::Finished(i, status));
            }
        });
    }

    fn extract(&mut self, ctx: &egui::Context, to: Option<PathBuf>, allow_large: bool) {
        // 내용 보기에서 고른 항목이 있으면 그것만
        if self.view.as_ref().is_some_and(|v| v.has_selection()) {
            self.extract_selection(ctx, to);
            return;
        }
        let ids = self.targets(Job::Extract);
        self.run_archives(ctx, ids, to, Job::Extract, allow_large);
    }

    fn test(&mut self, ctx: &egui::Context) {
        let ids = self.targets(Job::Test);
        self.run_archives(ctx, ids, None, Job::Test, true);
    }

    /// 내용 보기에서 고른 항목만, 압축 파일 옆(또는 `to`)에 그대로 풀기
    fn extract_selection(&mut self, ctx: &egui::Context, to: Option<PathBuf>) {
        let Some(v) = &self.view else { return };
        let Some(sel) = v.selection() else { return };
        let idx = v.idx;
        let path = self.items[idx].path.clone();
        let dest = to
            .or_else(|| self.settings.out_dir.clone())
            .unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_default());
        let progress = self.start_progress();
        let opts = Options { progress: Some(progress), select: Some(Arc::new(sel)), ..self.options() };
        self.step = (0, 1);
        self.run(ctx, move |tx| {
            tx.send(Msg::Started(idx));
            let (status, _) = status_of(extract(&path, Some(&dest), &opts).map(|mut s| {
                s.output = dest.clone();
                s
            }), Job::Extract);
            tx.send(Msg::Finished(idx, status));
        });
    }

    /// 압축 파일 안 파일 하나를 임시 폴더에 풀어 기본 프로그램으로 열기
    fn open_entry(&mut self, ctx: &egui::Context, full: String) {
        let Some(v) = &self.view else { return };
        let path = self.items[v.idx].path.clone();
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

    fn pack(&mut self, ctx: &egui::Context) {
        let inputs: Vec<PathBuf> = self.items.iter().map(|i| i.path.clone()).collect();
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
            let r = compress_with(&inputs, &out, kind, &opts).map(|s| s.output).map_err(|e| {
                if is_cancelled(&e) { "취소했습니다".to_string() } else { format!("{e:#}") }
            });
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

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.worker else { return };
        let mut done = false;
        let mut packed = None;
        let mut opened = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Started(i) => {
                    self.items[i].status = Status::Working;
                    self.step.0 += 1;
                }
                Msg::Finished(i, s) => self.items[i].status = s,
                Msg::Packed(r) => packed = Some(r),
                Msg::Opened(r) => opened = Some(r),
                Msg::AllDone => done = true,
            }
        }
        if !done {
            // 진행률이 바뀌는 동안 계속 그리기
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
        if let Some(r) = packed {
            self.notice = Some(match r {
                Ok(p) => Notice { ok: true, text: format!("{} 을(를) 만들었습니다", crate::file_name(&p)), open: Some(p) },
                Err(e) => Notice { ok: false, text: e, open: None },
            });
            return;
        }
        self.summarize(ctx);
    }

    /// 작업이 끝나면 상태 표시줄에 결과 한 줄
    fn summarize(&mut self, ctx: &egui::Context) {
        let pick = |f: &dyn Fn(&Status) -> bool| self.items.iter().find(|i| f(&i.status));
        self.notice = if pick(&|s| *s == Status::NeedPassword).is_some() {
            self.show_settings = true;
            Some(Notice { ok: false, text: "암호가 필요합니다. 오른쪽 설정에 암호를 넣고 다시 누르세요.".into(), open: None })
        } else if let Some(it) = pick(&|s| matches!(s, Status::TooLarge(_))) {
            let Status::TooLarge(m) = &it.status else { unreachable!() };
            Some(Notice { ok: false, text: m.clone(), open: None })
        } else if let Some(it) = pick(&|s| matches!(s, Status::Failed(_))) {
            let Status::Failed(e) = &it.status else { unreachable!() };
            Some(Notice { ok: false, text: format!("{}: {e}", crate::file_name(&it.path)), open: None })
        } else if pick(&|s| *s == Status::Cancelled).is_some() {
            Some(Notice { ok: false, text: "취소했습니다".into(), open: None })
        } else if let Some(it) = self.items.iter().rev().find(|i| matches!(i.status, Status::Tested(_))) {
            let Status::Tested(t) = &it.status else { unreachable!() };
            Some(Notice { ok: true, text: format!("{}: {t}", crate::file_name(&it.path)), open: None })
        } else if let Some(it) = self.items.iter().rev().find(|i| matches!(i.status, Status::Done(..))) {
            let Status::Done(out, trashed) = &it.status else { unreachable!() };
            let extra = if *trashed { " · 압축 파일은 휴지통으로" } else { "" };
            Some(Notice { ok: true, text: format!("압축을 풀었습니다 → {}{extra}", crate::file_name(out)), open: Some(out.clone()) })
        } else {
            None
        };
        // 휴지통으로 보낸 압축 파일을 보고 있었다면 목록으로
        if let Some(v) = &self.view
            && !self.items[v.idx].path.exists()
        {
            self.close_view(ctx);
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
                let selecting = self.view.as_ref().is_some_and(|v| v.has_selection());
                let can_extract = idle && !self.targets(Job::Extract).is_empty();
                let (label, tip) = if selecting {
                    ("선택 풀기", "고른 항목만 압축 파일 옆에 풀기")
                } else {
                    ("압축 풀기", "알아서 풀기: 폴더가 하나면 그대로, 아니면 파일 이름 폴더로")
                };
                if style::tool_button(ui, Icon::Extract, label, can_extract).on_hover_text(tip).clicked() {
                    self.extract(ctx, None, false);
                }
                if style::tool_button(ui, Icon::ExtractTo, "다른 위치에", can_extract).on_hover_text("풀 위치를 골라서 풀기").clicked()
                    && let Some(dir) = rfd::FileDialog::new().set_title("풀 위치").pick_folder()
                {
                    self.extract(ctx, Some(dir), false);
                }
                let can_test = idle && !self.targets(Job::Test).is_empty();
                if style::tool_button(ui, Icon::Check, "검사", can_test).on_hover_text("풀지 않고 손상 여부 확인").clicked() {
                    self.test(ctx);
                }
                let can_pack = idle && self.view.is_none() && !self.items.is_empty();
                if style::tool_button(ui, Icon::Compress, "압축하기", can_pack)
                    .on_hover_text(format!("목록 전체를 {} 하나로 압축 ({})", self.settings.kind.label(), self.settings.level.label()))
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
        let frame = egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(Margin { left: 10, right: 10, top: 8, bottom: 6 });
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

            ui.weak("저장 위치");
            let here = self.settings.out_dir.is_none();
            if ui.radio(here, "원래 위치 (압축 파일 옆)").clicked() {
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
                for k in ArchiveKind::ALL {
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
            ui.add(egui::TextEdit::singleline(&mut self.password).password(true).hint_text("없으면 비워 두세요").desired_width(f32::INFINITY));
            if !self.settings.kind.supports_password() && !self.password.is_empty() {
                ui.colored_label(WARN, format!("{}는 암호를 지원하지 않습니다", self.settings.kind.label()));
            }
            ui.add_space(12.0);

            ui.weak("풀고 나서");
            ui.checkbox(&mut self.settings.trash_after, "압축 파일을 휴지통으로 보내기")
                .on_hover_text("경고 없이 모두 풀렸을 때만 보냅니다. 휴지통에서 되살릴 수 있습니다.");
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let frame = style::bar(ui, Margin { left: 14, right: 14, top: 6, bottom: 6 });
        let mut retry_large = false;
        egui::Panel::bottom("status").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if let Some(p) = self.progress.clone() {
                    // 진행 중: 진행 막대 + 지금 항목 + 취소
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
                    None if self.items.is_empty() => format!("{} — 압축 풀기 · 압축하기", unpack::APP_NAME),
                    None => {
                        let archives = self.items.iter().filter(|i| i.archive).count();
                        format!("항목 {}개 · 압축 파일 {}개", self.items.len(), archives)
                    }
                };
                ui.label(RichText::new(info).size(12.5).color(style::weak(ui)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(n) = &self.notice {
                        if let Some(p) = n.open.clone()
                            && ui.link(RichText::new("폴더 열기").size(12.5)).clicked()
                        {
                            crate::open_in_explorer(&p);
                        }
                        if self.items.iter().any(|i| matches!(i.status, Status::TooLarge(_)))
                            && ui.button(RichText::new("그래도 풀기").size(12.5)).clicked()
                        {
                            retry_large = true;
                        }
                        // 긴 문구는 줄이고 전체는 마우스를 올리면 보이게
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
            let ids: Vec<usize> = self.items.iter().enumerate().filter(|(_, i)| matches!(i.status, Status::TooLarge(_))).map(|(i, _)| i).collect();
            self.run_archives(ctx, ids, None, Job::Extract, true);
        }
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
            Col { title: "상태", width: 140.0, right: false },
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
        let mut test_one = None;
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
                    Status::Working => ("처리 중…".into(), Some(ACCENT)),
                    Status::Done(_, true) => ("완료 · 휴지통으로".into(), Some(OK)),
                    Status::Done(_, false) => ("완료".into(), Some(OK)),
                    Status::Tested(_) => ("이상 없음".into(), Some(OK)),
                    Status::NeedPassword => ("암호 필요".into(), Some(WARN)),
                    Status::TooLarge(_) => ("확인 필요".into(), Some(WARN)),
                    Status::Cancelled => ("취소됨".into(), Some(WARN)),
                    Status::Failed(_) => ("실패".into(), Some(ERR)),
                };
                let size = if it.is_dir { String::new() } else { crate::human_size(it.size) };
                let cells = [(crate::file_name(&it.path), None), (kind(it), None), (size, None), (st, color)];
                let r = table::row(ui, &cols, icon, &cells, self.selected == Some(i), row_no);
                let r = match &it.status {
                    Status::Failed(e) | Status::TooLarge(e) => r.on_hover_text(e),
                    _ => r.on_hover_text(it.path.display().to_string()),
                };
                if r.clicked() {
                    self.selected = Some(i);
                }
                if r.double_clicked() {
                    match &it.status {
                        Status::Done(out, _) => crate::open_in_explorer(out),
                        _ if it.archive && it.path.exists() => open = Some(i),
                        _ => {}
                    }
                }
                r.context_menu(|ui| {
                    if it.archive && it.path.exists() && ui.button("내용 보기").clicked() {
                        open = Some(i);
                        ui.close();
                    }
                    if it.archive && it.path.exists() && !busy && ui.button("압축 파일 검사").clicked() {
                        test_one = Some(i);
                        ui.close();
                    }
                    if let Status::Done(out, _) = &it.status
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
        if let Some(i) = test_one {
            self.run_archives(ctx, vec![i], None, Job::Test, true);
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
        self.poll(&ctx);

        // 설정이 바뀌면 저장
        if self.settings != self.saved {
            self.settings.save();
            self.saved = self.settings.clone();
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() && !self.busy() {
            self.add(dropped);
            if self.view.is_some() {
                self.close_view(&ctx);
            }
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        self.toolbar(ui, &ctx);
        self.status_bar(ui, &ctx);
        if self.show_settings {
            self.settings_panel(ui);
        }
        if self.view.is_some() {
            self.address_bar(ui, &ctx);
        }

        let frame = egui::Frame::central_panel(&ctx.global_style()).inner_margin(Margin { left: 8, right: 8, top: 4, bottom: 4 });
        let mut action = ViewAction::None;
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if let Some(view) = self.view.as_mut() {
                action = view.table_ui(ui, &mut self.password);
            } else if self.items.is_empty() {
                self.empty_state(ui, &ctx, hovering);
            } else {
                self.list_table(ui, &ctx);
            }
        });
        match action {
            ViewAction::None => {}
            ViewAction::Retry => {
                let pw = Some(self.password.clone());
                if let Some(v) = self.view.as_mut() {
                    v.reload(pw);
                }
            }
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
