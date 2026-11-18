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

/// 분할 크기 고르기 (나누지 않음 · 100 MB · 700 MB · 4 GB · 직접 입력). `text`는 `700m` 같은 글자, 비면 나누지 않음.
pub fn split_ui(ui: &mut egui::Ui, id: &str, text: &mut String) {
    const PRESETS: [(&str, &str); 4] = [("", "나누지 않음"), ("100m", "100 MB"), ("700m", "700 MB (CD)"), ("4000m", "4 GB (FAT32·USB)")];
    let custom = !PRESETS.iter().any(|(v, _)| *v == text.as_str());
    let shown = if custom { "직접 입력".to_string() } else { PRESETS.iter().find(|(v, _)| *v == text.as_str()).map(|p| p.1).unwrap_or("").to_string() };
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(id).selected_text(shown).show_ui(ui, |ui| {
            for (v, label) in PRESETS {
                if ui.selectable_label(!custom && text == v, label).clicked() {
                    *text = v.to_string();
                }
            }
            if ui.selectable_label(custom, "직접 입력").clicked() && !custom {
                *text = "50m".into();
            }
        });
        if custom {
            ui.add(egui::TextEdit::singleline(text).desired_width(70.0).hint_text("50m"));
            if unpack::parse_size(text).is_none() {
                ui.colored_label(crate::style::WARN, "예: 50m, 2g");
            }
        }
    });
}

/// 분할 크기 글자 → 바이트 (비었거나 잘못되면 None)
pub fn split_bytes(text: &str) -> Option<u64> {
    if text.trim().is_empty() { None } else { unpack::parse_size(text) }
}

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
    always_compress: bool,
    split: String,
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
            always_compress: saved.always_compress,
            split: saved.split.clone(),
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
            always_compress: self.always_compress,
            split: split_bytes(&self.split),
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

            ui.label("분할");
            split_ui(ui, "pack_split", &mut self.split);
            ui.end_row();

            ui.label("");
            ui.checkbox(&mut self.always_compress, "이미 압축된 파일(jpg·mp4·zip …)도 다시 압축")
                .on_hover_text("끄면 압축해도 줄지 않는 파일은 그대로 넣어 훨씬 빠릅니다");
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
        let exists = out.exists() || (split_bytes(&self.split).is_some() && out.with_file_name(format!("{}.001", out.file_name().unwrap_or_default().to_string_lossy())).exists());
        let split_ok = self.split.trim().is_empty() || unpack::parse_size(&self.split).is_some();
        let valid = !self.stem.trim().is_empty() && !self.stem.contains(['/', '\\']);
        if !pw_ok {
            ui.colored_label(crate::style::WARN, "암호가 서로 다릅니다");
        } else if exists {
            ui.colored_label(crate::style::WARN, "같은 이름의 파일이 이미 있습니다");
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let can = pw_ok && !exists && valid && split_ok;
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
                let size = s.size;
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
                if s.parts > 0 || s.stored > 0 {
                    let mut t = Vec::new();
                    if s.parts > 0 {
                        t.push(format!("{}조각으로 나눔 (.001 ~ .{:03})", s.parts, s.parts));
                    }
                    if s.stored > 0 {
                        t.push(format!("이미 압축된 파일 {}개는 그대로 넣음", s.stored));
                    }
                    ui.weak(t.join(" · "));
                }
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
