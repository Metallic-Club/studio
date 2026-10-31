//! Network ids: the server's name for a replicated instance, the same on every client.

use serde::{Deserialize, Serialize};

/// A replicated instance's id on the wire. Built-in services have fixed ids below [`FIRST_DYNAMIC_NET_ID`]; the
/// server hands out the rest from 64 upward and never reuses one (plan ruling R6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct NetId(pub u64);

/// The first id the server gives to an instance that is not a built-in service.
pub const FIRST_DYNAMIC_NET_ID: u64 = 64;

/// The fixed id of a built-in service, by class name; `None` for anything else.
pub fn service_net_id(class: &str) -> Option<NetId> {
    let id = match class {
        "Workspace" => 1,
        "Players" => 2,
        "ReplicatedStorage" => 3,
        "ServerScriptService" => 4,
        "StarterPlayer" => 5,
        "StarterGui" => 6,
        "Lighting" => 7,
        "RunService" => 8,
        "ServerStorage" => 9,
        "StarterPlayerScripts" => 10,
        _ => return None,
    };
    Some(NetId(id))
}
