//! 모든 창이 함께 쓰는 단순한 스타일: 넉넉한 여백, 둥근 모서리, 조금 큰 글자.

use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, TextStyle, Vec2};

/// 강조 색 (버튼, 선택 표시)
pub const ACCENT: Color32 = Color32::from_rgb(0x2f, 0x6f, 0xeb);
pub const OK: Color32 = Color32::from_rgb(0x2e, 0xa0, 0x5a);
pub const WARN: Color32 = Color32::from_rgb(0xd9, 0x8c, 0x1a);
pub const ERR: Color32 = Color32::from_rgb(0xd6, 0x45, 0x45);

pub fn apply(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        s.text_styles.insert(TextStyle::Body, FontId::proportional(15.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(15.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(12.5));
        s.text_styles.insert(TextStyle::Heading, FontId::proportional(20.0));
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.button_padding = Vec2::new(14.0, 7.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.window_margin = Margin::same(16);
        let r = CornerRadius::same(8);
        let w = &mut s.visuals.widgets;
        for v in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            v.corner_radius = r;
        }
        s.visuals.selection.bg_fill = ACCENT;
        s.visuals.hyperlink_color = ACCENT;
    });
}

/// 강조 색으로 채운 주 버튼
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()).color(Color32::WHITE).strong())
        .fill(ACCENT)
        .min_size(Vec2::new(120.0, 36.0))
}

/// 테두리만 있는 보조 버튼
pub fn secondary(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()).strong()).min_size(Vec2::new(120.0, 36.0))
}

/// 창 전체에 쓰는 여백 있는 패널 프레임
pub fn panel(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::central_panel(&ctx.global_style()).inner_margin(Margin::same(20))
}
