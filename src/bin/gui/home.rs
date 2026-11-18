//! 메인 창 — 파일 탐색기 (최대 4개로 나눠 보기).
//!
//! 왼쪽 사이드바(즐겨찾기·드라이브) / 위 도구 모음 / 가운데 창 1~4개 / 아래 상태 표시줄.
//! 창마다 주소 줄과 폴더 표(또는 압축 파일 안 보기)가 따로 있고, 마지막으로 누른 창이 활성 창입니다.
//! 창끼리 끌어다 놓으면 이동(Ctrl: 복사), 압축 파일 안 항목을 폴더 창에 놓으면 그곳에 풉니다.

use crate::browse::ViewAction;
use crate::dnd::DragItems;
use crate::fileops;
use crate::fsview::{FsAction, FsCommand};
use crate::icons::Icon;
use crate::pane::Pane;
use crate::places::{self, Place};
use crate::settings::Settings;
use crate::style::{self, ACCENT, OK, WARN};
use eframe::egui::{self, Margin, Rect, RichText};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use unpack::{
    CompressOptions, Level, Mode, Options, Progress, Selection, compress_with, default_archive_path, extract, extract_auto,
    is_cancelled, is_password_error, is_too_large, move_to_trash, test_archive,
};

const MAX_PANES: usize = 4;

/// 한 압축 파일의 작업 결과
#[derive(PartialEq, Clone)]
enum Status {
    /// 결과 경로, 휴지통으로 보냈는지
    Done(PathBuf, bool),
    Tested(String),
    NeedPassword,
    /// 압축 폭탄 경고 (어떤 방식이었는지, 문구)
    TooLarge(Mode, String),
    Cancelled,
    Failed(String),
}

enum Msg {
    Finished(PathBuf, Status),
    Step(usize),
    Packed(Result<PathBuf, String>),
    Opened(Result<PathBuf, String>),
    /// 복사·이동 결과 (만든 경로들, 대상 폴더, 이동이었는지)
    Transferred(Result<Vec<PathBuf>, String>, PathBuf, bool),
    /// 압축 파일 안에서 골라 푼 항목들 (표에서 선택해 보여 주기용)
    Unpacked(Vec<PathBuf>),
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

/// 이름 바꾸기 창
struct Renaming {
    path: PathBuf,
    text: String,
    focus: bool,
}

pub struct Home {
    panes: Vec<Pane>,
    active: usize,
    favorites: Vec<Place>,
    drives: Vec<Place>,
    /// 복사·잘라내기한 항목 (잘라내기면 true)
    clipboard: Option<(Vec<PathBuf>, bool)>,
    renaming: Option<Renaming>,

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

fn status_of(r: anyhow::Result<unpack::Stats>, job: Job) -> (Status, Option<unpack::Stats>) {
    match r {
        Ok(s) if job == Job::Test => (Status::Tested(format!("이상 없음 · 파일 {}개", s.files)), Some(s)),
        Ok(s) => (Status::Done(s.output.clone(), false), Some(s)),
        Err(e) if is_cancelled(&e) => (Status::Cancelled, None),
        Err(e) if is_password_error(&e) => (Status::NeedPassword, None),
        Err(e) if is_too_large(&e) => {
            let mode = if let Job::Extract(m) = job { m } else { Mode::Smart };
            (Status::TooLarge(mode, format!("{e:#}")), None)
        }
        Err(e) => (Status::Failed(format!("{e:#}")), None),
    }
}

/// 같은 드라이브(맨 앞 경로 조각)인지 — 같으면 끌어다 놓기가 이동, 다르면 복사 (Windows 탐색기와 같음)
fn same_drive(a: &Path, b: &Path) -> bool {
    a.components().next() == b.components().next()
}

/// 창 n개 배치: 1개 전체 / 2개 좌우 / 3개 위 둘·아래 하나 / 4개 2×2
fn layout(full: Rect, n: usize, gap: f32) -> Vec<Rect> {
    let half_w = (full.width() - gap) / 2.0;
    let half_h = (full.height() - gap) / 2.0;
    let cell = |col: f32, row: f32, w: f32, h: f32| {
        Rect::from_min_size(full.min + egui::vec2(col * (half_w + gap), row * (half_h + gap)), egui::vec2(w, h))
    };
    match n {
        0 | 1 => vec![full],
        2 => vec![cell(0.0, 0.0, half_w, full.height()), cell(1.0, 0.0, half_w, full.height())],
        3 => vec![cell(0.0, 0.0, half_w, half_h), cell(1.0, 0.0, half_w, half_h), cell(0.0, 1.0, full.width(), half_h)],
        _ => vec![
            cell(0.0, 0.0, half_w, half_h),
            cell(1.0, 0.0, half_w, half_h),
            cell(0.0, 1.0, half_w, half_h),
            cell(1.0, 1.0, half_w, half_h),
        ],
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
            panes: vec![Pane::new(start, settings.show_hidden)],
            active: 0,
            favorites: places::favorites(),
            drives: places::drives(),
            clipboard: None,
            renaming: None,
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
        app.panes[0].folder.select_only(&given);
        if auto {
            let archives: Vec<PathBuf> = given.into_iter().filter(|p| unpack::is_archive_name(p)).collect();
            app.run_archives(ctx, archives, Job::Extract(Mode::Smart), None, false);
        } else if let [one] = given.as_slice()
            && unpack::is_archive_name(one)
        {
            // 압축 파일을 더블클릭해서 열면 바로 안을 보여 줌
            let pw = app.options().password;
            app.panes[0].open_archive(one.clone(), pw, app.settings.show_hidden);
        }
        app.update_title(ctx);
        app
    }

    fn busy(&self) -> bool {
        self.worker.is_some()
    }

    fn pane(&self) -> &Pane {
        &self.panes[self.active]
    }

    fn pane_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active]
    }

