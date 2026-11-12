//! Go to definition (#532): where a name under the pointer is declared, in its own script or another.
//!
//! A local, a parameter, a loop variable or a function of the file is where it is declared; a field of a table of the
//! file is where it is set; a member of what a ModuleScript returns (through `require`) is in that script's code; and a
//! child of the game that is a script is that script.

use std::ops::Range;

use metallic_engine::metallic_core::Id;

use super::analysis::{Analysis, Env, Ref, Ty};
use super::lexer::TokenKind;

/// Where a definition is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// These bytes of the same code.
    Here(Range<usize>),
    /// These bytes of the script `Id`'s code (its start, for the script itself).
    Script(Id, Range<usize>),
}

/// Where the name at byte `at` of the code `analysis` read is defined. `None` for a declaration itself, a name of the
/// API, and anything not known.
pub fn definition(analysis: &Analysis, at: usize, env: &Env<'_>) -> Option<Target> {
    let index = analysis.token_at(at)?;
    let token = *analysis.tokens.get(index)?;
    if token.kind != TokenKind::Name {
        return None;
    }
    let name = token.text(&analysis.source);
    if index > 0 && (analysis.text(index - 1) == "." || analysis.text(index - 1) == ":") {
        let first = analysis.chain_start(index - 1)?;
        return match analysis.type_before(first, index - 1, env) {
            Ty::Table(owner) => analysis
                .field(owner, name)
                .map(|field| Target::Here(analysis.decls[field].range.clone())),
            Ty::ModuleTable(module, owner) => {
                let code = (env.module)(module)?;
                code.field(owner, name)
                    .map(|field| Target::Script(module, code.decls[field].range.clone()))
            }
            Ty::Live(id) => {
                let child = env.child(id, name)?;
                super::is_script(env.dm, child).then_some(Target::Script(child, 0..0))
            }
            _ => None,
        };
    }
    match analysis.refs.get(&index)? {
        Ref::Decl(decl) => {
            let declaration = &analysis.decls[*decl];
            // A method's `self` is declared nowhere in the text.
            if declaration.range.is_empty() || declaration.range == (token.start..token.end) {
                return None;
            }
            Some(Target::Here(declaration.range.clone()))
        }
        Ref::Global => None,
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use metallic_engine::metallic_core::DataModel;

    use super::*;

    fn game() -> DataModel {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let scene = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d/Scenes/Main.scene"),
        )
        .unwrap();
        metallic_engine::metallic_project::load_scene_bytes(&mut dm, &scene).unwrap();
        dm
    }

    #[test]
    fn a_name_goes_to_its_declaration_here_or_in_a_module() {
        let mut dm = game();
        let registry = dm.registry().clone();
        let storage = dm.service(registry.class_id("ReplicatedStorage").unwrap()).unwrap();
        let module = dm.create(registry.class_id("ModuleScript").unwrap()).unwrap();
        dm.set_name(module, "Doors".into()).unwrap();
        dm.set_parent(module, Some(storage)).unwrap();
        let module_source = "local Doors = {}\nfunction Doors.open(by) return by end\nreturn Doors\n";
        let code = Rc::new(Analysis::new(module_source));
        let modules = move |id: Id| (id == module).then(|| Rc::clone(&code));
        let env = Env {
            dm: &dm,
            script: None,
            module: &modules,
        };
        let source = "\
local Doors = require(game:GetService(\"ReplicatedStorage\").Doors)
local count = 0
local M = {}
function M.bump() count += 1 end
M.bump()
Doors.open(count)
print(math.pi)
";
        let analysis = Analysis::new(source);
        let at = |needle: &str, n: usize| source.match_indices(needle).nth(n).unwrap().0;
        let here = |range: Range<usize>| Target::Here(range);
        // A local, used: where it is declared.
        assert_eq!(
            definition(&analysis, at("count", 1), &env),
            Some(here(at("count", 0)..at("count", 0) + 5))
        );
        // A field of the file's table.
        assert_eq!(
            definition(&analysis, at("bump", 1), &env),
            Some(here(at("bump", 0)..at("bump", 0) + 4))
        );
        // A module's member: in its code.
        let open = module_source.find("open").unwrap();
        assert_eq!(
            definition(&analysis, at("open", 0), &env),
            Some(Target::Script(module, open..open + 4))
        );
        // A child that is a script: the script.
        assert_eq!(
            definition(&analysis, at("Doors)", 0), &env),
            Some(Target::Script(module, 0..0))
        );
        // A declaration itself, and the API's: nowhere.
        assert_eq!(definition(&analysis, at("count", 0), &env), None);
        assert_eq!(definition(&analysis, at("pi", 0), &env), None);
        assert_eq!(definition(&analysis, at("print", 0), &env), None);
    }
}
