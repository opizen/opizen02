//! 탐색기 우클릭 메뉴("여기에 압축 풀기" / "새 폴더에 압축 풀기")에서 쓰는 작은 진행 창.
//!
//! 해제가 모두 성공하면 잠시 뒤 저절로 닫히고, 실패하거나 경고가 있으면 열린 채로
//! 이유를 보여 줍니다. 암호가 필요한 파일은 이 창에서 바로 암호를 입력해 다시 시도합니다.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};
use unpack::{Mode, Options, extract_auto, is_password_error};

/// 성공 후 창이 닫히기까지 기다리는 시간
const AUTO_CLOSE: Duration = Duration::from_millis(1200);

enum State {
    Waiting,
    Working,
    Done { output: PathBuf, warnings: Vec<String> },
    NeedPassword { wrong: bool },
    Failed(String),
}

struct Job {
    path: PathBuf,
    state: State,
}

enum Msg {
    Started(usize),
    Finished(usize, State),
    AllDone,
}

pub struct QuickApp {
    mode: Mode,
    jobs: Vec<Job>,
    password: String,
    worker: Option<Receiver<Msg>>,
    finished_at: Option<Instant>,
    focus_password: bool,
}

impl QuickApp {
    pub fn new(ctx: &egui::Context, mode: Mode, files: Vec<PathBuf>) -> Self {
        let mut app = Self {
            mode,
            jobs: files.into_iter().map(|path| Job { path, state: State::Waiting }).collect(),
            password: String::new(),
            worker: None,
            finished_at: None,
            focus_password: false,
        };
        app.run(ctx, None);
        app
    }

    /// 대기 중이거나 암호가 필요했던 항목을 해제합니다.
    fn run(&mut self, ctx: &egui::Context, password: Option<String>) {
        let todo: Vec<(usize, PathBuf)> = self
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, j)| matches!(j.state, State::Waiting | State::NeedPassword { .. }))
            .map(|(i, j)| (i, j.path.clone()))
            .collect();
        if todo.is_empty() {
            return;
        }
        let opts = Options { password, ..Default::default() };
        let mode = self.mode;
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            for (i, path) in todo {
                let _ = tx.send(Msg::Started(i));
                ctx.request_repaint();
                let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
                let state = match extract_auto(&path, &parent, mode, &opts) {
                    Ok(s) => State::Done { output: s.output, warnings: s.warnings },
                    Err(e) if is_password_error(&e) => State::NeedPassword { wrong: opts.password.is_some() },
                    Err(e) => State::Failed(format!("{e:#}")),
                };
                let _ = tx.send(Msg::Finished(i, state));
                ctx.request_repaint();
            }
            let _ = tx.send(Msg::AllDone);
            ctx.request_repaint();
        });
        self.worker = Some(rx);
        self.finished_at = None;
    }

    fn poll(&mut self) {
        let Some(rx) = &self.worker else { return };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Started(i) => self.jobs[i].state = State::Working,
                Msg::Finished(i, state) => self.jobs[i].state = state,
                Msg::AllDone => {
                    self.worker = None;
                    self.finished_at = Some(Instant::now());
                    self.focus_password = true;
                    break;
                }
            }
        }
    }

    fn all_clean(&self) -> bool {
        self.jobs.iter().all(|j| matches!(&j.state, State::Done { warnings, .. } if warnings.is_empty()))
    }

    fn needs_password(&self) -> bool {
        self.jobs.iter().any(|j| matches!(j.state, State::NeedPassword { .. }))
    }

    fn last_output(&self) -> Option<&Path> {
        self.jobs.iter().rev().find_map(|j| match &j.state {
            State::Done { output, .. } => Some(output.as_path()),
            _ => None,
        })
    }
}

impl eframe::App for QuickApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll();

        // 모두 문제없이 끝났으면 잠시 뒤 닫기
        if let Some(t) = self.finished_at
            && self.all_clean()
        {
            let left = AUTO_CLOSE.saturating_sub(t.elapsed());
            if left.is_zero() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(left);
            }
        }

        egui::CentralPanel::default().frame(crate::style::panel(&ctx)).show(ui, |ui| {
            let total = self.jobs.len();
            let done = self
                .jobs
                .iter()
                .filter(|j| !matches!(j.state, State::Waiting | State::Working))
                .count();
            let title = match self.mode {
                Mode::Smart => "여기에 압축 풀기",
                Mode::NewFolder => "새 폴더에 압축 풀기",
            };
            ui.horizontal(|ui| {
                ui.strong(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{done} / {total}"));
                });
            });
            ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).animate(self.worker.is_some()).desired_height(6.0));
            ui.add_space(4.0);

            egui::ScrollArea::vertical().max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
                for job in &self.jobs {
                    let name = job.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.horizontal_wrapped(|ui| {
                        match &job.state {
                            State::Waiting => {
                                ui.weak("대기");
                            }
                            State::Working => {
                                ui.spinner();
                            }
                            State::Done { .. } => {
                                ui.colored_label(crate::style::OK, "✔");
                            }
                            State::NeedPassword { .. } => {
                                ui.colored_label(crate::style::WARN, "🔒");
                            }
                            State::Failed(_) => {
                                ui.colored_label(crate::style::ERR, "✖");
                            }
                        }
                        ui.label(&name);
                        match &job.state {
                            State::Done { output, warnings } => {
                                let shown = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                ui.weak(format!("→ {shown}")).on_hover_text(output.display().to_string());
                                if !warnings.is_empty() {
                                    ui.colored_label(crate::style::WARN, format!("경고 {}개", warnings.len()))
                                        .on_hover_text(warnings.join("\n"));
                                }
                            }
                            State::NeedPassword { wrong: true } => {
                                ui.colored_label(crate::style::WARN, "암호가 올바르지 않습니다");
                            }
                            State::NeedPassword { wrong: false } => {
                                ui.colored_label(crate::style::WARN, "암호가 필요합니다");
                            }
                            State::Failed(e) => {
                                ui.colored_label(crate::style::ERR, e);
                            }
                            _ => {}
                        }
                    });
                }
            });

            if self.worker.is_some() {
                return;
            }
            ui.separator();

            if self.needs_password() {
                let mut submit = false;
                ui.horizontal(|ui| {
                    ui.label("암호");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.password).password(true).desired_width(200.0),
                    );
                    if std::mem::take(&mut self.focus_password) {
                        resp.request_focus();
                    }
                    submit |= resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    submit |= ui
                        .add_enabled(!self.password.is_empty(), crate::style::primary("다시 시도").min_size(egui::vec2(90.0, 30.0)))
                        .clicked();
                });
                if submit && !self.password.is_empty() {
                    let pw = self.password.clone();
                    self.run(&ctx, Some(pw));
                }
            }

            ui.horizontal(|ui| {
                if let Some(out) = self.last_output().map(Path::to_path_buf)
                    && ui.button("결과 폴더 열기").clicked()
                {
                    crate::open_in_explorer(&out);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.button("닫기").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if self.all_clean() {
                    ui.weak("완료 — 곧 닫힙니다");
                }
            });
        });
    }
}
