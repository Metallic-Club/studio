//! Headless egui tests of the popover: keyboard-only use, AccessKit labels, commit rules and the
//! focus trap. They drive `egui::Context::run_ui` with synthetic input; no window is opened.

use eframe::egui::{
    self, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, accesskit, pos2, vec2,
};

use super::*;

struct Harness {
    ctx: egui::Context,
    popover: ColourPopover,
    library: ColourLibrary,
    time: f64,
    events: Vec<ColourEvent>,
    last_accesskit: Option<accesskit::TreeUpdate>,
}

impl Harness {
    fn open(colour: Rgb8, targets: usize) -> Self {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        ctx.enable_accesskit();
        let mut popover = ColourPopover::new("test");
        popover.open(colour, targets, false, pos2(40.0, 40.0));
        let mut h = Self {
            ctx,
            popover,
            library: ColourLibrary::default(),
            time: 0.0,
            events: Vec::new(),
            last_accesskit: None,
        };
        h.frame(Vec::new());
        h.frame(Vec::new());
        h.events.clear();
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (popover, library, sink) = (&mut self.popover, &mut self.library, &mut self.events);
        let mut out = self.ctx.run_ui(input, |ui| {
            sink.extend(popover.show(ui.ctx(), library));
        });
        // No renderer here: drop the font atlas uploads.
        out.textures_delta.clear();
        if out.platform_output.accesskit_update.is_some() {
            self.last_accesskit = out.platform_output.accesskit_update;
        }
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        let ev = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        // Modifiers reach egui as their own event before the key, as from a real keyboard.
        self.frame(vec![
            Event::ModifiersChanged(modifiers),
            ev(true),
            ev(false),
        ]);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }

    fn tab(&mut self, times: usize) {
        for _ in 0..times {
            self.key(Key::Tab, Modifiers::NONE);
        }
    }

