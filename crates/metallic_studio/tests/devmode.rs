//! Developer mode (#14): the channel default and the Settings switch, feature flags, bug report
//! zips (contents and redaction), and the `dev.*` actions behind the Developer menu.

use std::path::Path;

use eframe::egui::{self, Color32, ColorImage, Key};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_core::{Value, World};
use metallic_studio::devmode::bug_report::{BugReport, Redactor, crc32, secrets_in_stored};
use metallic_studio::devmode::{DevMode, FeatureFlags, channel_named};
use metallic_studio::editor::EditorState;
use metallic_studio::editor_ui::EditorUi;
use metallic_studio::output::OutputPanel;
use metallic_studio::settings::Settings;
use metallic_studio::updater::Channel;

#[test]
fn dev_channel_enables_devmode_by_default() {
    assert!(DevMode::default_for(Channel::Dev).enabled);
    assert!(DevMode::resolve(None, Channel::Dev).enabled);
    assert!(
        !DevMode::resolve(Some(false), Channel::Dev).enabled,
        "Settings wins"
    );
    assert_eq!(channel_named(Some("dev")), Channel::Dev);
    assert_eq!(
        channel_named(None),
        Channel::Dev,
        "untagged builds are dev builds"
    );
    // A fresh settings file has no choice, so the build channel decides.
    assert_eq!(Settings::default().developer_mode, None);
}

#[test]
fn stable_channel_disables_by_default() {
    assert!(!DevMode::default_for(Channel::Stable).enabled);
    assert!(!DevMode::resolve(None, Channel::Stable).enabled);
    assert!(
        DevMode::resolve(Some(true), Channel::Stable).enabled,
        "Settings wins"
    );
    assert_eq!(channel_named(Some("stable")), Channel::Stable);
}

#[test]
fn the_developer_mode_choice_is_saved_only_once_made() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");
    let s = Settings::default();
    s.save(&file).unwrap();
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .contains("developer_mode")
    );
    let s = Settings {
        developer_mode: Some(false),
        ..Settings::default()
    };
    s.save(&file).unwrap();
    assert_eq!(Settings::load(&file).developer_mode, Some(false));
}

#[test]
fn flags_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cfg").join("flags.toml");
    let mut flags = FeatureFlags::load(&file);
    assert!(flags.is_empty(), "a missing file is no flags");
    assert!(!flags.is_on("new_shadows"));
    assert!(flags.set("new_shadows", true));
    assert!(flags.set("gpu.culling", false));
    assert!(!flags.set("Bad Name", true), "names are checked");
    flags.save().unwrap();

    let back = FeatureFlags::load(&file);
    assert_eq!(back, flags);
    assert!(back.is_on("new_shadows"));
    assert!(!back.is_on("gpu.culling"));
    assert!(!back.is_on("unknown"));

    // Bad lines are skipped, the rest still loads; a corrupt file is no flags.
    let parsed = FeatureFlags::parse("good = true\nnumber = 3\n\"Bad Name\" = true\n");
    assert_eq!(parsed.iter().collect::<Vec<_>>(), vec![("good", true)]);
    assert!(FeatureFlags::parse("not toml [").is_empty());
}

/// A zip's entries by name, read from the central directory; each CRC is checked.
fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let u32_at =
        |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let end = bytes.len() - 22;
    assert_eq!(u32_at(end), 0x0605_4b50, "end of central directory");
    let count = u16_at(end + 10);
    let mut at = u32_at(end + 16) as usize;
    let mut out = Vec::new();
    for _ in 0..count {
        assert_eq!(u32_at(at), 0x0201_4b50, "central directory record");
        assert_eq!(u16_at(at + 10), 0, "stored");
        let crc = u32_at(at + 16);
        let size = u32_at(at + 20) as usize;
        let name_len = u16_at(at + 28);
        let local = u32_at(at + 42) as usize;
        let name = String::from_utf8(bytes[at + 46..at + 46 + name_len].to_vec()).unwrap();
        assert_eq!(u32_at(local), 0x0403_4b50, "local header");
        let data_at = local + 30 + u16_at(local + 26) + u16_at(local + 28);
        let data = bytes[data_at..data_at + size].to_vec();
        assert_eq!(crc32(&data), crc, "{name} CRC");
        out.push((name, data));
        at += 46 + name_len + u16_at(at + 30) + u16_at(at + 32);
    }
    out
}

