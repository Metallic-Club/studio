//! `BindableEvent`: a signal scripts fire themselves, to talk to other scripts on the same side.

use metallic_core::{ClassDesc, MethodCall, MethodDesc, Param, ScriptError, ScriptValue, SignalDesc, Ty};

/// Any number of values of any type: what `Fire` takes and `Event` passes on unchanged.
static ARGS: &[Param] = &[Param::new("args", Ty::Variadic(&Ty::Any))];

/// Fires `Event` with every argument, through the data model, so nothing is queued while nothing listens.
fn fire(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let args = std::mem::take(&mut call.args).into_vec();
    call.dm.fire_by_name(call.this, "Event", args)?;
    Ok(ScriptValue::Nil)
}

/// A signal scripts fire themselves.
pub static CLASS: ClassDesc = ClassDesc::new("BindableEvent", Some("Instance"))
    .describe(
        "A signal that scripts fire themselves: one script calls Fire, and every function connected to Event \
         runs with the same arguments. It works between scripts on the same side (server or client).",
    )
    .category("Events")
    .creatable()
    .methods(&[MethodDesc::new("Fire", fire)
        .params(ARGS)
        .returns_nothing()
        .describe(
            "Fires Event with all the arguments given. Listeners run later in the frame, not inside Fire; when \
             nothing is connected, nothing happens.",
        )])
    .signals(&[SignalDesc::new("Event")
        .params(ARGS)
        .describe("Fires when Fire is called, with the arguments passed to Fire.")]);

#[cfg(test)]
mod tests {
    use metallic_core::{ListenerKey, ScriptValue};

    use crate::shared_registry;
    use crate::testing::{call, game, new_in, workspace};

    #[test]
    fn bindable_event_is_a_creatable_instance() {
        let registry = shared_registry().unwrap();
        let class = registry.class_id("BindableEvent").unwrap();
        let info = registry.class(class).unwrap();
        let chain: Vec<&str> = info
            .ancestors()
            .iter()
            .map(|&c| registry.class(c).unwrap().name())
            .collect();
        assert_eq!(chain, ["Object", "Instance", "BindableEvent"]);
        assert!(info.creatable());
        assert!(registry.method(class, "Fire").is_some());
        assert_eq!(registry.signal(class, "Event").unwrap().params, Some(super::ARGS));
        let fire = registry.method(class, "Fire").unwrap();
        assert_eq!(fire.params, Some(super::ARGS));
        assert_eq!(super::ARGS[0].to_string(), "...: any");
    }

    #[test]
    fn fire_queues_its_arguments_for_a_listener() {
        let mut dm = game();
        let ws = workspace(&dm);
        let event = new_in(&mut dm, "BindableEvent", ws, "Opened");
        let signal = dm.signal_id(event, "Event").unwrap();
        let listener = ListenerKey(7);
        let connection = dm.connect(event, signal, listener).unwrap();
        let mut out = Vec::new();
        dm.drain_deliveries(&mut out);
        out.clear();

        let args = vec![ScriptValue::Number(1.0), "a".into()];
        assert_eq!(call(&mut dm, event, "Fire", args.clone()), Ok(ScriptValue::Nil));
        assert_eq!(dm.pending_events(), 1);
        dm.drain_deliveries(&mut out);
        assert_eq!(out.len(), 1);
        let delivery = &out[0];
        assert_eq!(delivery.connection, connection);
        assert_eq!(delivery.listener, listener);
        assert_eq!(delivery.source, event);
        assert_eq!(delivery.args, args);
    }

    #[test]
    fn fire_with_no_arguments_delivers_none() {
        let mut dm = game();
        let ws = workspace(&dm);
        let event = new_in(&mut dm, "BindableEvent", ws, "Ping");
        let signal = dm.signal_id(event, "Event").unwrap();
        dm.connect(event, signal, ListenerKey(1)).unwrap();
        let mut out = Vec::new();
        dm.drain_deliveries(&mut out);
        out.clear();

        call(&mut dm, event, "Fire", vec![]).unwrap();
        dm.drain_deliveries(&mut out);
        assert_eq!(out.len(), 1);
        assert!(out[0].args.is_empty());
    }

    #[test]
    fn fire_queues_nothing_when_nothing_listens() {
        let mut dm = game();
        let ws = workspace(&dm);
        let event = new_in(&mut dm, "BindableEvent", ws, "Unheard");
        let mut out = Vec::new();
        dm.drain_deliveries(&mut out);
        assert_eq!(dm.pending_events(), 0);

        let args = vec![ScriptValue::Number(1.0), "a".into()];
        assert_eq!(call(&mut dm, event, "Fire", args), Ok(ScriptValue::Nil));
        assert_eq!(dm.pending_events(), 0);
    }
}