    fn options(&self) -> Options {
        Options { password: (!self.password.is_empty()).then(|| self.password.clone()), ..Default::default() }
    }

    fn update_title(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{} - {}", self.pane().title(), unpack::APP_NAME)));
    }

    fn reload_all(&mut self) {
        let hidden = self.settings.show_hidden;
        for p in &mut self.panes {
            p.reload(hidden);
        }
    }

    /// 그 폴더를 보고 있는 창마다 `paths`를 선택해 보여 줌
    fn select_in_panes(&mut self, dir: &Path, paths: &[PathBuf]) {
        for p in &mut self.panes {
            if p.view.is_none() && p.dir() == dir {
                p.folder.select_only(paths);
            }
        }
    }

    fn add_pane(&mut self, ctx: &egui::Context) {
        if self.panes.len() < MAX_PANES {
            let dir = self.pane().dir().to_path_buf();
            self.panes.push(Pane::new(dir, self.settings.show_hidden));
            self.active = self.panes.len() - 1;
            self.update_title(ctx);
        }
    }

    fn close_pane(&mut self, ctx: &egui::Context, i: usize) {
        if self.panes.len() > 1 && i < self.panes.len() {
            self.panes.remove(i);
            if self.active >= self.panes.len() || self.active > i {
                self.active = self.active.saturating_sub(1).min(self.panes.len() - 1);
            }
            self.update_title(ctx);
        }
    }

