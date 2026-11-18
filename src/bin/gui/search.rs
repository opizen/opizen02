//! 검색 — 지금 폴더와 그 아래 모든 폴더에서 이름으로 찾습니다 (압축 파일 안도).
//!
//! 띄어쓰기로 나눈 낱말이 모두 들어 있는 이름을 찾고(대소문자 구분 없음), 결과는 찾는 대로 늘어납니다.

use crate::dnd::DragItems;
use crate::icons::Icon;
use crate::table::{self, Col};
use eframe::egui;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

const MAX_HITS: usize = 5000;
/// 압축 파일 안을 찾아볼 최대 크기 (zip·7z·rar는 목록만 읽어 빠르지만 tar.*는 끝까지 읽어야 함)
const MAX_STREAM_ARCHIVE: u64 = 200 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Hit {
    /// 찾은 파일·폴더 (압축 파일 안이면 그 압축 파일)
    pub path: PathBuf,
    /// 압축 파일 안 경로 (`폴더/이름`)
    pub inner: Option<String>,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

pub enum SearchAction {
    None,
    /// 검색 닫기
    Close,
    /// 폴더 들어가기 / 파일 열기 / 압축 파일 보기 (Home의 열기와 같음)
    Open(PathBuf),
    /// 압축 파일 안 항목: 그 압축 파일을 열고 그 폴더에서 선택
    OpenInner(PathBuf, String),
    /// 들어 있는 폴더를 열고 선택
    Reveal(PathBuf),
}

pub struct SearchView {
    pub query: String,
    pub root: PathBuf,
    pub hits: Vec<Hit>,
    rx: Option<Receiver<Vec<Hit>>>,
    stop: Arc<AtomicBool>,
    scanned: Arc<AtomicUsize>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    sort: table::Sort,
}

impl Drop for SearchView {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn matches(name: &str, words: &[String]) -> bool {
    let n = name.to_lowercase();
    words.iter().all(|w| n.contains(w.as_str()))
}

impl SearchView {
    pub fn start(ctx: &egui::Context, root: PathBuf, query: String, show_hidden: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let scanned = Arc::new(AtomicUsize::new(0));
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        {
            let (stop, scanned, root, ctx) = (stop.clone(), scanned.clone(), root.clone(), ctx.clone());
            std::thread::spawn(move || {
                let mut stack = vec![root];
                let mut found = 0;
                let mut batch = Vec::new();
                let mut last = std::time::Instant::now();
                while let Some(dir) = stack.pop() {
                    let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                    let mut entries: Vec<_> = rd.flatten().collect();
                    entries.sort_by_key(|e| e.file_name());
                    for e in entries.into_iter().rev() {
                        if stop.load(Ordering::Relaxed) || found >= MAX_HITS {
                            let _ = tx.send(batch);
                            ctx.request_repaint();
                            return;
                        }
                        let name = e.file_name().to_string_lossy().into_owned();
                        if !show_hidden && name.starts_with('.') {
                            continue;
                        }
                        scanned.fetch_add(1, Ordering::Relaxed);
                        let path = e.path();
                        let Ok(ft) = e.file_type() else { continue };
                        let meta = e.metadata().ok();
                        if ft.is_dir() {
                            stack.push(path.clone());
                        }
                        if matches(&name, &words) {
                            found += 1;
                            batch.push(Hit {
                                path: path.clone(),
                                inner: None,
                                name: name.clone(),
                                is_dir: ft.is_dir(),
                                size: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()).unwrap_or(0),
                                modified: meta.as_ref().and_then(|m| m.modified().ok()),
                            });
                        }
                        // 압축 파일 안 (목록만 읽음)
                        if ft.is_file() && unpack::is_archive_name(&path) {
                            let fmt = unpack::detect(&path).ok();
                            let cheap = matches!(fmt, Some(unpack::Format::Zip | unpack::Format::SevenZ | unpack::Format::Rar));
                            let small = meta.as_ref().is_some_and(|m| m.len() <= MAX_STREAM_ARCHIVE);
                            if (cheap || small)
                                && let Ok(list) = unpack::list(&path, &unpack::Options::default())
                            {
                                for x in list {
                                    let full = x.name.replace('\\', "/");
                                    let full = full.trim_end_matches('/');
                                    let base = full.rsplit('/').next().unwrap_or(full);
                                    if matches(base, &words) && found < MAX_HITS {
                                        found += 1;
                                        batch.push(Hit {
                                            path: path.clone(),
                                            inner: Some(full.to_string()),
                                            name: base.to_string(),
                                            is_dir: x.is_dir,
                                            size: x.size,
                                            modified: x.modified,
                                        });
                                    }
                                }
                            }
                        }
                        if !batch.is_empty() && last.elapsed().as_millis() > 150 {
                            let _ = tx.send(std::mem::take(&mut batch));
                            ctx.request_repaint();
                            last = std::time::Instant::now();
                        }
                    }
                }
                let _ = tx.send(batch);
                ctx.request_repaint();
            });
        }
        Self { query, root, hits: Vec::new(), rx: Some(rx), stop, scanned, selected: BTreeSet::new(), anchor: None, sort: (0, true) }
    }

    pub fn running(&self) -> bool {
        self.rx.is_some()
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        loop {
            match rx.try_recv() {
                Ok(v) => self.hits.extend(v),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    break;
                }
            }
        }
    }

    pub fn status(&self) -> String {
        let more = if self.hits.len() >= MAX_HITS { format!(" (처음 {MAX_HITS}개만)") } else { String::new() };
        let state = if self.running() {
            format!(" · 찾는 중… {}개 살펴봄", self.scanned.load(Ordering::Relaxed))
        } else {
            String::new()
        };
        let sel = if self.selected.is_empty() { String::new() } else { format!(" · {}개 선택", self.selected.len()) };
        format!("‘{}’ 검색 결과 {}개{more}{state}{sel}", self.query, self.hits.len())
    }

    /// 고른 결과 중 실제 파일·폴더 (압축 파일 안 항목은 빼고)
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected.iter().filter_map(|&i| self.hits.get(i)).filter(|h| h.inner.is_none()).map(|h| h.path.clone()).collect()
    }

    fn location(&self, h: &Hit) -> String {
        let rel = |p: &Path| p.strip_prefix(&self.root).map(|r| r.display().to_string()).unwrap_or_else(|_| p.display().to_string());
        match &h.inner {
            None => h.path.parent().map(rel).unwrap_or_default(),
            Some(inner) => {
                let folder = inner.rsplit_once('/').map(|(d, _)| format!(" › {d}")).unwrap_or_default();
                format!("{}{folder}", rel(&h.path))
            }
        }
    }

    pub fn table_ui(&mut self, ui: &mut egui::Ui, pane: usize, focused: bool) -> SearchAction {
        self.poll();
        if self.running() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        let mut action = SearchAction::None;
        let all = [
            Col { id: 0, title: "이름", width: 0.0, right: false },
            Col { id: 1, title: "위치", width: 180.0, right: false },
            Col { id: 2, title: "수정한 날짜", width: 130.0, right: false },
            Col { id: 3, title: "크기", width: 80.0, right: true },
        ];
        let w = ui.available_width();
        let shown: Vec<usize> = if w >= 620.0 { vec![0, 1, 2, 3] } else if w >= 420.0 { vec![0, 1, 3] } else { vec![0, 1] };
        let cols: Vec<Col> = shown.iter().map(|&i| Col { width: if i == 1 && w < 420.0 { 130.0 } else { all[i].width }, ..all[i] }).collect();
        let before = self.sort;
        table::header(ui, &cols, &mut self.sort);
        if before != self.sort {
            let (c, asc) = self.sort;
            let loc: Vec<String> = self.hits.iter().map(|h| self.location(h)).collect();
            let mut idx: Vec<usize> = (0..self.hits.len()).collect();
            idx.sort_by(|&a, &b| {
                let (x, y) = (&self.hits[a], &self.hits[b]);
                let o = match c {
                    1 => loc[a].cmp(&loc[b]),
                    2 => x.modified.cmp(&y.modified),
                    3 => x.size.cmp(&y.size),
                    _ => x.name.to_lowercase().cmp(&y.name.to_lowercase()),
                };
                if asc { o } else { o.reverse() }
            });
            self.hits = idx.into_iter().map(|i| self.hits[i].clone()).collect();
            self.selected.clear();
        }
        let mods = ui.input(|i| i.modifiers);
        let mut clicked = None;
        let mut start_drag = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, table::ROW_H, self.hits.len().max(1), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if self.hits.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| ui.weak(if self.running() { "찾는 중…" } else { "찾은 것이 없습니다" }));
                return;
            }
            for i in range {
                let Some(h) = self.hits.get(i) else { break };
                let icon = if h.is_dir {
                    Icon::Folder
                } else if h.inner.is_none() && unpack::is_archive_name(&h.path) {
                    Icon::Archive
                } else {
                    Icon::File
                };
                let size = if h.is_dir { String::new() } else { crate::human_size(h.size) };
                let full = [(h.name.clone(), None), (self.location(h), None), (h.modified.map(crate::fmt_time).unwrap_or_default(), None), (size, None)];
                let cells: Vec<_> = shown.iter().map(|&c| full[c].clone()).collect();
                let sel = self.selected.contains(&i);
                let r = table::row_ex(ui, &cols, icon, &cells, sel, i, h.inner.is_none());
                if r.clicked() {
                    clicked = Some(i);
                }
                if r.drag_started() {
                    start_drag = Some(i);
                }
                if r.secondary_clicked() && !sel {
                    self.selected = [i].into();
                    self.anchor = Some(i);
                }
                if r.double_clicked() {
                    action = Self::open_action(h);
                }
                let h = h.clone();
                r.context_menu(|ui| {
                    if ui.button(if h.is_dir { "폴더 열기" } else { "열기" }).clicked() {
                        action = Self::open_action(&h);
                        ui.close();
                    }
                    if ui.button("들어 있는 폴더 열기").clicked() {
                        action = match &h.inner {
                            Some(inner) => SearchAction::OpenInner(h.path.clone(), inner.clone()),
                            None => SearchAction::Reveal(h.path.clone()),
                        };
                        ui.close();
                    }
                });
            }
        });
        if let Some(i) = clicked {
            if mods.command {
                if !self.selected.remove(&i) {
                    self.selected.insert(i);
                }
                self.anchor = Some(i);
            } else if mods.shift
                && let Some(a) = self.anchor
            {
                self.selected = (a.min(i)..=a.max(i)).collect();
            } else {
                self.selected = [i].into();
                self.anchor = Some(i);
            }
        }
        if let Some(i) = start_drag {
            if !self.selected.contains(&i) {
                self.selected = [i].into();
            }
            let paths = self.selected_paths();
            if !paths.is_empty() {
                egui::DragAndDrop::set_payload(ui.ctx(), DragItems::Files { pane, paths });
            }
        }
        if focused && !ui.ctx().egui_wants_keyboard_input() {
            let (esc, enter) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Enter)));
            if esc {
                action = SearchAction::Close;
            }
            if enter
                && self.selected.len() == 1
                && let Some(h) = self.selected.iter().next().and_then(|&i| self.hits.get(i))
            {
                action = Self::open_action(h);
            }
        }
        action
    }

    fn open_action(h: &Hit) -> SearchAction {
        match &h.inner {
            Some(inner) => SearchAction::OpenInner(h.path.clone(), inner.clone()),
            None => SearchAction::Open(h.path.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_names_and_inside_archives() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::create_dir_all(d.join("가/나")).unwrap();
        std::fs::write(d.join("가/나/여름 휴가.jpg"), "x").unwrap();
        std::fs::write(d.join("가/겨울.jpg"), "x").unwrap();
        std::fs::write(d.join(".숨김 휴가.txt"), "x").unwrap();
        let src = d.join("src");
        std::fs::create_dir_all(src.join("속")).unwrap();
        std::fs::write(src.join("속/휴가 계획.txt"), "x").unwrap();
        unpack::compress(&[src.join("속")], &d.join("묶음.zip"), unpack::ArchiveKind::Zip, None).unwrap();
        std::fs::remove_dir_all(&src).unwrap();

        let ctx = egui::Context::default();
        let mut s = SearchView::start(&ctx, d.to_path_buf(), "휴가".into(), false);
        while s.running() {
            std::thread::sleep(std::time::Duration::from_millis(10));
            s.poll();
        }
        let mut names: Vec<(String, bool)> = s.hits.iter().map(|h| (h.name.clone(), h.inner.is_some())).collect();
        names.sort();
        assert_eq!(names, [("여름 휴가.jpg".to_string(), false), ("휴가 계획.txt".to_string(), true)]);
        let inner = s.hits.iter().find(|h| h.inner.is_some()).unwrap();
        assert_eq!(inner.inner.as_deref(), Some("속/휴가 계획.txt"));
        assert_eq!(s.location(inner), "묶음.zip › 속");
        assert!(matches("Report 2024.PDF", &["report".into(), "pdf".into()]));
    }
}
