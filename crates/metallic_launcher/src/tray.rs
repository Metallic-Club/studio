//! What the tray icon offers and what closing the window does. The icon itself is made in
//! `platform`; the rules are here so they can be tested.

use crate::settings::LauncherSettings;

/// A tray menu entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    /// Show the Launcher window (also the left click on the icon).
    Show,
    /// Open Metallic Studio.
    OpenStudio,
    /// Look for a Launcher update.
    CheckUpdates,
    /// Close the Launcher.
    Quit,
}

impl TrayAction {
    /// The id the menu item carries.
    pub fn id(self) -> &'static str {
        match self {
            TrayAction::Show => "show",
            TrayAction::OpenStudio => "studio",
            TrayAction::CheckUpdates => "updates",
            TrayAction::Quit => "quit",
        }
    }

    /// The action for a menu item id, or `None` for an id this Launcher never made.
    pub fn from_id(id: &str) -> Option<TrayAction> {
        [
            TrayAction::Show,
            TrayAction::OpenStudio,
            TrayAction::CheckUpdates,
            TrayAction::Quit,
        ]
        .into_iter()
        .find(|action| action.id() == id)
    }
}

/// The tray menu, in order: `Open Metallic`, `Open Studio` (only when installed),
/// `Check for updates`, `Quit`.
pub fn tray_menu(studio_installed: bool) -> Vec<(TrayAction, &'static str)> {
    let mut items = vec![(TrayAction::Show, "Open Metallic")];
    if studio_installed {
        items.push((TrayAction::OpenStudio, "Open Studio"));
    }
    items.push((TrayAction::CheckUpdates, "Check for updates"));
    items.push((TrayAction::Quit, "Quit"));
    items
}

/// What the close button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    /// Hide the window; the Launcher keeps running in the tray.
    HideToTray,
    /// Close the Launcher.
    Quit,
}

/// The close button's action for these settings.
pub fn on_close_requested(settings: &LauncherSettings) -> CloseAction {
    if settings.close_to_tray {
        CloseAction::HideToTray
    } else {
        CloseAction::Quit
    }
}