fn tiny_png() -> Vec<u8> {
    let image = ColorImage::new([4, 3], vec![Color32::from_rgb(200, 40, 40); 12]);
    metallic_studio::screenshot::png_bytes(&image).unwrap()
}

fn report(log: &str, output: &str) -> BugReport {
    BugReport {
        version: "9.8.7".into(),
        channel: "dev".into(),
        gpu: "Test GPU (Vulkan)".into(),
        os: "windows x86_64".into(),
        log: log.into(),
        output: output.into(),
        screenshot_png: Some(tiny_png()),
    }
}

#[test]
fn bug_report_zip_contains_logs_version_and_screenshot() {
    let dir = tempfile::tempdir().unwrap();
    let r = report(
        "    1.000 WARN  metallic_studio::app: hello log\n",
        "ran Probe\n",
    );
    let path = r
        .write(dir.path(), 1_793_455_509, &Redactor::default())
        .unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "2026-10-31_14-05-09.zip"
    );
    let entries = unzip(&std::fs::read(&path).unwrap());
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["version.txt", "log.txt", "output.txt", "screenshot.png"]
    );
    let text = |name: &str| {
        let (_, data) = entries.iter().find(|(n, _)| n == name).unwrap();
        String::from_utf8(data.clone()).unwrap()
    };
    assert!(text("version.txt").contains("Metallic Studio 9.8.7"));
    assert!(text("version.txt").contains("Channel: dev"));
    assert!(text("version.txt").contains("GPU: Test GPU (Vulkan)"));
    assert!(text("log.txt").contains("hello log"));
    assert!(text("output.txt").contains("ran Probe"));
    let (_, png) = entries.iter().find(|(n, _)| n == "screenshot.png").unwrap();
    let image = metallic_studio::png_image::decode_png(png, "screenshot").unwrap();
    assert_eq!((image.width, image.height), (4, 3));

    // A second report in the same second gets its own file.
    let again = r
        .write(dir.path(), 1_793_455_509, &Redactor::default())
        .unwrap();
    assert_ne!(again, path);
}

#[test]
fn bug_report_has_no_tokens() {
    // The stored session as the credential store holds it, and a token pasted into a log line.
    let stored = r#"{"access_token":"gho_SecretAccessToken0123456789abcd","refresh_token":"plainRefreshSecret77"}"#;
    let log = format!(
        "signed in with gho_SecretAccessToken0123456789abcd\n\
         refresh plainRefreshSecret77\n\
         another ghp_UnrelatedTokenShape9876543210xyz here\n\
         session {stored}\n"
    );
    let output = "token=gho_SecretAccessToken0123456789abcd\n";
    let dir = tempfile::tempdir().unwrap();
    let redactor = Redactor::new(secrets_in_stored(stored), None);
    let path = report(&log, output)
        .write(dir.path(), 1_793_455_509, &redactor)
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let all = String::from_utf8_lossy(&bytes);
    for secret in [
        "gho_SecretAccessToken0123456789abcd",
        "SecretAccessToken",
        "plainRefreshSecret77",
        "ghp_UnrelatedTokenShape9876543210xyz",
    ] {
        assert!(!all.contains(secret), "{secret} is in the zip");
    }
    let entries = unzip(&bytes);
    let (_, log) = entries.iter().find(|(n, _)| n == "log.txt").unwrap();
    let log = String::from_utf8(log.clone()).unwrap();
    assert!(log.contains("signed in with [redacted]"), "{log}");
}

fn editor() -> EditorUi {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let part = world.create("Part", Some(ws)).unwrap();
    world
        .set(part, "Name", Value::String("Probe".into()))
        .unwrap();
    let output = OutputPanel::new();
    let mut state = EditorState::new(
        world,
        Path::new("."),
        Path::new("unused.mscene"),
        "Test",
        output.log.sink(),
    );
    state.can_save = false;
    EditorUi::new(state, output)
}

