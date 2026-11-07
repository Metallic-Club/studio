//! Settings as a dockable panel (ADR 0009, Settings): the same pages as the Settings window the screens before the
//! editor show, opened as a tab beside the viewport.

use egui::Ui;

use super::ids;
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::theme::fonts;

/// The Settings panel.
#[derive(Clone, Copy, Debug)]
pub struct SettingsPanel;

impl Panel for SettingsPanel {
    fn id(&self) -> &'static str {
        ids::SETTINGS
    }

    fn title(&self) -> &'static str {
        "Settings"
    }

    fn icon(&self) -> char {
        fonts::icon::GEAR
    }

    fn zone(&self) -> Zone {
        Zone::Centre
    }

    fn context(&self) -> Context {
        Context::Editor
    }

    fn open_by_default(&self) -> bool {
        false
    }

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        cx.settings_page.contents(ui, cx.settings, cx.keymap);
    }
}
