//! 공통 스타일 — 운영체제 라이트/다크 설정을 따르는 깔끔한 데스크톱 앱 모양.

use crate::icons::{self, Icon};
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Margin, Response, Sense, Stroke, TextStyle, Theme, Ui, Vec2, vec2,
};

/// 강조 색 (버튼, 선택 표시)
pub const ACCENT: Color32 = Color32::from_rgb(0x25, 0x63, 0xeb);
pub const OK: Color32 = Color32::from_rgb(0x16, 0xa3, 0x4a);
pub const WARN: Color32 = Color32::from_rgb(0xd9, 0x77, 0x06);
pub const ERR: Color32 = Color32::from_rgb(0xdc, 0x26, 0x26);

/// 테마별 바탕색
struct Palette {
    window: Color32,
    bar: Color32,
    border: Color32,
    hover: Color32,
    text: Color32,
    weak: Color32,
    field: Color32,
}

const LIGHT: Palette = Palette {
    window: Color32::from_rgb(0xff, 0xff, 0xff),
    bar: Color32::from_rgb(0xf4, 0xf5, 0xf7),
    border: Color32::from_rgb(0xe2, 0xe4, 0xe9),
    hover: Color32::from_rgb(0xe9, 0xec, 0xf1),
    text: Color32::from_rgb(0x1f, 0x23, 0x2b),
    weak: Color32::from_rgb(0x6b, 0x72, 0x80),
    field: Color32::from_rgb(0xf7, 0xf8, 0xfa),
};

const DARK: Palette = Palette {
    window: Color32::from_rgb(0x1e, 0x1f, 0x23),
    bar: Color32::from_rgb(0x26, 0x28, 0x2d),
    border: Color32::from_rgb(0x36, 0x39, 0x40),
    hover: Color32::from_rgb(0x33, 0x36, 0x3d),
    text: Color32::from_rgb(0xe6, 0xe7, 0xea),
    weak: Color32::from_rgb(0x9a, 0xa0, 0xab),
    field: Color32::from_rgb(0x18, 0x19, 0x1c),
};

fn palette(ui: &Ui) -> &'static Palette {
    if ui.visuals().dark_mode { &DARK } else { &LIGHT }
}

pub fn bar_fill(ui: &Ui) -> Color32 {
    palette(ui).bar
}
pub fn border(ui: &Ui) -> Color32 {
    palette(ui).border
}
pub fn hover_fill(ui: &Ui) -> Color32 {
    palette(ui).hover
}
/// 선택된 줄 배경
pub fn selection_fill(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { Color32::from_rgb(0x1e, 0x3a, 0x6e) } else { Color32::from_rgb(0xdb, 0xe8, 0xfe) }
}

pub fn apply(ctx: &egui::Context) {
    for (theme, pal) in [(Theme::Light, &LIGHT), (Theme::Dark, &DARK)] {
        ctx.style_mut_of(theme, |s| {
            s.text_styles.insert(TextStyle::Body, FontId::proportional(14.5));
            s.text_styles.insert(TextStyle::Button, FontId::proportional(14.5));
            s.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
            s.text_styles.insert(TextStyle::Heading, FontId::proportional(19.0));
            s.spacing.item_spacing = Vec2::new(8.0, 6.0);
            s.spacing.button_padding = Vec2::new(12.0, 6.0);
            s.spacing.interact_size.y = 28.0;
            s.spacing.window_margin = Margin::same(16);

            let v = &mut s.visuals;
            v.panel_fill = pal.window;
            v.window_fill = pal.window;
            v.extreme_bg_color = pal.field;
            v.faint_bg_color = pal.bar;
            v.window_stroke = Stroke::new(1.0, pal.border);
            v.override_text_color = None;
            v.hyperlink_color = ACCENT;
            v.selection.bg_fill = ACCENT;
            v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
            let r = CornerRadius::same(6);
            let w = &mut v.widgets;
            w.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
            w.noninteractive.fg_stroke = Stroke::new(1.0, pal.text);
            w.inactive.weak_bg_fill = pal.bar;
            w.inactive.bg_fill = pal.bar;
            w.inactive.bg_stroke = Stroke::new(1.0, pal.border);
            w.inactive.fg_stroke = Stroke::new(1.0, pal.text);
            w.hovered.weak_bg_fill = pal.hover;
            w.hovered.bg_fill = pal.hover;
            w.hovered.bg_stroke = Stroke::new(1.0, pal.border);
            w.hovered.fg_stroke = Stroke::new(1.0, pal.text);
            w.active.weak_bg_fill = pal.hover;
            w.active.bg_fill = pal.hover;
            w.active.fg_stroke = Stroke::new(1.0, pal.text);
            for st in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
                st.corner_radius = r;
                st.expansion = 0.0;
            }
            let _ = pal.weak;
        });
    }
}

/// 흐린 글자 색
pub fn weak(ui: &Ui) -> Color32 {
    palette(ui).weak
}

/// 강조 색으로 채운 주 버튼
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()).color(Color32::WHITE).strong())
        .fill(ACCENT)
        .stroke(Stroke::NONE)
        .min_size(Vec2::new(110.0, 32.0))
}

/// 테두리만 있는 보조 버튼
pub fn secondary(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned())).min_size(Vec2::new(110.0, 32.0))
}

/// 도구 모음 버튼: 아이콘 위, 글자 아래
pub fn tool_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool) -> Response {
    let size = vec2(76.0, 58.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let p = ui.painter();
    if enabled && resp.hovered() {
        p.rect_filled(rect, 8.0, palette(ui).hover);
    }
    let color = if !enabled {
        palette(ui).weak.gamma_multiply(0.5)
    } else if matches!(icon, Icon::Extract | Icon::ExtractTo | Icon::Compress) {
        ACCENT
    } else {
        palette(ui).text
    };
    let icon_rect = egui::Rect::from_center_size(rect.center_top() + vec2(0.0, 20.0), vec2(26.0, 26.0));
    icons::paint(p, icon_rect, icon, color);
    let text_color = if enabled { palette(ui).text } else { palette(ui).weak.gamma_multiply(0.6) };
    p.text(rect.center_bottom() - vec2(0.0, 11.0), egui::Align2::CENTER_CENTER, label, FontId::proportional(12.5), text_color);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// 세로 구분선 (도구 모음용)
pub fn vsep(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 40.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, palette(ui).border));
}

/// 내용 영역 프레임
pub fn panel(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::central_panel(&ctx.global_style()).inner_margin(Margin::same(20))
}

/// 위·아래 막대 프레임 (도구 모음, 상태 표시줄)
pub fn bar(ui: &Ui, margin: Margin) -> egui::Frame {
    egui::Frame::NONE.fill(palette(ui).bar).inner_margin(margin)
}
