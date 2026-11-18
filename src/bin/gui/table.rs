//! 파일 목록 표: 열 머리글(눌러서 정렬), 아이콘이 있는 이름 열, 선택 표시.

use crate::icons::{self, Icon};
use crate::style;
use eframe::egui::{self, Color32, FontId, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

pub const ROW_H: f32 = 30.0;

pub struct Col {
    pub title: &'static str,
    /// 0이면 남는 너비를 모두 씀
    pub width: f32,
    pub right: bool,
}

/// 정렬 상태: (열 번호, 오름차순 여부)
pub type Sort = (usize, bool);

fn x_ranges(cols: &[Col], rect: Rect) -> Vec<(f32, f32)> {
    let fixed: f32 = cols.iter().map(|c| c.width).sum();
    let flex = (rect.width() - fixed).max(120.0);
    let mut x = rect.left();
    cols.iter()
        .map(|c| {
            let w = if c.width == 0.0 { flex } else { c.width };
            let r = (x, x + w);
            x += w;
            r
        })
        .collect()
}

/// 머리글. 누르면 그 열로 정렬(다시 누르면 반대로).
pub fn header(ui: &mut Ui, cols: &[Col], sort: &mut Sort) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let p = ui.painter().clone();
    p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, style::border(ui)));
    for (i, ((x0, x1), col)) in x_ranges(cols, rect).into_iter().zip(cols).enumerate() {
        let cell = Rect::from_x_y_ranges(x0..=x1, rect.y_range());
        let resp = ui.interact(cell, ui.scope_id().with(("hdr", i)), Sense::click());
        if resp.hovered() {
            p.rect_filled(cell.shrink(2.0), 4.0, style::hover_fill(ui));
        }
        if resp.clicked() {
            *sort = if sort.0 == i { (i, !sort.1) } else { (i, true) };
        }
        let color = style::weak(ui);
        let font = FontId::proportional(12.5);
        let (anchor, pos) = if col.right {
            (egui::Align2::RIGHT_CENTER, pos2(x1 - 12.0, cell.center().y))
        } else {
            (egui::Align2::LEFT_CENTER, pos2(x0 + 10.0, cell.center().y))
        };
        let text_rect = p.text(pos, anchor, col.title, font, color);
        if sort.0 == i {
            // 정렬 방향 삼각형
            let cx = if col.right { text_rect.left() - 8.0 } else { text_rect.right() + 8.0 };
            let cy = cell.center().y;
            let pts = if sort.1 {
                vec![pos2(cx - 3.5, cy + 2.0), pos2(cx + 3.5, cy + 2.0), pos2(cx, cy - 2.5)]
            } else {
                vec![pos2(cx - 3.5, cy - 2.0), pos2(cx + 3.5, cy - 2.0), pos2(cx, cy + 2.5)]
            };
            p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
        }
    }
}

/// 한 줄. `cells[0]`은 아이콘 옆 이름, 나머지는 각 열 내용.
/// `name_color`가 있으면 이름 대신 상태 글자 색으로 씀 (마지막 열 강조용 아님).
pub fn row(ui: &mut Ui, cols: &[Col], icon: Icon, cells: &[(String, Option<Color32>)], selected: bool, idx: usize) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 4.0, style::selection_fill(ui));
    } else if resp.hovered() {
        p.rect_filled(rect, 4.0, style::hover_fill(ui));
    } else if idx % 2 == 1 {
        p.rect_filled(rect, 0.0, style::bar_fill(ui).gamma_multiply(0.55));
    }
    let text = ui.visuals().text_color();
    for (i, ((x0, x1), col)) in x_ranges(cols, rect).into_iter().zip(cols).enumerate() {
        let Some((s, color)) = cells.get(i) else { continue };
        let y = rect.center().y;
        let font = FontId::proportional(if i == 0 { 14.0 } else { 13.0 });
        let color = color.unwrap_or(if i == 0 { text } else { style::weak(ui) });
        if i == 0 {
            let ir = Rect::from_center_size(pos2(x0 + 20.0, y), vec2(18.0, 18.0));
            icons::paint(p, ir, icon, style::weak(ui));
            let clip = Rect::from_x_y_ranges((x0 + 36.0)..=(x1 - 8.0), rect.y_range());
            p.with_clip_rect(clip).text(pos2(x0 + 36.0, y), egui::Align2::LEFT_CENTER, s, font, color);
        } else if col.right {
            p.text(pos2(x1 - 12.0, y), egui::Align2::RIGHT_CENTER, s, font, color);
        } else {
            p.text(pos2(x0 + 10.0, y), egui::Align2::LEFT_CENTER, s, font, color);
        }
    }
    resp
}

/// 가운데 큰 아이콘과 안내 문구 (빈 화면, 암호 화면 등)
pub fn empty_state(ui: &mut Ui, rect: Rect, icon: Icon, title: &str, sub: &str) {
    let p = ui.painter();
    let c = rect.center();
    icons::paint(p, Rect::from_center_size(c - vec2(0.0, 46.0), vec2(56.0, 56.0)), icon, style::weak(ui));
    p.text(c + vec2(0.0, 6.0), egui::Align2::CENTER_CENTER, title, FontId::proportional(18.0), ui.visuals().text_color());
    p.text(c + vec2(0.0, 32.0), egui::Align2::CENTER_CENTER, sub, FontId::proportional(13.5), style::weak(ui));
}
