//! opizenAI_Unpack GUI — 압축 풀기 / 압축하기 데스크톱 앱 (Windows, Linux).
//!
//! 명령줄 인자로 파일·폴더 경로를 받으면 목록에 추가합니다.
//!
//! 파일 관리자 우클릭 메뉴에서 쓰는 옵션:
//! - `--smart <파일>…`    : "알아서 풀기" (폴더가 하나면 그대로, 아니면 파일 이름 폴더로)
//! - `--here <파일>…`     : "여기에 압축 풀기" (폴더를 만들지 않고 그대로)
//! - `--folder <파일>…`   : "새 폴더에 압축 풀기"
//! - `--auto <파일>…`     : 전체 창을 열고 바로 해제 시작
//! - `--zip <항목>…`      : "ZIP으로 압축" (선택한 항목 전체를 하나로)
//! - `--7z <항목>…`       : "7Z로 압축"
//! - `--compress <항목>…` : "압축하기…" (형식·이름·암호 선택)

#![cfg_attr(windows, windows_subsystem = "windows")]

mod browse;
mod fsview;
mod gather;
mod home;
mod icons;
mod pack;
mod places;
mod quick;
mod settings;
mod style;
mod table;

use eframe::egui;
use std::path::{Path, PathBuf};
use unpack::{ArchiveKind, Mode};

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
            Some("--smart") => launch = Launch::Extract(Mode::Smart),
            Some("--here") => launch = Launch::Extract(Mode::Here),
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
            let tag = match mode {
                Mode::Smart => "SMART",
                Mode::Here => "HERE",
                Mode::NewFolder => "FOLDER",
            };
            let Some(files) = gather::gather(tag, files) else { return Ok(()) };
            let height = (200.0 + 30.0 * files.len().min(7) as f32).min(420.0);
            return eframe::run_native(
                unpack::APP_NAME,
                small_window([480.0, height]),
                Box::new(move |cc| {
                    setup(&cc.egui_ctx);
                    Ok(Box::new(quick::QuickApp::new(&cc.egui_ctx, mode, files)))
                }),
            );
        }
        Launch::Pack(kind) if !files.is_empty() => {
            let tag = kind.map(|k| k.label()).unwrap_or("ASK");
            // 다른 프로세스가 모아서 처리하면 이 프로세스는 조용히 끝냄
            let Some(files) = gather::gather(tag, files) else { return Ok(()) };
            let size = if kind.is_some() { [460.0, 200.0] } else { [520.0, 330.0] };
            return eframe::run_native(
                unpack::APP_NAME,
                small_window(size),
                Box::new(move |cc| {
                    setup(&cc.egui_ctx);
                    Ok(Box::new(pack::PackApp::new(&cc.egui_ctx, files, kind)))
                }),
            );
        }
        _ => {}
    }
    let auto = matches!(launch, Launch::Main { auto: true });
    clean_old_temp();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(unpack::APP_NAME)
        .with_inner_size([1000.0, 640.0])
        .with_min_inner_size([680.0, 420.0])
        .with_drag_and_drop(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }

    eframe::run_native(
        unpack::APP_NAME,
        eframe::NativeOptions { viewport, centered: true, ..Default::default() },
        Box::new(move |cc| {
            setup(&cc.egui_ctx);
            Ok(Box::new(home::Home::new(&cc.egui_ctx, files, auto)))
        }),
    )
}

/// 글꼴과 스타일을 적용합니다.
fn setup(ctx: &egui::Context) {
    install_fonts(ctx);
    style::apply(ctx);
    // 기본은 운영체제 설정을 따름. OPIZEN_THEME=light|dark 로 강제할 수 있음
    match std::env::var("OPIZEN_THEME").as_deref() {
        Ok("light") => ctx.set_theme(egui::Theme::Light),
        Ok("dark") => ctx.set_theme(egui::Theme::Dark),
        _ => ctx.set_theme(egui::ThemePreference::System),
    }
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

/// 파일을 운영체제 기본 프로그램으로 열기 (압축 파일 안 파일 열기용)
fn open_with_default(path: &Path) {
    #[cfg(windows)]
    let cmd = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = std::process::Command::new("xdg-open").arg(path).spawn();
    let _ = cmd;
}

/// 현지 시각 "2024-05-01 13:20"
fn fmt_time(t: std::time::SystemTime) -> String {
    let utc = time::OffsetDateTime::from(t);
    let local = time::UtcOffset::current_local_offset().map(|o| utc.to_offset(o)).unwrap_or(utc);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", local.year(), local.month() as u8, local.day(), local.hour(), local.minute())
}

/// 지난번에 파일을 열면서 만든 임시 폴더 중 하루 지난 것 지우기
fn clean_old_temp() {
    let base = std::env::temp_dir().join(unpack::APP_NAME);
    let Ok(rd) = std::fs::read_dir(&base) else { return };
    for e in rd.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 86_400);
        if old && e.file_name().to_string_lossy().starts_with("open-") {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// 압축 파일 안 파일을 열 때 쓰는 임시 폴더 (프로그램마다 따로, 열 때마다 새로)
fn open_temp_dir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(unpack::APP_NAME).join(format!("open-{}", std::process::id()));
    (0..).map(|n| base.join(n.to_string())).find(|p| !p.exists()).expect("무한 반복자")
}