    /// 더블클릭·Enter: 폴더 들어가기 / 압축 파일 안 보기 / 기본 프로그램으로 열기
    fn open_path(&mut self, ctx: &egui::Context, i: usize, path: PathBuf) {
        let hidden = self.settings.show_hidden;
        if path.is_dir() {
            self.panes[i].navigate(path, hidden);
        } else if unpack::is_archive_name(&path) {
            let pw = self.options().password;
            self.panes[i].open_archive(path, pw, hidden);
        } else if path.is_file() {
            crate::open_with_default(&path);
        } else {
            self.notice = Some(Notice { ok: false, text: format!("찾을 수 없는 경로: {}", path.display()), open: None });
        }
        self.update_title(ctx);
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
                let (mut status, stats) = status_of(r, job);
                if trash
                    && let (Status::Done(out, _), Some(s)) = (&status, &stats)
                    && s.warnings.is_empty()
                    && s.skipped == 0
                {
                    status = Status::Done(out.clone(), move_to_trash(&path).is_ok());
                }
                tx.send(Msg::Finished(path, status));
            }
        });
    }

    fn extract(&mut self, ctx: &egui::Context, mode: Mode, to: Option<PathBuf>) {
        // 압축 파일 안에서 고른 항목이 있으면 그것만
        if let Some(v) = &self.pane().view
            && let Some(sel) = v.selection()
        {
            let path = v.path.clone();
            let dest = to
                .or_else(|| self.settings.out_dir.clone())
                .unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_default());
            self.extract_selection(ctx, path, sel, dest);
            return;
        }
        let archives = self.pane().selected_archives();
        self.run_archives(ctx, archives, Job::Extract(mode), to, false);
    }

    fn test(&mut self, ctx: &egui::Context) {
        let archives = self.pane().selected_archives();
        self.run_archives(ctx, archives, Job::Test, None, true);
    }

    /// 압축 파일 안의 고른 항목만 `dest`에 그대로 풀기 (끌어다 놓기에서도 사용)
    fn extract_selection(&mut self, ctx: &egui::Context, archive: PathBuf, sel: Selection, dest: PathBuf) {
        let progress = self.start_progress();
        let opts = Options { progress: Some(progress), select: Some(Arc::new(sel.clone())), ..self.options() };
        self.step = (0, 1);
        // 풀리면 생길 맨 위 항목들 (지금 보는 폴더 기준)
        let made: Vec<PathBuf> = {
            let pre = sel.prefix.trim_end_matches('/');
            let mut v: Vec<PathBuf> = sel
                .names
                .iter()
                .filter_map(|n| {
                    let rel = n.strip_prefix(pre).unwrap_or(n).trim_start_matches('/');
                    rel.split('/').next().filter(|s| !s.is_empty()).map(|first| dest.join(first))
                })
                .collect();
            v.dedup();
            v
        };
        self.run(ctx, move |tx| {
            let r = extract(&archive, Some(&dest), &opts).map(|mut s| {
                s.output = dest.clone();
                s
            });
            let ok = r.is_ok();
            tx.send(Msg::Finished(archive, status_of(r, Job::Extract(Mode::Here)).0));
            if ok {
                tx.send(Msg::Unpacked(made));
            }
        });
    }

    /// 압축 파일 안 파일 하나를 임시 폴더에 풀어 기본 프로그램으로 열기
    fn open_entry(&mut self, ctx: &egui::Context, i: usize, full: String) {
        let Some(v) = &self.panes[i].view else { return };
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
        let inputs = self.pane().folder.selected_paths();
        if inputs.is_empty() || self.pane().view.is_some() {
            return;
        }
        let kind = self.settings.kind;
        let mut out = default_archive_path(&inputs, kind);
        if let Some(dir) = &self.settings.out_dir {
            let name = out.file_name().unwrap_or_default().to_owned();
            out = unpack::unique_path(&dir.join(name));
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

    /// 복사(`mv == false`) 또는 이동
    fn transfer(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, dest: PathBuf, mv: bool) {
        if paths.is_empty() {
            return;
        }
        let progress = self.start_progress();
        self.step = (0, 0);
        self.run(ctx, move |tx| {
            let r = fileops::transfer(&paths, &dest, mv, &progress)
                .map_err(|e| if is_cancelled(&e) { "취소했습니다".to_string() } else { format!("{e:#}") });
            tx.send(Msg::Transferred(r, dest, mv));
        });
    }

    fn handle_drop(&mut self, ctx: &egui::Context, target_pane: usize, items: &DragItems, dest: PathBuf, ctrl: bool) {
        self.active = target_pane;
        match items {
            DragItems::Files { paths, .. } => {
                // Windows 탐색기처럼: 같은 드라이브면 이동, 다른 드라이브면 복사, Ctrl을 누르면 복사
                let mv = !ctrl && paths.first().is_some_and(|p| same_drive(p, &dest));
                self.transfer(ctx, paths.clone(), dest, mv);
            }
            DragItems::Archive { archive, sel, .. } => self.extract_selection(ctx, archive.clone(), sel.clone(), dest),
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.worker else { return };
        let mut done = false;
        let mut packed = None;
        let mut opened = None;
        let mut moved = None;
        let mut unpacked = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Step(n) => self.step.0 = n,
                Msg::Finished(p, s) => self.results.push((p, s)),
                Msg::Packed(r) => packed = Some(r),
                Msg::Opened(r) => opened = Some(r),
                Msg::Transferred(r, d, mv) => moved = Some((r, d, mv)),
                Msg::Unpacked(v) => unpacked = Some(v),
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
        self.reload_all();
        if let Some((r, dest, mv)) = moved {
            self.notice = Some(match r {
                Ok(made) => {
                    self.select_in_panes(&dest, &made);
                    let verb = if mv { "옮겼습니다" } else { "복사했습니다" };
                    Notice { ok: true, text: format!("{}개를 {verb} → {}", made.len(), crate::file_name(&dest)), open: None }
                }
                Err(e) => Notice { ok: false, text: e, open: None },
            });
            return;
        }
        if let Some(r) = packed {
            self.notice = Some(match r {
                Ok(p) => {
                    if let Some(dir) = p.parent() {
                        self.select_in_panes(dir, std::slice::from_ref(&p));
                    }
                    Notice { ok: true, text: format!("{} 을(를) 만들었습니다", crate::file_name(&p)), open: Some(p) }
                }
                Err(e) => Notice { ok: false, text: e, open: None },
            });
            return;
        }
        self.summarize(ctx);
        if let Some(made) = unpacked
            && let Some(dir) = made.first().and_then(|p| p.parent()).map(Path::to_path_buf)
        {
            self.select_in_panes(&dir, &made);
        }
    }

    /// 작업이 끝나면 상태 표시줄에 결과 한 줄 + 결과를 표에서 선택
    fn summarize(&mut self, ctx: &egui::Context) {
        self.held = self.results.iter().filter_map(|(p, s)| if let Status::TooLarge(m, _) = s { Some((p.clone(), *m)) } else { None }).collect();
        let find = |f: &dyn Fn(&Status) -> bool| self.results.iter().find(|(_, s)| f(s));
        self.notice = if find(&|s| *s == Status::NeedPassword).is_some() {
            self.show_settings = true;
            Some(Notice { ok: false, text: "암호가 필요합니다. 오른쪽 설정에 암호를 넣고 다시 누르세요.".into(), open: None })
        } else if let Some((_, Status::TooLarge(_, m))) = find(&|s| matches!(s, Status::TooLarge(..))) {
            Some(Notice { ok: false, text: m.clone(), open: None })
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
        // 풀린 결과를 그 폴더를 보는 창에서 선택
        let outs: Vec<PathBuf> = self.results.iter().filter_map(|(_, s)| if let Status::Done(o, _) = s { Some(o.clone()) } else { None }).collect();
        if let Some(dir) = outs.first().and_then(|o| o.parent()).map(Path::to_path_buf) {
            self.select_in_panes(&dir, &outs);
        }
        // 휴지통으로 보낸 압축 파일을 보던 창은 폴더로
        for p in &mut self.panes {
            if p.view.as_ref().is_some_and(|v| !v.path.exists()) {
                p.view = None;
            }
        }
        self.update_title(ctx);
    }

    fn command(&mut self, ctx: &egui::Context, c: FsCommand) {
        let sel = self.pane().folder.selected_paths();
        let dir = self.pane().dir().to_path_buf();
        match c {
            FsCommand::Extract => self.extract(ctx, Mode::Smart, None),
            FsCommand::ExtractHere => self.extract(ctx, Mode::Here, None),
            FsCommand::ExtractNewFolder => self.extract(ctx, Mode::NewFolder, None),
            FsCommand::ExtractTo => {
                if let Some(to) = rfd::FileDialog::new().set_title("풀 위치").set_directory(&dir).pick_folder() {
                    self.extract(ctx, Mode::Smart, Some(to));
                }
            }
            FsCommand::Test => self.test(ctx),
            FsCommand::Compress => self.pack(ctx),
            FsCommand::ShowInSystem => {
                if let Some(p) = sel.first() {
                    crate::open_in_explorer(p);
                }
            }
            FsCommand::Copy | FsCommand::Cut if !sel.is_empty() => {
                let cut = c == FsCommand::Cut;
                self.notice = Some(Notice {
                    ok: true,
                    text: format!("{}개를 {} — 다른 폴더에서 붙여넣기(Ctrl+V)", sel.len(), if cut { "잘라냈습니다" } else { "복사했습니다" }),
                    open: None,
                });
                // 시스템 클립보드에도 경로를 글자로 넣음 (다른 프로그램에 붙여넣기 가능, Ctrl+V 이벤트도 확실히 옴)
                ctx.copy_text(sel.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
                self.clipboard = Some((sel, cut));
            }
            FsCommand::Paste => {
                if let Some((paths, cut)) = self.clipboard.clone() {
                    if cut {
                        self.clipboard = None; // 잘라내기는 한 번만 붙여넣기
                    }
                    self.transfer(ctx, paths, dir, cut);
                }
            }
            FsCommand::Rename => {
                if let Some(p) = sel.first() {
                    self.renaming = Some(Renaming { path: p.clone(), text: crate::file_name(p), focus: true });
                }
            }
            FsCommand::NewFolder => match fileops::new_folder(&dir) {
                Ok(p) => {
                    self.reload_all();
                    self.pane_mut().folder.select_only(std::slice::from_ref(&p));
                    self.renaming = Some(Renaming { path: p.clone(), text: crate::file_name(&p), focus: true });
                }
                Err(e) => self.notice = Some(Notice { ok: false, text: format!("{e:#}"), open: None }),
            },
            FsCommand::Trash if !sel.is_empty() => {
                self.notice = Some(match fileops::to_trash(&sel) {
                    Ok(()) => Notice { ok: true, text: format!("{}개를 휴지통으로 보냈습니다", sel.len()), open: None },
                    Err(e) => Notice { ok: false, text: format!("{e:#}"), open: None },
                });
                self.reload_all();
            }
            _ => {}
        }
    }

    // ----- 화면 -----

    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let frame = style::bar(ui, Margin { left: 10, right: 10, top: 6, bottom: 6 });
        egui::Panel::top("toolbar").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let idle = !self.busy();
                let in_archive = self.pane().view.is_some();
                let has_sel = !self.pane().folder.selection().is_empty();
                let archives = !self.pane().selected_archives().is_empty();
                let selecting = self.pane().view.as_ref().is_some_and(|v| v.has_selection());

                let open_target = if in_archive { None } else { self.pane().folder.selection().first().map(|e| e.path.clone()) };
                if style::tool_button(ui, Icon::Open, "열기", idle && open_target.is_some())
                    .on_hover_text("폴더에 들어가거나, 압축 파일 안을 보거나, 파일을 엽니다")
                    .clicked()
                    && let Some(p) = open_target
                {
                    self.open_path(ctx, self.active, p);
                }
                style::vsep(ui);
                let (label, tip) = if selecting {
                    ("선택 풀기", "고른 항목만 압축 파일 옆에 풀기 (다른 창으로 끌어다 놓아도 됨)")
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
                    .on_hover_text(format!("고른 항목을 {} 하나로 압축 ({})", self.settings.kind.label(), self.settings.level.label()))
                    .clicked()
                {
                    self.pack(ctx);
                }
                style::vsep(ui);
                if style::tool_button(ui, Icon::NewFolder, "새 폴더", idle && !in_archive).on_hover_text("Ctrl+Shift+N").clicked() {
                    self.command(ctx, FsCommand::NewFolder);
                }
                if style::tool_button(ui, Icon::Split, "창 추가", self.panes.len() < MAX_PANES)
                    .on_hover_text(format!("화면을 나눠 창을 하나 더 엽니다 (최대 {MAX_PANES}개, Ctrl+T)"))
                    .clicked()
                {
                    self.add_pane(ctx);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::tool_button(ui, Icon::Settings, "설정", true).clicked() {
                        self.show_settings = !self.show_settings;
                    }
                });
            });
        });
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
                ui.add(egui::TextEdit::singleline(&mut self.password).password(true).hint_text("없으면 비워 두세요").desired_width(f32::INFINITY));
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
            self.reload_all();
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
                let prefix = if self.panes.len() > 1 { format!("창 {} · ", self.active + 1) } else { String::new() };
                ui.label(RichText::new(format!("{prefix}{}", self.pane().status())).size(12.5).color(style::weak(ui)));
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
            let mode = held.first().map(|(_, m)| *m).unwrap_or(Mode::Smart);
            let paths = held.into_iter().map(|(p, _)| p).collect();
            self.run_archives(ctx, paths, Job::Extract(mode), None, true);
        }
    }

    fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(r) = self.renaming.as_mut() else { return };
        let mut done: Option<bool> = None;
        egui::Window::new("이름 바꾸기")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -60.0))
            .show(ctx, |ui| {
                ui.weak(crate::file_name(&r.path));
                let resp = ui.add(egui::TextEdit::singleline(&mut r.text).desired_width(320.0));
                if std::mem::take(&mut r.focus) {
                    resp.request_focus();
                    // 확장자 앞까지만 선택 (Windows 탐색기와 같음)
                    let stem_len = if r.path.is_dir() {
                        r.text.chars().count()
                    } else {
                        r.text.rfind('.').map(|i| r.text[..i].chars().count()).unwrap_or(r.text.chars().count())
                    };
                    if let Some(mut state) = egui::TextEdit::load_state(ctx, resp.id) {
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(stem_len),
                        )));
                        state.store(ctx, resp.id);
                    }
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add(style::primary("바꾸기")).clicked() {
                        done = Some(true);
                    }
                    if ui.add(style::secondary("취소")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        match done {
            Some(true) => {
                let r = self.renaming.take().expect("이름 바꾸기 중");
                match fileops::rename(&r.path, &r.text) {
                    Ok(newp) => {
                        self.reload_all();
                        if let Some(dir) = newp.parent() {
                            self.select_in_panes(dir, std::slice::from_ref(&newp));
                        }
                    }
                    Err(e) => self.notice = Some(Notice { ok: false, text: format!("{e:#}"), open: None }),
                }
            }
            Some(false) => self.renaming = None,
            None => {}
        }
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.renaming.is_some() {
            return;
        }
        let k = ctx.input(|i| {
            [
                i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft),
                i.modifiers.alt && i.key_pressed(egui::Key::ArrowRight),
                i.modifiers.alt && i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::F5),
                i.modifiers.command && i.key_pressed(egui::Key::L),
                i.modifiers.command && i.key_pressed(egui::Key::T),
                i.modifiers.command && i.key_pressed(egui::Key::W),
                i.key_pressed(egui::Key::Tab) && i.modifiers.command,
            ]
        });
        let [back, fwd, up, f5, ctrl_l, ctrl_t, ctrl_w, next] = k;
        let hidden = self.settings.show_hidden;
        if back {
            self.pane_mut().go_back(hidden);
        }
        if fwd {
            self.pane_mut().go_forward(hidden);
        }
        if up {
            self.pane_mut().go_up(hidden);
        }
        if f5 {
            self.reload_all();
            self.drives = places::drives();
        }
        if ctrl_l {
            let d = self.pane().dir().display().to_string();
            self.pane_mut().editing_path = Some(d);
        }
        if ctrl_t {
            self.add_pane(ctx);
        }
        if ctrl_w {
            self.close_pane(ctx, self.active);
        }
        if next {
            self.active = (self.active + 1) % self.panes.len();
        }
        if back || fwd || up || next {
            self.update_title(ctx);
        }
    }

    /// 끄는 동안 마우스 옆에 "N개 · 이동/복사/풀기" 표시
    fn drag_ghost(&self, ctx: &egui::Context) {
        let Some(items) = egui::DragAndDrop::payload::<DragItems>(ctx) else { return };
        let Some(pos) = ctx.pointer_latest_pos() else { return };
        let ctrl = ctx.input(|i| i.modifiers.command);
        let what = match &*items {
            DragItems::Files { .. } if ctrl => "복사",
            DragItems::Files { .. } => "이동 (Ctrl: 복사)",
            DragItems::Archive { .. } => "여기에 풀기",
        };
        let text = format!("{}개 · {what}", items.count());
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("drag-ghost")));
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), egui::Color32::WHITE);
        let rect = Rect::from_min_size(pos + egui::vec2(14.0, 10.0), galley.size() + egui::vec2(16.0, 10.0));
        painter.rect_filled(rect, 6.0, ACCENT);
        painter.galley(rect.min + egui::vec2(8.0, 5.0), galley, egui::Color32::WHITE);
    }
}

