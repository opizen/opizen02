//! opizenAI_Unpack GUI — 압축 풀기 / 압축하기 데스크톱 앱 (Windows, Linux).
//!
//! 명령줄 인자로 파일 경로를 받으면 목록에 추가합니다.
//!
//! 파일 관리자 우클릭 메뉴에서 쓰는 옵션:
//! - `--here <파일>…`     : "여기에 압축 풀기" (스마트 해제, 작은 진행 창)
//! - `--folder <파일>…`   : "새 폴더에 압축 풀기"
//! - `--auto <파일>…`     : 전체 창을 열고 바로 해제 시작
//! - `--zip <항목>…`      : "ZIP으로 압축" (선택한 항목 전체를 하나로)
//! - `--7z <항목>…`       : "7Z로 압축"
//! - `--compress <항목>…` : "압축하기…" (형식·이름·암호 선택)

#![cfg_attr(windows, windows_subsystem = "windows")]

mod gather;
mod pack;
mod quick;

use eframe::egui::{self, Color32, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use unpack::{ArchiveKind, EntryInfo, Mode, Options, detect, extract_into, list};

const EXTENSIONS: &[&str] = &[
    "zip", "jar", "7z", "tar", "gz", "tgz", "bz2", "tbz2", "tbz", "xz", "txz", "zst", "tzst",
];

enum Launch {
    Main { auto: bool },
    Extract(Mode),
    /// `None`이면 형식을 묻는 창
    Pack(Option<ArchiveKind>),
}

fn main() -> eframe::Result {
    let mut launch = Launch::Main { auto: false };
    let mut files = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("--auto") => launch = Launch::Main { auto: true },
            Some("--here") => launch = Launch::Extract(Mode::Smart),
            Some("--folder") => launch = Launch::Extract(Mode::NewFolder),
            Some("--zip") => launch = Launch::Pack(Some(ArchiveKind::Zip)),
            Some("--7z") => launch = Launch::Pack(Some(ArchiveKind::SevenZ)),
            Some("--compress") => launch = Launch::Pack(None),
            _ => {
                let p = PathBuf::from(arg);
                files.push(std::path::absolute(&p).unwrap_or(p));
            }
        }
    }

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/unpack.png")).ok();
    let small_window = |size: [f32; 2]| {
        let mut v = egui::ViewportBuilder::default()
            .with_title(unpack::APP_NAME)
            .with_inner_size(size)
            .with_min_inner_size([360.0, 140.0])
            .with_always_on_top();
        if let Some(icon) = icon.clone() {
            v = v.with_icon(icon);
        }
        eframe::NativeOptions { viewport: v, centered: true, ..Default::default() }
    };

    match launch {
        Launch::Extract(mode) if !files.is_empty() => {
            let tag = if mode == Mode::Smart { "HERE" } else { "FOLDER" };
            let Some(files) = gather::gather(tag, files) else { return Ok(()) };
            let height = (150.0 + 26.0 * files.len().min(7) as f32).min(330.0);
            return eframe::run_native(
                unpack::APP_NAME,
                small_window([480.0, height]),
                Box::new(move |cc| {
                    install_fonts(&cc.egui_ctx);
                    Ok(Box::new(quick::QuickApp::new(&cc.egui_ctx, mode, files)))
                }),
            );
        }
        Launch::Pack(kind) if !files.is_empty() => {
            let tag = kind.map(|k| k.label()).unwrap_or("ASK");
            // 다른 프로세스가 모아서 처리하면 이 프로세스는 조용히 끝냄
            let Some(files) = gather::gather(tag, files) else { return Ok(()) };
            let size = if kind.is_some() { [460.0, 170.0] } else { [540.0, 220.0] };
            return eframe::run_native(
                unpack::APP_NAME,
                small_window(size),
                Box::new(move |cc| {
                    install_fonts(&cc.egui_ctx);
                    Ok(Box::new(pack::PackApp::new(&cc.egui_ctx, files, kind)))
                }),
            );
        }
        _ => {}
    }
    let auto = matches!(launch, Launch::Main { auto: true });

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("{} — 압축 해제", unpack::APP_NAME))
        .with_inner_size([720.0, 560.0])
        .with_min_inner_size([520.0, 420.0])
        .with_drag_and_drop(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }

    eframe::run_native(
        unpack::APP_NAME,
        eframe::NativeOptions { viewport, ..Default::default() },
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            let mut app = App::default();
            app.add_files(files);
            if auto && !app.items.is_empty() {
                app.open_when_done = true;
                app.start(&cc.egui_ctx);
            }
            Ok(Box::new(app))
        }),
    )
}

