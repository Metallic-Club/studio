//! The tray menu state and the start-hidden rule.

use metallic_launcher::{BridgeEvent, TrayAction, TrayMenuState, show_on_start};

fn actions(state: TrayMenuState) -> Vec<TrayAction> {
    state
        .items()
        .into_iter()
        .map(|(action, _)| action)
        .collect()
}

fn finished(product: &str, ok: bool) -> BridgeEvent {
    BridgeEvent::InstallFinished {
        product: product.to_string(),
        ok,
    }
}

/// #289: "Open Studio" appears after a Studio install without restarting the Launcher.
#[test]
fn a_studio_install_adds_open_studio() {
    let mut state = TrayMenuState::default();
    assert!(!actions(state).contains(&TrayAction::OpenStudio));
    assert!(state.on_event(&finished("studio", true), || true));
    assert!(actions(state).contains(&TrayAction::OpenStudio));
    // Nothing changed: no rebuild.
    assert!(!state.on_event(&finished("studio", true), || true));
    // The disk decides, not the event: a failed install that left no Studio changes nothing.
    let mut state = TrayMenuState::default();
    assert!(!state.on_event(&finished("studio", false), || false));
    // Another product never looks at Studio.
    assert!(!state.on_event(&finished("player", true), || { panic!("not asked") }));
}

#[test]
fn a_downloaded_update_adds_restart_and_keeps_studio() {
    let mut state = TrayMenuState {
        studio_installed: true,
        update_ready: false,
    };
    let ready = BridgeEvent::UpdateReady {
        version: "0.2.0".to_string(),
        required: false,
    };
    assert!(state.on_event(&ready, || false));
    assert_eq!(
        actions(state),
        [
            TrayAction::Show,
            TrayAction::OpenStudio,
            TrayAction::RestartToUpdate,
            TrayAction::CheckUpdates,
            TrayAction::Quit
        ]
    );
}

/// #289: `--hidden` starts in the tray, but never without a tray icon.
#[test]
fn hidden_start_stays_in_the_tray_only_when_there_is_one() {
    assert!(show_on_start(false, true));
    assert!(show_on_start(false, false));
    assert!(!show_on_start(true, true));
    assert!(show_on_start(true, false));
}