impl eframe::App for Home {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);

        // 설정이 바뀌면 저장 (마지막 폴더 포함)
        if self.pane().view.is_none() {
            let d = self.pane().dir().to_path_buf();
            if self.settings.last_dir.as_deref() != Some(d.as_path()) {
                self.settings.last_dir = Some(d);
            }
        }
        if self.settings != self.saved {
            self.settings.save();
            self.saved = self.settings.clone();
        }

        // 바깥(다른 프로그램)에서 끌어다 놓기: 폴더 하나면 그 폴더로, 파일이면 그 위치로 가서 선택
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() && !self.busy() {
            let hidden = self.settings.show_hidden;
            match dropped.as_slice() {
                [one] if one.is_dir() => self.pane_mut().navigate(one.clone(), hidden),
                [one] if unpack::is_archive_name(one) => {
                    let pw = self.options().password;
                    self.pane_mut().open_archive(one.clone(), pw, hidden);
                }
                many => {
                    if let Some(parent) = many[0].parent() {
                        self.pane_mut().navigate(parent.to_path_buf(), hidden);
                        self.pane_mut().folder.select_only(many);
                    }
                }
            }
            self.update_title(&ctx);
        }
        self.keyboard(&ctx);

        self.toolbar(ui, &ctx);
        self.status_bar(ui, &ctx);
        if self.show_settings {
            self.settings_panel(ui);
        }

        // --- 창들 ---
        let frame = egui::Frame::central_panel(&ctx.global_style()).inner_margin(Margin::same(6));
        let mut outs = Vec::new();
        let split = self.panes.len() > 1;
        let can_paste = self.clipboard.is_some();
        let hidden = self.settings.show_hidden;
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            let places = crate::pane::Places { favorites: &self.favorites, drives: &self.drives };
            let rects = layout(ui.available_rect_before_wrap(), self.panes.len(), 6.0);
            for (i, (pane, rect)) in self.panes.iter_mut().zip(rects).enumerate() {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("pane", i)));
                let active = i == self.active;
                outs.push(pane.ui(&mut child, i, active, split, &mut self.password, hidden, can_paste, &places));
            }
        });

        let busy = self.busy();
        let mut close = None;
        for (i, out) in outs.into_iter().enumerate() {
            if out.activated && self.active != i {
                self.active = i;
                self.update_title(&ctx);
            }
            if out.close {
                close = Some(i);
            }
            if let Some(p) = out.go {
                self.open_path(&ctx, i, p);
            }
            if let Some((items, dest, ctrl)) = out.dropped
                && !busy
            {
                self.handle_drop(&ctx, i, &items, dest, ctrl);
            }
            if !busy {
                match out.fs {
                    FsAction::None => {}
                    FsAction::Open(p) => {
                        self.active = i;
                        self.open_path(&ctx, i, p);
                    }
                    FsAction::Up => {
                        self.panes[i].go_up(hidden);
                        self.update_title(&ctx);
                    }
                    FsAction::Command(c) => {
                        self.active = i;
                        self.command(&ctx, c);
                    }
                }
            }
            match out.view {
                ViewAction::None => {}
                ViewAction::Retry => {
                    let pw = Some(self.password.clone());
                    if let Some(v) = self.panes[i].view.as_mut() {
                        v.reload(pw);
                    }
                }
                ViewAction::Exit => {
                    self.panes[i].view = None;
                    self.update_title(&ctx);
                }
                ViewAction::Open(full) if !busy => self.open_entry(&ctx, i, full),
                ViewAction::ExtractSelected { choose } if !busy => {
                    self.active = i;
                    let to = if choose { rfd::FileDialog::new().set_title("풀 위치").pick_folder() } else { None };
                    if !choose || to.is_some() {
                        self.extract(&ctx, Mode::Here, to);
                    }
                }
                _ => {}
            }
        }
        if let Some(i) = close {
            self.close_pane(&ctx, i);
        }
        self.rename_dialog(&ctx);
        self.drag_ghost(&ctx);
    }
}
