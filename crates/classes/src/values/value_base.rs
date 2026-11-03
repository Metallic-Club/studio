//! `ValueBase`: the base of the value objects, instances that hold one value under a name.
//!
//! Each subclass (`BoolValue`, `IntValue`, `NumberValue`, `StringValue`, `ObjectValue`) declares its own `Value`
//! property with its own type, so `ValueBase` itself has no members: it exists so scripts can ask
//! `obj:IsA("ValueBase")`.
//!
//! On value objects, `Changed` fires when `Value` changes and passes the new value instead of a property name, so
//! `ValueBase` re-declares the signal with that parameter (replacing `Object`'s declaration) and declares
//! `changed_passes_value("Value")`, which every subclass inherits. The data model then fires `Changed(newValue)`
//! when `Value` changes, and no `Changed` for other changes (a rename or a move); `GetPropertyChangedSignal` still
//! fires for every property.

use metallic_core::{ClassDesc, Param, SignalDesc, Ty};

/// The category value objects are listed under in Studio's Insert menu.
pub const CATEGORY: &str = "Values";

/// The base of the value objects.
pub static CLASS: ClassDesc = ClassDesc::new("ValueBase", Some("Instance"))
    .describe(
        "The base of the value objects: instances that store one value, such as a score or a door's target, where \
         scripts can share it.",
    )
    .signals(&[SignalDesc::new("Changed")
        .params(&[Param::new("value", Ty::Any)])
        .describe("Fires after Value changes, with the new value.")])
    .changed_passes_value("Value");

#[cfg(test)]
mod tests {
    use metallic_core::{Delivery, EnumItem, ListenerKey, Param, ScriptValue, Ty, Value};

    use crate::shared_registry;
    use crate::testing::{call, game, new_in, workspace};

    #[test]
    fn every_value_object_has_its_value() {
        let mut dm = game();
        let ws = workspace(&dm);
        let folder = new_in(&mut dm, "Folder", ws, "Target");
        let cases = [
            ("BoolValue", Value::Bool(false), Value::Bool(true)),
            ("IntValue", Value::Int(0), Value::Int(-42)),
            ("NumberValue", Value::Number(0.0), Value::Number(2.5)),
            ("StringValue", Value::String("".into()), Value::String("hello".into())),
            ("ObjectValue", Value::Ref(None), Value::Ref(Some(folder))),
        ];
        for (class, default, written) in cases {
            let value = new_in(&mut dm, class, ws, class);
            assert_eq!(dm.get_by_name(value, "Value"), Ok(default), "{class}");
            dm.set_by_name(value, "Value", written.clone()).unwrap();
            assert_eq!(dm.get_by_name(value, "Value"), Ok(written), "{class}");
            assert_eq!(
                call(&mut dm, value, "IsA", vec!["ValueBase".into()]),
                Ok(true.into()),
                "{class}"
            );
        }
    }

    #[test]
    fn value_objects_check_their_type() {
        let mut dm = game();
        let ws = workspace(&dm);
        let number = new_in(&mut dm, "NumberValue", ws, "N");
        assert!(dm.set_by_name(number, "Value", Value::Bool(true)).is_err());
        assert!(dm.set_by_name(number, "Value", Value::Number(f64::NAN)).is_err());
        let int = new_in(&mut dm, "IntValue", ws, "I");
        assert!(dm.set_by_name(int, "Value", Value::Number(1.5)).is_err());
        let string = new_in(&mut dm, "StringValue", ws, "S");
        let item = Value::Enum(EnumItem::new("PartShape", "Ball"));
        assert!(dm.set_by_name(string, "Value", item).is_err());
    }

    #[test]
    fn value_base_is_abstract() {
        let registry = shared_registry().unwrap();
        let class = registry.class_id("ValueBase").unwrap();
        assert!(!registry.class(class).unwrap().creatable());
    }

    /// `(listener, args)` of every queued delivery.
    fn drain(dm: &mut metallic_core::DataModel) -> Vec<(u64, Vec<ScriptValue>)> {
        let mut out: Vec<Delivery> = Vec::new();
        dm.drain_deliveries(&mut out);
        out.into_iter().map(|d| (d.listener.0, d.args)).collect()
    }

    #[test]
    fn changed_fires_with_the_new_value() {
        let mut dm = game();
        let ws = workspace(&dm);
        let number = new_in(&mut dm, "NumberValue", ws, "N");
        let changed = dm.signal_id(number, "Changed").unwrap();
        let value = dm.prop_id(number, "Value").unwrap();
        dm.connect(number, changed, ListenerKey(1)).unwrap();
        dm.connect_property_changed(number, value, ListenerKey(2)).unwrap();
        dm.set(number, value, Value::Number(5.0)).unwrap();
        dm.set_name(number, "Score".into()).unwrap();
        assert_eq!(
            drain(&mut dm),
            vec![(1, vec![ScriptValue::Number(5.0)]), (2, vec![])],
            "Changed(5) and Value's own signal; a rename fires no Changed"
        );

        let object = new_in(&mut dm, "ObjectValue", ws, "O");
        let folder = new_in(&mut dm, "Folder", ws, "Target");
        dm.connect(object, changed, ListenerKey(3)).unwrap();
        dm.set_by_name(object, "Value", Value::Ref(Some(folder))).unwrap();
        assert_eq!(drain(&mut dm), vec![(3, vec![ScriptValue::Instance(folder)])]);

        let part = new_in(&mut dm, "Part", ws, "P");
        dm.connect(part, changed, ListenerKey(4)).unwrap();
        dm.set_by_name(part, "Transparency", Value::Number(0.5)).unwrap();
        assert_eq!(drain(&mut dm), vec![(4, vec![ScriptValue::String("Transparency".into())])]);
    }

    #[test]
    fn changed_passes_the_new_value() {
        let registry = shared_registry().unwrap();
        let int = registry.class_id("IntValue").unwrap();
        assert_eq!(
            registry.signal(int, "Changed").unwrap().params,
            Some(&[Param::new("value", Ty::Any)][..])
        );
        let folder = registry.class_id("Folder").unwrap();
        assert_eq!(
            registry.signal(folder, "Changed").unwrap().params,
            Some(&[Param::new("propertyName", Ty::String)][..])
        );
    }
}
