//! The reference load of the input spec (section 13), shared by the allocation test and the
//! `input_eval` bench: 4 contexts, 32 actions, 96 bindings and 40 raw events a frame.
use metallic_runtime::InputRuntime;
use metallic_runtime::input::{
    ActionDef, ActionType, BindingDef, ContextDef, Control, Device, InputConfig, RawEvent, Trigger,
};

const KEYS: [Control; 12] = [
    Control::A,
    Control::B,
    Control::C,
    Control::D,
    Control::E,
    Control::F,
    Control::G,
    Control::H,
    Control::I,
    Control::J,
    Control::K,
    Control::L,
];

/// 4 contexts x 8 actions x 3 bindings, every trigger and value type in use, one sinking context.
pub fn config() -> InputConfig {
    let triggers = [
        Trigger::Press,
        Trigger::Release,
        Trigger::Hold,
        Trigger::Tap,
        Trigger::DoubleTap,
        Trigger::Repeat,
    ];
    let mut contexts = Vec::new();
    for c in 0..4usize {
        let mut ctx = ContextDef::new(&format!("Ctx{c}"));
        ctx.priority = c as f64;
        ctx.sink = c == 3;
        for a in 0..8usize {
            let n = c * 8 + a;
            let ty = match a % 4 {
                0 | 1 => ActionType::Bool,
                2 => ActionType::Direction2D,
                _ => ActionType::Direction1D,
            };
            let mut action = ActionDef::new(&format!("Action{a}"), ty)
                .with_trigger(triggers[n % triggers.len()]);
            for b in 0..3usize {
                let k = KEYS[(n + b) % KEYS.len()];
                action = action.with_binding(match (ty, b) {
                    (ActionType::Direction2D, 0) => BindingDef::composite(
                        KEYS[n % 12],
                        KEYS[(n + 1) % 12],
                        KEYS[(n + 2) % 12],
                        KEYS[(n + 3) % 12],
                    ),
                    (ActionType::Direction2D, 1) => BindingDef::key(Control::Thumbstick1),
                    (ActionType::Direction2D, _) => BindingDef::key(Control::MouseDelta)
                        .with_modifier(Control::MouseButton2)
                        .with_scale(0.006),
                    (ActionType::Direction1D, 0) => BindingDef::key(Control::MouseWheel),
                    (ActionType::Direction1D, 1) => BindingDef::key(Control::ButtonR2),
                    _ => BindingDef::key(k).with_modifier(Control::LeftShift),
                });
            }
            ctx.actions.push(action);
        }
        contexts.push(ctx);
    }
    InputConfig {
        contexts,
        warnings: Vec::new(),
    }
}

/// Pushes the 40 raw events of frame `f`, which ends at time `t`.
pub fn push_frame(rt: &mut InputRuntime, f: u64, t: f64) {
    for i in 0..30u64 {
        let key = KEYS[((f + i) % 12) as usize];
        let down = (f / 3 + i).is_multiple_of(2);
        let value = f32::from(u8::from(down));
        rt.push(RawEvent::button(Device::KeyboardMouse, key, value, t));
    }
    for i in 0..4 {
        let a = (f + i) as f32 * 0.1;
        let stick = [a.sin(), a.cos()];
        rt.push(RawEvent::vector(
            Device::Gamepad(0),
            Control::Thumbstick1,
            stick,
            t,
        ));
    }
    for i in 0..4 {
        let delta = [3.0, -2.0 + i as f32];
        rt.push(RawEvent::vector(
            Device::KeyboardMouse,
            Control::MouseDelta,
            delta,
            t,
        ));
    }
    let right = (f % 2) as f32;
    rt.push(RawEvent::button(
        Device::KeyboardMouse,
        Control::MouseButton2,
        right,
        t,
    ));
    rt.push(RawEvent::wheel(Device::KeyboardMouse, 1.0, t));
}