    fn type_text(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.to_owned())]);
    }

    fn wait(&mut self, seconds: f64) {
        self.time += seconds;
        self.frame(Vec::new());
    }

    fn commits(&self) -> Vec<(Rgb8, String)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                ColourEvent::Commit { colour, undo_name } => Some((*colour, undo_name.clone())),
                _ => None,
            })
            .collect()
    }

    /// (label, value) of every AccessKit node with a label.
    fn accesskit_labels(&self) -> Vec<(String, Option<String>)> {
        self.last_accesskit
            .as_ref()
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| {
                        n.label()
                            .map(|l| (l.to_owned(), n.value().map(str::to_owned)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn pointer(&mut self, pos: Pos2, pressed: Option<bool>) {
        let mut events = vec![Event::PointerMoved(pos)];
        if let Some(pressed) = pressed {
            events.push(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        self.frame(events);
    }
}

#[test]
fn keyboard_only_sets_5280cc() {
    let mut h = Harness::open(Rgb8::new(128, 128, 128), 3);
    // Tab order: ring (focused on open), square, old/new chip, Hex.
    h.tab(3);
    h.type_text("5280CC");
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(h.popover.colour(), Rgb8::new(0x52, 0x80, 0xCC));
    assert_eq!(
        h.commits(),
        [(
            Rgb8::new(0x52, 0x80, 0xCC),
            "Colour: #5280CC (3 parts)".to_owned()
        )]
    );
    assert_eq!(h.library.recent.items()[0], Rgb8::new(0x52, 0x80, 0xCC));
}

#[test]
fn keyboard_only_sets_a_css_name_and_hsv_fields() {
    let mut h = Harness::open(Rgb8::new(128, 128, 128), 1);
    h.tab(3);
    h.type_text("steel blue");
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(h.popover.colour(), Rgb8::new(70, 130, 180));

    // Hex -> R G B -> H: type an expression into the hue field.
    let mut h = Harness::open(Rgb8::new(204, 82, 82), 1);
    h.tab(7);
    h.type_text("200+18");
    h.key(Key::Enter, Modifiers::NONE);
    let hsv = h.popover.hsv();
    assert_eq!(hsv.h.round(), 218.0);
    assert_eq!(h.commits().len(), 1);
}

#[test]
fn accesskit_reads_hue_218_degrees() {
    let mut h = Harness::open(Rgb8::new(0x52, 0x80, 0xCC), 2);
    // #5280CC has hue 217.4; one step right on the focused ring gives 218.
    h.key(Key::ArrowRight, Modifiers::NONE);
    let labels = h.accesskit_labels();
    let hue = labels
        .iter()
        .find(|(l, _)| l == "Hue")
        .unwrap_or_else(|| panic!("no Hue node in {labels:?}"));
    let value = hue.1.as_deref().unwrap_or_default();
    assert!(value.starts_with("218 degrees, close to "), "{value}");
    let square = labels
        .iter()
        .find(|(l, _)| l == "Saturation and value")
        .and_then(|(_, v)| v.clone())
        .unwrap_or_default();
    assert!(
        square.starts_with("Saturation 60 percent, value 80 percent"),
        "{square}"
    );
}

#[test]
fn ring_keys_follow_the_slider_pattern() {
    let mut h = Harness::open(Rgb8::from_u32(0xFF0000), 1);
    let hue = |h: &Harness| h.popover.hsv().h.round();
    h.key(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(hue(&h), 1.0);
    h.key(Key::ArrowUp, Modifiers::SHIFT);
    assert_eq!(hue(&h), 11.0);
    h.key(Key::PageUp, Modifiers::NONE);
    assert_eq!(hue(&h), 41.0);
    h.key(Key::End, Modifiers::NONE);
    assert_eq!(hue(&h), 359.0);
    h.key(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(hue(&h), 0.0, "wraps");
    h.key(Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(hue(&h), 359.0);
    h.key(Key::Home, Modifiers::NONE);
    assert_eq!(hue(&h), 0.0);
}

#[test]
fn square_keys_move_saturation_and_value() {
    let mut h = Harness::open(Hsv::new(200.0, 0.5, 0.5).to_rgb8(), 1);
    let hue = h.popover.hsv().h;
    h.tab(1);
    let sv = |h: &Harness| {
        let c = h.popover.hsv();
        ((c.s * 100.0).round(), (c.v * 100.0).round())
    };
    h.key(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(sv(&h), (51.0, 50.0));
    h.key(Key::ArrowUp, Modifiers::SHIFT);
    assert_eq!(sv(&h), (51.0, 60.0));
    h.key(Key::PageDown, Modifiers::NONE);
    assert_eq!(sv(&h), (51.0, 50.0));
    h.key(Key::End, Modifiers::NONE);
    assert_eq!(sv(&h), (61.0, 50.0));
    h.key(Key::Home, Modifiers::NONE);
    assert_eq!(sv(&h), (51.0, 50.0));
    assert_eq!(h.popover.hsv().h, hue, "hue is untouched");
}

#[test]
fn keyboard_edits_merge_and_commit_after_a_pause() {
    let mut h = Harness::open(Rgb8::from_u32(0xFF0000), 2);
    for _ in 0..5 {
        h.key(Key::ArrowRight, Modifiers::NONE);
    }
    assert!(h.commits().is_empty(), "no commit while typing");
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, ColourEvent::Preview(_)))
    );
    h.wait(0.7);
    let commits = h.commits();
    assert_eq!(commits.len(), 1, "five key presses, one undo step");
    assert_eq!(commits[0].0, Hsv::new(5.0, 1.0, 1.0).to_rgb8());
}

#[test]
fn escape_drops_the_preview_and_closes() {
    let mut h = Harness::open(Rgb8::from_u32(0x336699), 1);
    h.key(Key::ArrowRight, Modifiers::NONE);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.commits().is_empty());
    assert!(h.events.contains(&ColourEvent::Cancel));
    assert!(!h.popover.is_open());
}

#[test]
fn a_ring_drag_is_one_undo_step() {
    let mut h = Harness::open(Rgb8::from_u32(0x336699), 4);
    let c = h.popover.wheel_centre();
    let r = RING / 2.0 - RING_THICKNESS / 2.0;
    h.pointer(c + vec2(0.0, -r), None);
    h.pointer(c + vec2(0.0, -r), Some(true));
    for i in 1..=10 {
        let a = (i as f32 * 9.0 - 90.0).to_radians();
        h.pointer(c + vec2(a.cos(), a.sin()) * r, None);
    }
    let end = c + vec2(r, 0.0);
    h.pointer(end, Some(false));
    h.frame(Vec::new());
    let commits = h.commits();
    assert_eq!(commits.len(), 1, "{:?}", h.events);
    assert!(
        h.events
            .iter()
            .filter(|e| matches!(e, ColourEvent::Preview(_)))
            .count()
            > 3
    );
    assert!(
        (h.popover.hsv().h - 90.0).abs() < 1.0,
        "hue {}",
        h.popover.hsv().h
    );
    assert!(commits[0].1.ends_with("(4 parts)"));
}

#[test]
fn a_square_drag_sets_saturation_and_value() {
    let mut h = Harness::open(Hsv::new(120.0, 0.2, 0.2).to_rgb8(), 1);
    let c = h.popover.wheel_centre();
    let half = SQUARE / 2.0;
    let target = c + vec2(half, -half); // top right: full saturation and value
    h.pointer(c, None);
    h.pointer(c, Some(true));
    h.pointer(target, None);
    h.pointer(target, Some(false));
    h.frame(Vec::new());
    assert_eq!(h.popover.colour(), Rgb8::new(0, 255, 0));
    assert_eq!(h.commits().len(), 1);
}

#[test]
fn shift_tab_from_the_ring_wraps_to_close() {
    let mut h = Harness::open(Rgb8::from_u32(0x336699), 1);
    h.key(Key::Tab, Modifiers::SHIFT);
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.events.contains(&ColourEvent::Closed), "{:?}", h.events);
    assert!(!h.popover.is_open());
}

#[test]
fn the_eyedropper_reports_clicks_outside_the_popover() {
    let mut h = Harness::open(Rgb8::from_u32(0x336699), 1);
    h.key(Key::I, Modifiers::NONE);
    assert!(h.popover.eyedropper_armed());
    let outside = pos2(1000.0, 700.0);
    h.pointer(outside, None);
    h.pointer(outside, Some(true));
    h.pointer(outside, Some(false));
    assert!(
        h.events.iter().any(|e| matches!(
            e,
            ColourEvent::EyedropperClick { pos, alt: false, ctrl: false } if *pos == outside
        )),
        "{:?}",
        h.events
    );
    let (popover, library) = (&mut h.popover, &mut h.library);
    popover.pick(Rgb8::from_u32(0xC0392B), library);
    assert!(!h.popover.eyedropper_armed());
    h.frame(Vec::new());
    assert_eq!(
        h.commits().last().map(|c| c.0),
        Some(Rgb8::from_u32(0xC0392B))
    );
}

#[test]
fn eyedropper_escape_disarms_without_closing() {
    let mut h = Harness::open(Rgb8::from_u32(0x336699), 1);
    h.key(Key::I, Modifiers::NONE);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(!h.popover.eyedropper_armed());
    assert!(h.popover.is_open());
}

#[test]
fn tab_order_reaches_the_palette_plus_and_enter_saves_the_colour() {
    let mut h = Harness::open(Rgb8::from_u32(0x5280CC), 1);
    // ring, square, chip, Hex, R, G, B, H, S, V, eyedropper, palette menu, + (Recent is empty).
    h.tab(12);
    h.key(Key::Enter, Modifiers::NONE);
    let palettes = h.library.palettes.palettes();
    assert_eq!(
        palettes.len(),
        1,
        "a palette is created for the first colour"
    );
    assert_eq!(palettes[0].name, "Palette 1");
    assert_eq!(palettes[0].colours, [Rgb8::from_u32(0x5280CC)]);
    assert!(
        h.commits().is_empty(),
        "saving a swatch is not a scene edit"
    );
}

#[test]
fn pixel_lookup_respects_scale_and_bounds() {
    let mut image = egui::ColorImage::filled([4, 2], egui::Color32::BLACK);
    image.pixels[4 + 3] = egui::Color32::from_rgb(1, 2, 3);
    assert_eq!(
        popover::pixel_at(&image, pos2(1.5, 0.5), 2.0),
        Some(Rgb8::new(1, 2, 3))
    );
    assert_eq!(popover::pixel_at(&image, pos2(2.0, 0.0), 2.0), None);
    assert_eq!(popover::pixel_at(&image, pos2(-0.1, 0.0), 1.0), None);
}

#[test]
fn the_popover_has_the_spec_size() {
    let h = Harness::open(Rgb8::from_u32(0x336699), 1);
    let size = h.popover.rect().size();
    assert_eq!(size.x, popover::WIDTH, "{size:?}");
    assert!((size.y - popover::HEIGHT).abs() <= 1.0, "{size:?}");
}

use popover::{RING, RING_THICKNESS, SQUARE};
