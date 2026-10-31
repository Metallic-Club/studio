//! The editor's panel chrome, per research doc sections 3.7 and 4.3: a 32 px header with a
//! `heading` title and ghost icon buttons, panel tabs with the 2 px `accent` bottom bar, and the
//! fills of tree and property rows (hover, selected, multi-selected, keyboard focus).
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Painter, Rect, Sense,
    Stroke, Ui, UiBuilder, vec2,
};

use crate::theme::{self, fonts, icons as phosphor};

/// How a row is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowLook {
    /// The pointer is over the row.
    pub hovered: bool,
    /// The row is the active (or only) selected row.
    pub selected: bool,
    /// The row is selected along with others.
    pub selected_weak: bool,
    /// The row has keyboard focus.
    pub focused: bool,
}

/// Paints the row background for `look` (fill only, no text).
pub fn paint_row(painter: &Painter, rect: Rect, look: RowLook) {
    let fill = if look.selected {
        theme::select()
    } else if look.selected_weak {
        theme::select_weak()
    } else if look.hovered {
        theme::bg_hover()
    } else {
        return paint_focus(painter, rect, look);
    };
    painter.rect_filled(rect, 0.0, fill);
    paint_focus(painter, rect, look);
}

fn paint_focus(painter: &Painter, rect: Rect, look: RowLook) {
    if look.focused {
        painter.rect_stroke(
            rect.shrink(1.0),
            CornerRadius::same(theme::R_XS),
            Stroke::new(2.0, theme::focus()),
            egui::StrokeKind::Inside,
        );
    }
}

/// The text colour on a row: `text-1` on a selected row, else `text-1` too (names), dimmed when
/// the row is disabled.
pub fn row_text(enabled: bool) -> Color32 {
    if enabled {
        theme::text_1()
    } else {
        theme::text_off()
    }
}

/// The fill of a docked panel.
pub fn frame() -> Frame {
    Frame::new().fill(theme::bg_panel()).inner_margin(0)
}

/// Draws a docked panel: a 32 px header (`title` plus whatever `extra` adds on the right, usually
/// ghost icon buttons) and the body inside an 8 px margin.
pub fn show<R>(
    ui: &mut Ui,
    title: &str,
    extra: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    header(ui, title, extra);
    Frame::new()
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, body)
        .inner
}

/// The panel header alone.
pub fn header(ui: &mut Ui, title: &str, extra: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::HEADER_HEIGHT),
        Sense::hover(),
    );
    let painter = ui.painter();
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, theme::line()),
    );
    painter.text(
        egui::pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        fonts::heading(),
        theme::text_1(),
    );
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(6.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    extra(&mut inner);
}

/// What a tab asked for.
#[derive(Debug, Clone, Copy, Default)]
pub struct TabResult {
    /// The tab was clicked.
    pub clicked: bool,
    /// The close mark was clicked.
    pub closed: bool,
}

/// A panel tab: `text-2` that turns `text-1` on hover, the selected one `text-1` with a 2 px
/// `accent` bar along the bottom. `dirty` adds a dot, `closable` a close mark.
pub fn tab(ui: &mut Ui, text: &str, selected: bool, dirty: bool, closable: bool) -> TabResult {
    let pad = 12.0;
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), fonts::strong(), theme::text_2());
    let dot = if dirty { 12.0 } else { 0.0 };
    let close = if closable { 22.0 } else { 0.0 };
    let width = galley.size().x + 2.0 * pad + dot + close;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, theme::HEADER_HEIGHT), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, text));
    let hover =
        ui.ctx()
            .animate_bool_with_time(resp.id.with("hover"), resp.hovered(), theme::t_fast());
    let ink = if selected {
        theme::text_1()
    } else {
        theme::text_2().lerp_to_gamma(theme::text_1(), hover)
    };
    let painter = ui.painter();
    painter.galley_with_override_text_color(
        egui::pos2(rect.left() + pad, rect.center().y - galley.size().y / 2.0),
        galley.clone(),
        ink,
    );
    if dirty {
        painter.circle_filled(
            egui::pos2(rect.left() + pad + galley.size().x + 8.0, rect.center().y),
            3.0,
            theme::text_2(),
        );
    }
    if selected {
        painter.rect_filled(
            Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - 2.0),
                rect.right_bottom(),
            ),
            0.0,
            theme::accent(),
        );
    }
    if resp.has_focus() {
        painter.rect_stroke(
            rect.shrink(2.0),
            CornerRadius::same(theme::R_XS),
            Stroke::new(2.0, theme::focus()),
            egui::StrokeKind::Inside,
        );
    }
    let mut out = TabResult {
        clicked: resp.clicked(),
        closed: false,
    };
    if closable {
        let at = Rect::from_center_size(
            egui::pos2(rect.right() - pad + 2.0, rect.center().y),
            vec2(18.0, 18.0),
        );
        let x = ui.interact(at, resp.id.with("close"), Sense::click());
        if x.hovered() {
            ui.painter()
                .rect_filled(at, CornerRadius::same(theme::R_XS), theme::bg_hover());
        }
        ui.painter().text(
            at.center(),
            Align2::CENTER_CENTER,
            phosphor::X,
            fonts::icon(12.0),
            if x.hovered() {
                theme::text_1()
            } else {
                theme::text_3()
            },
        );
        let x = x.on_hover_text("Close");
        out.closed = x.clicked();
        out.clicked = out.clicked && !out.closed;
    }
    out
}

/// A thin strip behind a row of tabs: `bg-panel` with a 1 px `line` under it.
pub fn tab_strip<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = Rect::from_min_size(
        ui.cursor().min,
        vec2(ui.available_width(), theme::HEADER_HEIGHT),
    );
    ui.painter().rect_filled(rect, 0.0, theme::bg_panel());
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, theme::line()),
    );
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 0.0;
    let out = add(&mut inner);
    ui.advance_cursor_after_rect(rect);
    out
}

/// A filter chip (Output): 26 px tall, `bg-active` with `text-1` when on, an outline with `text-2`
/// when off. `dot` is a small colour mark before the label (a tag colour), so the label itself
/// always uses a text role that meets contrast on the chip.
pub fn chip(ui: &mut Ui, text: &str, on: &mut bool, dot: Option<Color32>) -> bool {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), fonts::caption(), theme::text_1());
    let lead = if dot.is_some() { 14.0 } else { 0.0 };
    let size = vec2(galley.size().x + 20.0 + lead, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, *on, text));
    let hover = resp.hovered();
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::R_SM);
    if *on {
        painter.rect_filled(rect, radius, theme::bg_active());
    } else if hover {
        painter.rect_filled(rect, radius, theme::bg_hover());
    }
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::line_control()),
        egui::StrokeKind::Inside,
    );
    if let Some(c) = dot {
        painter.circle_filled(egui::pos2(rect.left() + 13.0, rect.center().y), 3.5, c);
    }
    let ink = if *on || hover {
        theme::text_1()
    } else {
        theme::text_2()
    };
    painter.galley_with_override_text_color(
        egui::pos2(
            rect.left() + 10.0 + lead,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        ink,
    );
    if resp.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            radius,
            Stroke::new(2.0, theme::focus()),
            egui::StrokeKind::Outside,
        );
    }
    if resp.clicked() {
        *on = !*on;
        return true;
    }
    false
}