/// Windows의 맑은 고딕 등 한글 글꼴을 우선 사용합니다.
fn install_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\malgun.ttf",
        r"C:\Windows\Fonts\gulim.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    ];
    if let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) {
        use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
        ctx.add_font(FontInsert::new(
            "korean",
            egui::FontData::from_owned(bytes),
            vec![
                InsertFontFamily { family: egui::FontFamily::Proportional, priority: FontPriority::Highest },
                InsertFontFamily { family: egui::FontFamily::Monospace, priority: FontPriority::Lowest },
            ],
        ));
    }
}

#[derive(Clone, PartialEq)]
enum Status {
    Waiting,
    Working,
    Done(String),
    Failed(String),
}

struct Item {
    path: PathBuf,
    format: String,
    status: Status,
    output: Option<PathBuf>,
}

enum Msg {
    Started(usize),
    Done(usize, unpack::Stats),
    Failed(usize, String),
    Finished,
}

#[derive(PartialEq)]
enum OutputMode {
    BesideArchive,
    Folder,
}

struct App {
    items: Vec<Item>,
    output_mode: OutputMode,
    output_dir: Option<PathBuf>,
    overwrite: bool,
    open_when_done: bool,
    password: String,
    show_password: bool,
    worker: Option<Receiver<Msg>>,
    log: Vec<(Color32, String)>,
    viewer: Option<(PathBuf, Result<Vec<EntryInfo>, String>)>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            output_mode: OutputMode::BesideArchive,
            output_dir: None,
            overwrite: false,
            open_when_done: false,
            password: String::new(),
            show_password: false,
            worker: None,
            log: Vec::new(),
            viewer: None,
        }
    }
}

impl App {
    fn busy(&self) -> bool {
        self.worker.is_some()
    }

    fn add_files(&mut self, files: impl IntoIterator<Item = PathBuf>) {
        for path in files {
            if path.is_dir() || self.items.iter().any(|i| i.path == path) {
                continue;
            }
            match detect(&path) {
                Ok(f) => self.items.push(Item {
                    path,
                    format: f.to_string(),
                    status: Status::Waiting,
                    output: None,
                }),
                Err(e) => self.log.push((Color32::LIGHT_RED, format!("{}: {e:#}", path.display()))),
            }
        }
    }

