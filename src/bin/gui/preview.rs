//! 미리보기 칸 (오른쪽) — 활성 창에서 고른 파일 하나를 보여 줍니다.
//!
//! - 사진: 줄인 그림 / 글 파일: 앞부분 (UTF-8, 아니면 CP949)
//! - 압축 파일: 형식 · 파일 수 · 풀면 크기 / 폴더: 안에 든 항목 수
//! - 그 밖: 이름 · 크기 · 수정한 날짜

use crate::icons::{self, Icon};
use crate::style;
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

const TEXT_BYTES: usize = 48 * 1024;

enum Body {
    None,
    Text(String),
    /// 압축 파일 요약 (읽는 중이면 None)
    Archive(Option<String>),
    Folder(String),
}

#[derive(Default)]
pub struct Preview {
    path: Option<PathBuf>,
    body: Option<Body>,
    rx: Option<Receiver<String>>,
}

fn is_text(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(
        ext.as_str(),
        "txt" | "md" | "log" | "csv" | "tsv" | "json" | "xml" | "yml" | "yaml" | "toml" | "ini" | "cfg" | "conf"
            | "rs" | "py" | "js" | "ts" | "html" | "htm" | "css" | "c" | "h" | "cpp" | "java" | "kt" | "go" | "sh"
            | "bat" | "ps1" | "sql" | "srt" | "smi" | "nfo"
    )
}

fn read_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(TEXT_BYTES as u64).read_to_end(&mut buf).ok()?;
    if buf.contains(&0) {
        return None;
    }
    Some(match std::str::from_utf8(&buf) {
        Ok(s) => s.to_string(),
        // 끝에서 글자가 잘렸을 수 있음
        Err(e) if e.error_len().is_none() => String::from_utf8_lossy(&buf[..e.valid_up_to()]).into_owned(),
        Err(_) => encoding_rs::EUC_KR.decode(&buf).0.into_owned(),
    })
}

impl Preview {
    fn load(&mut self, ctx: &egui::Context, path: &Path) {
        self.path = Some(path.to_path_buf());
        self.rx = None;
        self.body = Some(if path.is_dir() {
            let n = std::fs::read_dir(path).map(|r| r.count()).unwrap_or(0);
            Body::Folder(format!("항목 {n}개"))
        } else if unpack::is_archive_name(path) {
            let (tx, rx) = mpsc::channel();
            let p = path.to_path_buf();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let fmt = unpack::detect(&p).map(|f| f.to_string().to_uppercase()).unwrap_or_default();
                let text = match unpack::list(&p, &unpack::Options::default()) {
                    Ok(list) => {
                        let files = list.iter().filter(|e| !e.is_dir).count();
                        let total: u64 = list.iter().map(|e| e.size).sum();
                        let mut tops: Vec<String> = list
                            .iter()
                            .filter_map(|e| e.name.replace('\\', "/").split('/').find(|s| !s.is_empty()).map(str::to_string))
                            .collect();
                        tops.dedup();
                        tops.sort();
                        tops.dedup();
                        let shown: Vec<String> = tops.iter().take(12).cloned().collect();
                        let more = if tops.len() > 12 { format!("\n… 외 {}개", tops.len() - 12) } else { String::new() };
                        format!("{fmt} · 파일 {files}개 · 풀면 {}\n\n맨 위 항목:\n{}{more}", crate::human_size(total), shown.join("\n"))
                    }
                    Err(e) if unpack::is_password_error(&e) => format!("{fmt} · 암호가 걸려 있습니다"),
                    Err(e) => format!("{fmt} · 읽을 수 없습니다: {e:#}"),
                };
                let _ = tx.send(text);
                ctx.request_repaint();
            });
            self.rx = Some(rx);
            Body::Archive(None)
        } else if is_text(path) {
            read_text(path).map(Body::Text).unwrap_or(Body::None)
        } else {
            Body::None
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, target: Option<&Path>) {
        let Some(path) = target else {
            self.path = None;
            let rect = ui.available_rect_before_wrap();
            crate::table::empty_state(ui, rect, Icon::Preview, "미리보기", "파일 하나를 고르세요");
            return;
        };
        if self.path.as_deref() != Some(path) {
            self.load(ui.ctx(), path);
        }
        if let Some(rx) = &self.rx
            && let Ok(t) = rx.try_recv()
        {
            self.body = Some(Body::Archive(Some(t)));
            self.rx = None;
        }
        let meta = std::fs::metadata(path).ok();
        ui.horizontal(|ui| {
            let icon = if path.is_dir() {
                Icon::Folder
            } else if unpack::is_archive_name(path) {
                Icon::Archive
            } else {
                Icon::File
            };
            let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
            icons::paint(ui.painter(), r, icon, style::weak(ui));
            ui.add(egui::Label::new(RichText::new(crate::file_name(path)).strong()).wrap());
        });
        if let Some(m) = &meta {
            let size = if m.is_dir() { String::new() } else { format!("{} · ", crate::human_size(m.len())) };
            let date = m.modified().ok().map(crate::fmt_time).unwrap_or_default();
            ui.weak(format!("{size}{date}"));
        }
        ui.separator();
        if let Some(tex) = crate::thumbs::texture(ui.ctx(), path, 1024) {
            let w = ui.available_width();
            let size = tex.size_vec2();
            let h = (size.y * w / size.x).min(ui.available_height() - 10.0).max(40.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
            crate::thumbs::paint_fit(ui, rect, &tex);
            ui.weak(format!("{} × {}", size.x as u32, size.y as u32));
            return;
        }
        if crate::thumbs::is_image(path) {
            ui.weak("그림을 읽는 중…");
            return;
        }
        match &self.body {
            Some(Body::Text(t)) => {
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(t.as_str()).monospace().size(12.0)).extend());
                });
            }
            Some(Body::Archive(Some(t))) | Some(Body::Folder(t)) => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.label(t.as_str());
                });
            }
            Some(Body::Archive(None)) => {
                ui.weak("압축 파일을 읽는 중…");
            }
            _ => {
                ui.weak("미리 볼 수 없는 형식입니다");
            }
        }
    }
}
