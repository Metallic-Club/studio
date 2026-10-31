//! What the tray icon offers and what closing the window does. The icon itself is made in
//! `platform`; the rules are here so they can be tested.

use crate::bridge::BridgeEvent;
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
    /// Restart into the downloaded Launcher update.
    RestartToUpdate,
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
            TrayAction::RestartToUpdate => "restart",
            TrayAction::Quit => "quit",
        }
    }

    /// The action for a menu item id, or `None` for an id this Launcher never made.
    pub fn from_id(id: &str) -> Option<TrayAction> {
        [
            TrayAction::Show,
            TrayAction::OpenStudio,
            TrayAction::CheckUpdates,
            TrayAction::RestartToUpdate,
            TrayAction::Quit,
        ]
        .into_iter()
        .find(|action| action.id() == id)
    }
}

/// The tray menu, in order: `Open Metallic Play`, `Open Studio` (only when installed),
/// `Restart to update` (only when an update is downloaded), `Check for updates`, `Quit`.
pub fn tray_menu(studio_installed: bool, update_ready: bool) -> Vec<(TrayAction, &'static str)> {
    let mut items = vec![(TrayAction::Show, "Open Metallic Play")];
    if studio_installed {
        items.push((TrayAction::OpenStudio, "Open Studio"));
    }
    if update_ready {
        items.push((TrayAction::RestartToUpdate, "Restart to update"));
    }
    items.push((TrayAction::CheckUpdates, "Check for updates"));
    items.push((TrayAction::Quit, "Quit"));
    items
}

/// What the tray menu shows now. Starts from what is installed; events keep it current.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrayMenuState {
    /// Studio is installed ("Open Studio" is offered).
    pub studio_installed: bool,
    /// A Launcher update is downloaded ("Restart to update" is offered).
    pub update_ready: bool,
}

impl TrayMenuState {
    /// The menu items for this state ([`tray_menu`]).
    pub fn items(self) -> Vec<(TrayAction, &'static str)> {
        tray_menu(self.studio_installed, self.update_ready)
    }

    /// Applies `event`; true when the menu must be rebuilt. A finished Studio install looks at
    /// the disk again with `studio_installed` (so "Open Studio" appears without a restart); a
    /// downloaded update adds "Restart to update".
    pub fn on_event(
        &mut self,
        event: &BridgeEvent,
        studio_installed: impl FnOnce() -> bool,
    ) -> bool {
        let before = *self;
        match event {
            BridgeEvent::InstallFinished { product, .. } if product == "studio" => {
                self.studio_installed = studio_installed();
            }
            BridgeEvent::UpdateReady { .. } => self.update_ready = true,
            _ => {}
        }
        *self != before
    }
}

/// Whether the window shows once the first page has loaded. `--hidden` (autostart) keeps it
/// in the tray, but only when there is a tray icon: without one the Launcher could not be
/// reached, so it shows anyway.
pub fn show_on_start(start_hidden: bool, has_tray: bool) -> bool {
    !(start_hidden && has_tray)
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