#[test]
fn dev_actions_need_developer_mode_and_keep_working_in_play() {
    let mut ui = editor();
    assert!(
        !ui.actions.run("dev.overlay", &mut ui.state),
        "off by default"
    );
    assert!(!ui.state.dev.overlay);
    ui.state.dev.enabled = true;
    assert!(ui.actions.run("dev.overlay", &mut ui.state));
    assert!(ui.state.dev.overlay);
    assert!(ui.state.start_play("play.start"));
    assert!(
        !ui.actions.run("edit.select_all", &mut ui.state),
        "editing is locked"
    );
    assert!(
        ui.actions.run("dev.inspector", &mut ui.state),
        "dev tools are not"
    );
    assert!(ui.state.dev.inspector);
    ui.state.stop_play();
    assert_eq!(ui.keymap.label("dev.overlay"), "F3");
}

#[test]
fn a_later_dev_action_is_gated_and_listed_like_the_built_in_ones() {
    use metallic_studio::actions::{Action, ToolbarGroup};
    use metallic_studio::icons::Icon;
    let mut ui = editor();
    // What the Profiler (#23) or the debugger (#22) does to join developer mode.
    ui.actions.register(Action::new(
        "dev.profiler",
        "Profiler",
        Icon::Text("P"),
        ToolbarGroup::Hidden,
        |s| s.dev.log = true,
    ));
    let a = ui.actions.get("dev.profiler").unwrap();
    assert!(!a.is_enabled(&ui.state));
    ui.state.dev.enabled = true;
    assert!(
        ui.actions
            .get("dev.profiler")
            .unwrap()
            .is_enabled(&ui.state)
    );
    let listed: Vec<&str> = ui
        .actions
        .iter()
        .filter(|a| metallic_studio::devmode::is_dev_action(a.id))
        .map(|a| a.id)
        .collect();
    assert_eq!(
        listed,
        [
            "dev.overlay",
            "dev.log",
            "dev.inspector",
            "dev.flags",
            "dev.report_bug",
            "dev.profiler"
        ]
    );
}

fn harness(ui: EditorUi) -> Harness<'static, EditorUi> {
    let mut themed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(
            move |ui, editor: &mut EditorUi| {
                if !themed {
                    metallic_studio::theme::apply(
                        ui.ctx(),
                        metallic_studio::theme::ThemeId::Graphite,
                    );
                    themed = true;
                    ui.ctx().request_repaint();
                    return;
                }
                editor.ui(ui, None);
            },
            ui,
        );
    h.run_steps(3);
    h
}

#[test]
fn f3_shows_the_overlay_and_the_windows_draw() {
    let mut ui = editor();
    ui.state.dev.enabled = true;
    let mut h = harness(ui);
    h.key_press(Key::F3);
    h.run_steps(3);
    assert!(h.state().state.dev.overlay);
    h.get_by_label_contains("parts");
    h.get_by_label_contains("Heap");
    // The windows open and show what they are about.
    let probe = {
        let s = &h.state().state;
        let ws = s.world.root("Workspace").unwrap();
        s.world.children(ws)[0]
    };
    h.state_mut().state.selection.select_only(probe);
    h.state_mut().state.dev.inspector = true;
    h.state_mut().state.dev.flags = true;
    h.state_mut().state.dev.log = true;
    metallic_studio::devmode::log_view::push(log::Level::Warn, "devmode_test", "devmode-test-line");
    h.run_steps(3);
    h.get_by_label_contains("Workspace.Probe");
    h.get_by_label("No flags yet.");
    h.get_by_label_contains("devmode-test-line");
    // Developer mode off hides everything again.
    h.state_mut().state.dev.enabled = false;
    h.run_steps(3);
    assert!(h.query_by_label_contains("Heap").is_none());
    assert!(h.query_by_label("No flags yet.").is_none());
}
