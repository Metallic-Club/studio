//! Studio's tooltips (#774): faintly rounded. egui draws a tooltip in its popup frame, whose corner radius is the
//! menus' ([`radius::PANEL`]); Studio's tooltips draw in a frame of their own with [`radius::TOOLTIP`] instead, and
//! leave the menus as they are. [`Tip`] is used in place of `Response::on_hover_text` and its kin, and behaves as
//! they do: shown when the widget is hovered and enabled (or disabled, for [`Tip::disabled_tip`]), and its text
//! describes the widget to screen readers.

use egui::{CornerRadius, Response, Tooltip, Ui, WidgetText};

use super::tokens::radius;

/// The frame Studio's tooltips are drawn in: egui's popup frame, faintly rounded.
pub fn frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::popup(&ctx.global_style()).corner_radius(CornerRadius::same(radius::TOOLTIP))
}

/// Tooltips in Studio's frame, for a widget's [`Response`].
pub trait Tip: Sized {
    /// Shows `text` while the widget is hovered and enabled, as `on_hover_text` does.
    fn tip(self, text: impl Into<WidgetText>) -> Self;
    /// Shows what `add` draws while the widget is hovered and enabled, as `on_hover_ui` does.
    fn tip_ui(self, add: impl FnOnce(&mut Ui)) -> Self;
    /// Shows `text` while the widget is hovered and disabled, as `on_disabled_hover_text` does.
    fn disabled_tip(self, text: impl Into<WidgetText>) -> Self;
}

impl Tip for Response {
    fn tip(self, text: impl Into<WidgetText>) -> Self {
        let text = text.into();
        describe(&self, text.text());
        self.tip_ui(|ui| {
            // As egui's: the area's sizing would otherwise shrink a tooltip whose text changes.
            ui.set_max_width(ui.spacing().tooltip_width);
            ui.add(egui::Label::new(text));
        })
    }

    fn tip_ui(self, add: impl FnOnce(&mut Ui)) -> Self {
        let mut tooltip = Tooltip::for_enabled(&self);
        tooltip.popup = tooltip.popup.frame(frame(&self.ctx));
        tooltip.show(add);
        self
    }

    fn disabled_tip(self, text: impl Into<WidgetText>) -> Self {
        let text = text.into();
        let mut tooltip = Tooltip::for_disabled(&self);
        tooltip.popup = tooltip.popup.frame(frame(&self.ctx));
        tooltip.show(|ui| {
            ui.set_max_width(ui.spacing().tooltip_width);
            ui.add(egui::Label::new(text));
        });
        self
    }
}

/// Uses a tooltip's `text` as the widget's description for screen readers, and as its name when it has none (an
/// icon alone), as egui's `on_hover_text` does.
fn describe(response: &Response, text: &str) {
    let ctx = &response.ctx;
    if !ctx.has_accesskit_node(response.id) {
        return;
    }
    ctx.accesskit_node_builder(response.id, |node| {
        if node.description().is_none() {
            node.set_description(text);
        }
        if node.role() != egui::accesskit::Role::Label
            && node.labelled_by().is_empty()
            && node.label().is_none_or(str::is_empty)
        {
            node.set_label(text);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tooltip_is_faintly_rounded() {
        let ctx = egui::Context::default();
        let frame = frame(&ctx);
        assert_eq!(frame.corner_radius, CornerRadius::same(radius::TOOLTIP));
        assert!(radius::TOOLTIP < radius::PANEL && radius::TOOLTIP > 0);
    }
}
