//! `DataModel`: the root of the game tree (`game` in scripts). Every service sits directly under it.

use metallic_core::{ClassDesc, MethodCall, MethodDesc, ScriptError, ScriptValue};

use crate::script_args::class_arg;

fn get_service(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let class = class_arg(call, 0)?;
    if let Some(service) = call.dm.service(class) {
        return Ok(service.into());
    }
    if let Some(info) = call.dm.registry().class(class)
        && info.service_order().is_none()
    {
        return Err(ScriptError::runtime(format!("'{}' is not a service", info.name())));
    }
    // A service the game was started without is made on first use; it still exists only once.
    Ok(call.dm.create_service(class)?.into())
}

/// The root of the game tree.
pub static CLASS: ClassDesc = ClassDesc::new("DataModel", Some("Instance"))
    .describe("The whole game (`game` in scripts). Its children are the services, such as Workspace and Lighting.")
    .methods(&[MethodDesc::new("GetService", get_service).describe(
        "Returns the service with the given class name, such as game:GetService(\"Lighting\"). It always returns the \
         same instance for a service.",
    )]);

#[cfg(test)]
mod tests {
    use metallic_core::{ScriptValue, Value};

    use crate::testing::{call, game, new_in, workspace};

    #[test]
    fn get_service_returns_the_same_instance() {
        let mut dm = game();
        let root = dm.root().unwrap();
        let lighting = call(&mut dm, root, "GetService", vec!["Lighting".into()]).unwrap();
        assert_eq!(dm.children(root).unwrap()[1], match lighting {
            ScriptValue::Instance(id) => id,
            other => panic!("expected an instance, got {other:?}"),
        });
        assert_eq!(call(&mut dm, root, "GetService", vec!["Lighting".into()]), Ok(lighting));
        assert_eq!(
            call(&mut dm, root, "GetService", vec!["Workspace".into()]),
            Ok(ScriptValue::Instance(workspace(&dm)))
        );
    }

    #[test]
    fn get_service_refuses_other_classes() {
        let mut dm = game();
        let root = dm.root().unwrap();
        let folder = call(&mut dm, root, "GetService", vec!["Folder".into()]).unwrap_err();
        assert_eq!(folder.to_string(), "'Folder' is not a service");
        let nope = call(&mut dm, root, "GetService", vec!["Nope".into()]).unwrap_err();
        assert_eq!(nope.to_string(), "unknown class 'Nope'");
        assert!(call(&mut dm, root, "GetService", vec![]).is_err());
    }

    #[test]
    fn services_cannot_be_destroyed_or_moved() {
        let mut dm = game();
        let root = dm.root().unwrap();
        let ws = workspace(&dm);
        let folder = new_in(&mut dm, "Folder", ws, "F");
        let services: Vec<_> = dm.children(root).unwrap().to_vec();
        for service in services {
            assert!(call(&mut dm, service, "Destroy", vec![]).is_err());
            assert!(dm.set_by_name(service, "Parent", Value::Ref(Some(folder))).is_err());
            assert!(dm.set_by_name(service, "Parent", Value::Ref(None)).is_err());
            assert_eq!(dm.parent(service), Ok(Some(root)));
        }
        assert!(call(&mut dm, root, "Destroy", vec![]).is_err());
        // Clearing the root's children keeps the services.
        assert_eq!(call(&mut dm, root, "ClearAllChildren", vec![]), Ok(ScriptValue::Nil));
        assert_eq!(dm.children(root).unwrap().len(), 5);
        assert!(dm.contains(folder));
    }
}
