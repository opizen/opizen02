//! 직접 그리는 벡터 아이콘 — 글꼴(이모지)에 의존하지 않아 어느 OS에서나 똑같이 보입니다.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Open,
    Extract,
    ExtractTo,
    Compress,
    Settings,
    Folder,
    File,
    Archive,
    Lock,
    Up,
    Back,
    Forward,
    /// 드라이브 (저장 장치)
    Drive,
    Close,
    /// 창 나누기 (창 추가)
    Split,
    NewFolder,
    /// 검사 (방패 + 체크)
    Check,
    /// 왼쪽 바로가기 목록 보이기/숨기기
    Sidebar,
}

/// 폴더 아이콘 색 (파일 관리자에서 흔히 쓰는 노란색)
pub const FOLDER: Color32 = Color32::from_rgb(0xf2, 0xb8, 0x4b);
pub const FOLDER_DARK: Color32 = Color32::from_rgb(0xe0, 0xa1, 0x30);
/// 압축 파일 아이콘 색
pub const ARCHIVE: Color32 = Color32::from_rgb(0x8b, 0x5c, 0xf6);

/// `rect` 안에 아이콘을 그립니다. `color`는 선 아이콘의 색입니다.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let u = s / 24.0; // 24x24 격자 기준
    let at = |x: f32, y: f32| pos2(c.x + (x - 12.0) * u, c.y + (y - 12.0) * u);
    let line = Stroke::new((1.8 * u).max(1.2), color);

    match icon {
        Icon::Folder => folder(p, &at, u, true, line),
        Icon::Open => {
            folder(p, &at, u, false, line);
            // 열린 덮개
            p.add(Shape::convex_polygon(
                vec![at(5.0, 19.0), at(8.0, 11.0), at(22.0, 11.0), at(19.0, 19.0)],
                Color32::TRANSPARENT,
                line,
            ));
        }
        Icon::File => {
            let pts = vec![at(6.0, 3.0), at(14.0, 3.0), at(19.0, 8.0), at(19.0, 21.0), at(6.0, 21.0)];
            p.add(Shape::convex_polygon(pts, color.gamma_multiply(0.12), line));
            p.add(Shape::line(vec![at(14.0, 3.0), at(14.0, 8.0), at(19.0, 8.0)], line));
        }
        Icon::Archive => {
            // 보라색 서류 모양 + 가운데 지퍼
            let r = Rect::from_min_max(at(4.5, 2.5), at(19.5, 21.5));
            p.rect_filled(r, 2.5 * u, ARCHIVE);
            let teeth = Color32::from_rgb(0xed, 0xe9, 0xfe);
            for i in 0..4 {
                let y = 3.5 + i as f32 * 2.4;
                let x = if i % 2 == 0 { 9.5 } else { 12.0 };
                p.rect_filled(Rect::from_min_max(at(x, y), at(x + 2.5, y + 1.6)), 0.4 * u, teeth);
            }
            // 손잡이
            p.rect_filled(Rect::from_min_max(at(9.5, 13.5), at(14.5, 19.0)), 1.2 * u, teeth);
            p.rect_filled(Rect::from_min_max(at(11.0, 15.5), at(13.0, 17.5)), 0.5 * u, ARCHIVE);
        }
        Icon::Extract | Icon::ExtractTo | Icon::Compress => {
            // 상자
            let lid_y = 11.0;
            p.add(Shape::convex_polygon(
                vec![at(4.0, lid_y), at(20.0, lid_y), at(20.0, 21.0), at(4.0, 21.0)],
                color.gamma_multiply(0.12),
                line,
            ));
            p.line_segment([at(9.0, 15.0), at(15.0, 15.0)], line);
            // 화살표: 풀기는 위로 나옴, 압축은 아래로 들어감
            if icon == Icon::Compress {
                p.line_segment([at(12.0, 1.5), at(12.0, 9.5)], line);
                p.add(Shape::line(vec![at(8.5, 6.0), at(12.0, 9.5), at(15.5, 6.0)], line));
            } else {
                p.line_segment([at(12.0, 9.5), at(12.0, 1.5)], line);
                p.add(Shape::line(vec![at(8.5, 5.0), at(12.0, 1.5), at(15.5, 5.0)], line));
            }
            if icon == Icon::ExtractTo {
                // 오른쪽 아래 작은 폴더 표시
                let r = Rect::from_min_max(at(15.0, 15.5), at(23.0, 22.5));
                p.rect_filled(r, 1.5 * u, FOLDER);
            }
        }
        Icon::Settings => {
            let r = 7.0 * u;
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * r, c + d * (r + 3.0 * u)], Stroke::new(3.0 * u, color));
            }
            p.circle_stroke(c, r, line);
            p.circle_stroke(c, 2.8 * u, line);
        }
        Icon::Lock => {
            let body = Rect::from_min_max(at(5.0, 11.0), at(19.0, 21.0));
            p.rect_filled(body, 2.0 * u, color);
            let shackle: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = std::f32::consts::PI + i as f32 * std::f32::consts::PI / 16.0;
                    at(12.0 + 4.5 * a.cos(), 9.0 + 5.0 * a.sin())
                })
                .collect();
            let mut pts = vec![at(7.5, 11.0)];
            pts.extend(shackle);
            pts.push(at(16.5, 11.0));
            p.add(Shape::line(pts, line));
        }
        Icon::Up => {
            p.line_segment([at(12.0, 20.0), at(12.0, 5.0)], line);
            p.add(Shape::line(vec![at(6.0, 11.0), at(12.0, 5.0), at(18.0, 11.0)], line));
        }
        Icon::Check => {
            // 방패
            let shield = vec![at(12.0, 2.0), at(20.0, 5.0), at(19.0, 14.0), at(12.0, 22.0), at(5.0, 14.0), at(4.0, 5.0)];
            p.add(Shape::convex_polygon(shield, color.gamma_multiply(0.12), line));
            p.add(Shape::line(vec![at(8.0, 12.0), at(11.0, 15.0), at(16.5, 9.0)], line));
        }
        Icon::Forward => {
            p.add(Shape::line(vec![at(9.0, 5.0), at(16.0, 12.0), at(9.0, 19.0)], line));
        }
        Icon::Drive => {
            let body = Rect::from_min_max(at(3.0, 8.0), at(21.0, 17.0));
            p.rect_filled(body, 2.0 * u, color.gamma_multiply(0.15));
            p.rect_stroke(body, 2.0 * u, line, egui::StrokeKind::Middle);
            p.circle_filled(at(17.0, 12.5), 1.3 * u, color);
            p.line_segment([at(6.0, 12.5), at(12.0, 12.5)], line);
        }
        Icon::Close => {
            p.line_segment([at(6.0, 6.0), at(18.0, 18.0)], line);
            p.line_segment([at(18.0, 6.0), at(6.0, 18.0)], line);
        }
        Icon::Split => {
            let r = Rect::from_min_max(at(3.0, 4.0), at(21.0, 20.0));
            p.rect_filled(r, 2.0 * u, color.gamma_multiply(0.12));
            p.rect_stroke(r, 2.0 * u, line, egui::StrokeKind::Middle);
            p.line_segment([at(12.0, 4.0), at(12.0, 20.0)], line);
            p.line_segment([at(15.0, 12.0), at(19.0, 12.0)], line);
            p.line_segment([at(17.0, 10.0), at(17.0, 14.0)], line);
        }
        Icon::Sidebar => {
            let r = Rect::from_min_max(at(3.0, 4.0), at(21.0, 20.0));
            p.rect_stroke(r, 2.0 * u, line, egui::StrokeKind::Middle);
            p.rect_filled(Rect::from_min_max(at(3.0, 4.0), at(9.0, 20.0)), 2.0 * u, color.gamma_multiply(0.35));
            p.line_segment([at(9.0, 4.0), at(9.0, 20.0)], line);
        }
        Icon::NewFolder => {
            folder(p, &at, u, false, line);
            p.line_segment([at(15.0, 12.5), at(15.0, 18.5)], line);
            p.line_segment([at(12.0, 15.5), at(18.0, 15.5)], line);
        }
        Icon::Back => {
            p.add(Shape::line(vec![at(15.0, 5.0), at(8.0, 12.0), at(15.0, 19.0)], line));
        }
    }
}

fn folder(p: &Painter, at: &impl Fn(f32, f32) -> Pos2, u: f32, filled: bool, line: Stroke) {
    let tab = vec![at(2.5, 6.0), at(9.0, 6.0), at(11.0, 8.0), at(2.5, 8.0)];
    let body = Rect::from_min_max(at(2.5, 7.5), at(21.5, 19.5));
    if filled {
        p.add(Shape::convex_polygon(tab, FOLDER_DARK, Stroke::NONE));
        p.rect_filled(body, 2.0 * u, FOLDER);
    } else {
        p.add(Shape::line(vec![at(2.5, 19.5), at(2.5, 6.0), at(9.0, 6.0), at(11.0, 8.0), at(21.5, 8.0), at(21.5, 11.0)], line));
        p.line_segment([at(2.5, 19.5), at(19.0, 19.5)], line);
    }
}