    fn options(&self) -> Options {
        Options {
            overwrite: self.overwrite,
            password: (!self.password.is_empty()).then(|| self.password.clone()),
            verbose: false,
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        let jobs: Vec<(usize, PathBuf, PathBuf)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| !matches!(it.status, Status::Done(_)))
            .map(|(i, it)| {
                let parent = match (&self.output_mode, &self.output_dir) {
                    (OutputMode::Folder, Some(dir)) => dir.clone(),
                    _ => it.path.parent().map(Path::to_path_buf).unwrap_or_default(),
                };
                (i, it.path.clone(), parent)
            })
            .collect();
        if jobs.is_empty() {
            return;
        }
        for (i, _, _) in &jobs {
            self.items[*i].status = Status::Waiting;
        }
        let opts = self.options();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            for (i, path, parent) in jobs {
                let _ = tx.send(Msg::Started(i));
                ctx.request_repaint();
                let msg = match extract_into(&path, &parent, &opts) {
                    Ok(stats) => Msg::Done(i, stats),
                    Err(e) => Msg::Failed(i, format!("{e:#}")),
                };
                let _ = tx.send(msg);
                ctx.request_repaint();
            }
            let _ = tx.send(Msg::Finished);
            ctx.request_repaint();
        });
        self.worker = Some(rx);
    }

    fn poll_worker(&mut self) {
        let Some(rx) = &self.worker else { return };
        let mut finished = false;
        let mut last_output = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Started(i) => self.items[i].status = Status::Working,
                Msg::Done(i, s) => {
                    let name = file_name(&self.items[i].path);
                    for w in &s.warnings {
                        self.log.push((Color32::YELLOW, format!("{name}: {w}")));
                    }
                    let mut summary = format!("파일 {}개 · {}", s.files, human_size(s.bytes));
                    if s.skipped > 0 {
                        summary += &format!(" · 건너뜀 {}", s.skipped);
                    }
                    self.log.push((
                        Color32::LIGHT_GREEN,
                        format!("{name} → {} ({summary})", s.output.display()),
                    ));
                    self.items[i].status = Status::Done(summary);
                    self.items[i].output = Some(s.output.clone());
                    last_output = Some(s.output);
                }
                Msg::Failed(i, e) => {
                    let name = file_name(&self.items[i].path);
                    self.log.push((Color32::LIGHT_RED, format!("{name}: {e}")));
                    self.items[i].status = Status::Failed(e);
                }
                Msg::Finished => finished = true,
            }
        }
        if finished {
            self.worker = None;
            if self.open_when_done
                && let Some(out) = last_output
            {
                open_in_explorer(&out);
            }
        }
    }

    fn pick_files(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .set_title("압축 파일 선택")
            .add_filter("압축 파일", EXTENSIONS)
            .add_filter("모든 파일", &["*"])
            .pick_files()
        {
            self.add_files(files);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_worker();

        // 끌어다 놓은 파일 추가
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() && !self.busy() {
            self.add_files(dropped);
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        egui::Panel::bottom("actions").show(ui, |ui| {
            ui.add_space(6.0);
            let total = self.items.len();
            let done = self
                .items
                .iter()
                .filter(|i| matches!(i.status, Status::Done(_) | Status::Failed(_)))
                .count();
            if self.busy() {
                ui.add(
                    egui::ProgressBar::new(if total == 0 { 0.0 } else { done as f32 / total as f32 })
                        .text(format!("{done} / {total} 처리 중…"))
                        .animate(true),
                );
            }
            ui.horizontal(|ui| {
                let can_start = !self.busy()
                    && self.items.iter().any(|i| !matches!(i.status, Status::Done(_)))
                    && (self.output_mode == OutputMode::BesideArchive || self.output_dir.is_some());
                let btn = egui::Button::new(RichText::new("  압축 풀기  ").size(18.0).strong());
                if ui.add_enabled(can_start, btn).clicked() {
                    self.start(&ctx);
                }
                if let Some(out) = self.items.iter().rev().find_map(|i| i.output.clone())
                    && ui.button("결과 폴더 열기").clicked()
                {
                    open_in_explorer(&out);
                }
            });
            if !self.log.is_empty() {
                egui::CollapsingHeader::new(format!("기록 ({})", self.log.len()))
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(120.0).stick_to_bottom(true).show(ui, |ui| {
                            for (c, line) in &self.log {
                                ui.colored_label(*c, line);
                            }
                        });
                    });
            }
            ui.add_space(6.0);
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.heading(format!("{} 압축 해제", unpack::APP_NAME));
            ui.label("zip · 7z · tar · tar.gz · tar.bz2 · tar.xz · tar.zst · gz · bz2 · xz · zst");
            ui.add_space(8.0);

            // 파일 목록 / 드롭 영역
            let stroke = if hovering {
                egui::Stroke::new(2.0, ui.visuals().selection.bg_fill)
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke
            };
            egui::Frame::group(ui.style()).stroke(stroke).show(ui, |ui| {
                ui.set_min_height(180.0);
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!self.busy(), |ui| {
                        if ui.button("➕ 파일 추가…").clicked() {
                            self.pick_files();
                        }
                        if ui.button("목록 비우기").clicked() {
                            self.items.clear();
                        }
                    });
                });
                ui.separator();
                if self.items.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("여기에 압축 파일을 끌어다 놓으세요").size(16.0).weak());
                    });
                    return;
                }
                let mut remove = None;
                let mut view = None;
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    egui::Grid::new("files").striped(true).num_columns(4).spacing([12.0, 6.0]).show(
                        ui,
                        |ui| {
                            for (i, it) in self.items.iter().enumerate() {
                                ui.label(file_name(&it.path)).on_hover_text(it.path.display().to_string());
                                ui.label(RichText::new(&it.format).monospace());
                                match &it.status {
                                    Status::Waiting => ui.label("대기"),
                                    Status::Working => ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label("해제 중")
                                    }).inner,
                                    Status::Done(s) => ui.colored_label(Color32::LIGHT_GREEN, format!("✔ {s}")),
                                    Status::Failed(e) => ui
                                        .colored_label(Color32::LIGHT_RED, "✖ 실패")
                                        .on_hover_text(e),
                                };
                                ui.horizontal(|ui| {
                                    if ui.small_button("내용").clicked() {
                                        view = Some(i);
                                    }
                                    if ui.add_enabled(!self.busy(), egui::Button::new("✕").small()).clicked() {
                                        remove = Some(i);
                                    }
                                });
                                ui.end_row();
                            }
                        },
                    );
                });
                if let Some(i) = view {
                    let path = self.items[i].path.clone();
                    let entries = list(&path, &self.options()).map_err(|e| format!("{e:#}"));
                    self.viewer = Some((path, entries));
                }
                if let Some(i) = remove {
                    self.items.remove(i);
                }
            });

            ui.add_space(10.0);
            ui.add_enabled_ui(!self.busy(), |ui| {
                egui::Grid::new("opts").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("출력 위치");
                    ui.vertical(|ui| {
                        ui.radio_value(&mut self.output_mode, OutputMode::BesideArchive, "압축 파일과 같은 위치");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.output_mode, OutputMode::Folder, "지정한 폴더:");
                            let label = self
                                .output_dir
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "(선택 안 됨)".into());
                            if ui.button("찾아보기…").clicked()
                                && let Some(dir) = rfd::FileDialog::new().set_title("출력 폴더 선택").pick_folder()
                            {
                                self.output_dir = Some(dir);
                                self.output_mode = OutputMode::Folder;
                            }
                            ui.label(label);
                        });
                    });
                    ui.end_row();

                    ui.label("암호");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.password)
                                .password(!self.show_password)
                                .hint_text("zip / 7z 암호 (없으면 비워 두세요)")
                                .desired_width(240.0),
                        );
                        ui.checkbox(&mut self.show_password, "표시");
                    });
                    ui.end_row();

                    ui.label("옵션");
                    ui.vertical(|ui| {
                        ui.checkbox(&mut self.overwrite, "이미 있는 파일 덮어쓰기");
                        ui.checkbox(&mut self.open_when_done, "완료 후 결과 폴더 열기");
                    });
                    ui.end_row();
                });
            });
        });

        // 내용 보기 창
        let mut close = false;
        if let Some((path, entries)) = &self.viewer {
            let mut open = true;
            egui::Window::new(format!("내용 — {}", file_name(path)))
                .open(&mut open)
                .default_size([480.0, 360.0])
                .show(&ctx, |ui| match entries {
                    Err(e) => {
                        ui.colored_label(Color32::LIGHT_RED, e);
                    }
                    Ok(list) => {
                        ui.label(format!("{}개 항목", list.len()));
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            egui::Grid::new("entries").striped(true).num_columns(2).show(ui, |ui| {
                                for e in list {
                                    if e.is_dir {
                                        ui.label(RichText::new("<폴더>").weak());
                                    } else {
                                        ui.label(RichText::new(human_size(e.size)).monospace());
                                    }
                                    ui.label(&e.name);
                                    ui.end_row();
                                }
                            });
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

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn human_size(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{b} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

fn open_in_explorer(path: &Path) {
    #[cfg(windows)]
    let cmd = if path.is_file() {
        std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn()
    } else {
        std::process::Command::new("explorer").arg(path).spawn()
    };
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = std::process::Command::new("xdg-open")
        .arg(if path.is_file() { path.parent().unwrap_or(path) } else { path })
        .spawn();
    let _ = cmd;
}
