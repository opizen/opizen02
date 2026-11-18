//! 우클릭 메뉴 "압축하기" 창.
//!
//! - `--zip` / `--7z`: 바로 압축하고, 끝나면 잠시 뒤 저절로 닫힘
//! - `--compress`: 형식·이름·암호·저장 위치를 고른 뒤 압축

use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};
use unpack::{ArchiveKind, CompressOptions, CompressStats, Level, Progress, compress_with, default_archive_path, is_cancelled};

const AUTO_CLOSE: Duration = Duration::from_millis(1200);

enum Phase {
    Ask,
    Working(Receiver<Result<CompressStats, String>>),
    Done(CompressStats, Instant),
    Failed(String),
}

pub struct PackApp {
    inputs: Vec<PathBuf>,
    kind: ArchiveKind,
    folder: PathBuf,
    /// 확장자를 뺀 파일 이름
    stem: String,
    password: String,
    password2: String,
    level: Level,
    phase: Phase,
    progress: Option<Arc<Progress>>,
}

impl PackApp {
    pub fn new(ctx: &egui::Context, inputs: Vec<PathBuf>, kind: Option<ArchiveKind>) -> Self {
        // 형식을 묻는 창은 지난번 설정(형식·수준)으로 시작
        let saved = crate::settings::Settings::load();
        let k = kind.unwrap_or(saved.kind);
        let path = default_archive_path(&inputs, k);
        let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = name.strip_suffix(k.ext()).unwrap_or(&name).to_string();
        let mut app = Self {
            inputs,
            kind: k,
            folder,
            stem,
            password: String::new(),
            password2: String::new(),
            level: saved.level,
            phase: Phase::Ask,
            progress: None,
        };
        if kind.is_some() {
            app.start(ctx);
        }
        app
    }

    fn output(&self) -> PathBuf {
        self.folder.join(format!("{}{}", self.stem.trim(), self.kind.ext()))
    }

    fn start(&mut self, ctx: &egui::Context) {
        let inputs = self.inputs.clone();
        let out = self.output();
        let kind = self.kind;
        let progress = Progress::new();
        self.progress = Some(progress.clone());
        let opts = CompressOptions {
            password: (kind.supports_password() && !self.password.is_empty()).then(|| self.password.clone()),
            level: self.level,
            progress: Some(progress),
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let r = compress_with(&inputs, &out, kind, &opts)
                .map_err(|e| if is_cancelled(&e) { "취소했습니다".to_string() } else { format!("{e:#}") });
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.phase = Phase::Working(rx);
    }

    fn summary(&self) -> String {
        match self.inputs.as_slice() {
            [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            many => format!("{}개 항목", many.len()),
        }
    }

    fn ask_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(format!("압축하기 · {}", self.summary()));
        ui.add_space(6.0);
        let mut go = false;
        egui::Grid::new("pack").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label("형식");
            ui.horizontal(|ui| {
                for k in ArchiveKind::ALL {
                    ui.selectable_value(&mut self.kind, k, k.label());
                }
            });
            ui.end_row();

            ui.label("압축 수준");
            ui.horizontal(|ui| {
                for l in Level::ALL {
                    ui.selectable_value(&mut self.level, l, l.label());
                }
            });
            ui.end_row();

            ui.label("파일 이름");
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.stem).desired_width(220.0));
                go |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.label(RichText::new(self.kind.ext()).monospace());
            });
            ui.end_row();

            ui.label("저장 위치");
            ui.horizontal(|ui| {
                let shown = self.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.folder.display().to_string());
                ui.label(shown).on_hover_text(self.folder.display().to_string());
                if ui.link("변경").clicked()
                    && let Some(dir) = rfd::FileDialog::new().set_directory(&self.folder).pick_folder()
                {
                    self.folder = dir;
                }
            });
            ui.end_row();

            ui.label("암호");
            ui.add_enabled_ui(self.kind.supports_password(), |ui| {
                ui.vertical(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.password)
                            .password(true)
                            .hint_text("없으면 비워 두세요")
                            .desired_width(220.0),
                    );
                    if !self.password.is_empty() {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.password2)
                                .password(true)
                                .hint_text("암호 확인")
                                .desired_width(220.0),
                        );
                    }
                });
            });
            ui.end_row();
        });

        let out = self.output();
        let pw_ok = self.password.is_empty() || !self.kind.supports_password() || self.password == self.password2;
        let exists = out.exists();
        let valid = !self.stem.trim().is_empty() && !self.stem.contains(['/', '\\']);
        if !pw_ok {
            ui.colored_label(crate::style::WARN, "암호가 서로 다릅니다");
        } else if exists {
            ui.colored_label(crate::style::WARN, "같은 이름의 파일이 이미 있습니다");
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let can = pw_ok && !exists && valid;
            if ui.add_enabled(can, crate::style::primary("압축")).clicked() || (go && can) {
                self.start(ui.ctx());
            }
            if ui.add(crate::style::secondary("취소")).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

impl eframe::App for PackApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Phase::Working(rx) = &self.phase
            && let Ok(r) = rx.try_recv()
        {
            self.phase = match r {
                Ok(s) => Phase::Done(s, Instant::now()),
                Err(e) => Phase::Failed(e),
            };
        }

        egui::CentralPanel::default().frame(crate::style::panel(&ctx)).show(ui, |ui| match &self.phase {
            Phase::Ask => self.ask_ui(ui),
            Phase::Working(_) => {
                ui.strong(format!("{} 압축 중…", self.kind.label()));
                ui.add_space(6.0);
                ui.label(format!("{} → {}", self.summary(), self.output().file_name().unwrap_or_default().to_string_lossy()));
                if let Some(p) = self.progress.clone() {
                    let f = p.fraction().unwrap_or(0.0);
                    ui.add(egui::ProgressBar::new(f).desired_height(6.0));
                    ui.horizontal(|ui| {
                        let name: String = p.current().chars().take(40).collect();
                        ui.weak(format!("{}%  {name}", (f * 100.0) as u32));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let label = if p.is_cancelled() { "취소하는 중…" } else { "취소" };
                            if ui.add_enabled(!p.is_cancelled(), egui::Button::new(label).small()).clicked() {
                                p.cancel();
                            }
                        });
                    });
                }
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Phase::Done(s, at) => {
                let size = std::fs::metadata(&s.output).map(|m| m.len()).unwrap_or(0);
                ui.horizontal(|ui| {
                    ui.colored_label(crate::style::OK, "✔");
                    ui.strong(s.output.file_name().unwrap_or_default().to_string_lossy());
                });
                ui.label(format!(
                    "파일 {}개 · 폴더 {}개 · {} → {}",
                    s.files,
                    s.dirs,
                    crate::human_size(s.bytes),
                    crate::human_size(size)
                ));
                for w in &s.warnings {
                    ui.colored_label(crate::style::WARN, w);
                }
                ui.add_space(4.0);
                let out = s.output.clone();
                let clean = s.warnings.is_empty();
                let at = *at;
                ui.horizontal(|ui| {
                    if ui.button("폴더에서 보기").clicked() {
                        crate::open_in_explorer(&out);
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("닫기").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if clean {
                        ui.weak("완료 — 곧 닫힙니다");
                    }
                });
                if clean {
                    let left = AUTO_CLOSE.saturating_sub(at.elapsed());
                    if left.is_zero() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        ctx.request_repaint_after(left);
                    }
                }
            }
            Phase::Failed(e) => {
                ui.colored_label(crate::style::ERR, "✖ 압축하지 못했습니다");
                ui.label(e);
                if ui.button("닫기").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        });
    }
}
