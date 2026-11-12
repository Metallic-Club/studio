//! Completion and hover (#531): what fits where the cursor is, and what a name under the pointer is.
//!
//! After a `.` or `:` the list is the members of what is before it, worked out by the analysis
//! ([`super::analysis`]): an instance's class's members and, for one of the game, its children by name; a data type's
//! or a library's members; an enum's items; a table's fields, a ModuleScript's included. Elsewhere it is the keywords,
//! the locals in scope and the globals. Inside the string of `GetService("`, `Instance.new("` or `FindFirstChild("` it is
//! the services, the classes that can be made, or the children.

use std::ops::Range;

use super::analysis::{Analysis, DeclKind, Env, Ref, Ty};
use super::api::Member;
use super::lexer::{KEYWORDS, TokenKind};

/// What a suggestion is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemKind {
    /// A local, a parameter or a loop variable.
    Local,
    /// A child of an instance of the game.
    Child,
    /// A property, a field, an enum's item.
    Property,
    /// A method or a function.
    Method,
    /// An event.
    Event,
    /// A global.
    Global,
    /// A reserved word.
    Keyword,
    /// A class's or a service's name, in a string.
    Class,
}

impl ItemKind {
    /// The letter the list shows for it.
    pub fn letter(self) -> &'static str {
        match self {
            ItemKind::Local => "L",
            ItemKind::Child => "C",
            ItemKind::Property => "P",
            ItemKind::Method => "M",
            ItemKind::Event => "E",
            ItemKind::Global => "G",
            ItemKind::Keyword => "K",
            ItemKind::Class => "T",
        }
    }
}

/// One suggestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// What it is called, and what is put in.
    pub label: String,
    /// What it is.
    pub kind: ItemKind,
    /// Its type or signature.
    pub detail: String,
    /// What it does.
    pub doc: String,
}

/// The suggestions at a place, and what taking one replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// The suggestions, best first.
    pub items: Vec<Item>,
    /// The bytes the one taken replaces: the word typed so far.
    pub replace: Range<usize>,
}

/// What hovering a name says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    /// What it is, as code: `function FindFirstChild(self, name: string, ...): Instance?`, `local part: Part`.
    pub title: String,
    /// What it does.
    pub doc: String,
}

/// Whether `c` can be in a name.
fn is_name_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// The suggestions at byte `at` of the code `analysis` read; `None` where nothing is suggested (a comment, a string that
/// takes no name, or nothing typed outside a member access).
pub fn complete(analysis: &Analysis, at: usize, env: &Env<'_>) -> Option<Completion> {
    let source = &analysis.source;
    let at = at.min(source.len());
    // In a string or a comment.
    if let Some(token) = super::lexer::lex(source)
        .into_iter()
        .find(|token| token.start < at && at <= token.end)
        && matches!(token.kind, TokenKind::String | TokenKind::Comment)
    {
        let closed = !token.unterminated && at == token.end;
        if token.kind == TokenKind::Comment || closed {
            return None;
        }
        return in_string(analysis, token.start, at, env);
    }
    let start = source[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_name_char(*c))
        .last()
        .map_or(at, |(index, _)| index);
    let prefix = &source[start..at];
    if prefix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let before = source[..start].chars().next_back();
    let mut items = if matches!(before, Some('.' | ':')) {
        // `x.` or `x:`: the members of x.
        let dot = analysis
            .tokens
            .iter()
            .position(|token| token.end == start && token.start + 1 == start)?;
        let first = analysis.chain_start(dot)?;
        let ty = analysis.type_before(first, dot, env);
        members(analysis, &ty, before == Some(':'), env)
    } else {
        if prefix.is_empty() {
            return None;
        }
        // `local x` and `function f`: a new name, nothing to suggest.
        let previous = analysis
            .tokens
            .iter()
            .rev()
            .find(|token| token.end <= start)
            .map(|token| token.text(source));
        if matches!(previous, Some("local" | "function" | "for")) {
            return None;
        }
        everything(analysis, start, env)
    };
    filter(&mut items, prefix);
    Some(Completion {
        items,
        replace: start..at,
    })
}

/// Keeps the items `prefix` fits (its letters at the start, in any case, else anywhere), the closest first.
fn filter(items: &mut Vec<Item>, prefix: &str) {
    let lower = prefix.to_lowercase();
    items.retain(|item| item.label.to_lowercase().contains(&lower));
    items.sort_by_key(|item| {
        let label = item.label.to_lowercase();
        (
            !item.label.starts_with(prefix),
            !label.starts_with(&lower),
            item.kind,
            label,
        )
    });
    items.dedup_by(|a, b| a.label == b.label);
}

/// The keywords, the locals in scope at byte `at`, the file's globals and the API's.
fn everything(analysis: &Analysis, at: usize, env: &Env<'_>) -> Vec<Item> {
    let mut items: Vec<Item> = KEYWORDS
        .iter()
        .map(|keyword| Item {
            label: (*keyword).to_owned(),
            kind: ItemKind::Keyword,
            detail: String::new(),
            doc: String::new(),
        })
        .collect();
    for decl in analysis.visible(at) {
        let declaration = &analysis.decls[decl];
        if declaration.range.start <= at && at <= declaration.range.end {
            continue;
        }
        let ty = analysis.decl_type(decl, env);
        items.push(Item {
            label: declaration.name.clone(),
            kind: if declaration.function {
                ItemKind::Method
            } else if declaration.kind == DeclKind::Global {
                ItemKind::Global
            } else {
                ItemKind::Local
            },
            detail: type_name(&ty, env),
            doc: String::new(),
        });
    }
    for global in env.api().globals.values() {
        items.push(Item {
            label: global.name.clone(),
            kind: if global.function {
                ItemKind::Method
            } else {
                ItemKind::Global
            },
            detail: global.ty.clone(),
            doc: global.doc.clone(),
        });
    }
    items
}

/// The members of a value of type `ty`, after `.` or with `methods` after `:`.
pub fn members(analysis: &Analysis, ty: &Ty, methods: bool, env: &Env<'_>) -> Vec<Item> {
    let api = env.api();
    let of_type = |name: &str| -> Vec<Item> {
        api.members(name)
            .into_iter()
            .filter(|member| !methods || member.method)
            .map(member_item)
            .collect()
    };
    match ty {
        Ty::Live(id) => {
            let class = env.class(*id).unwrap_or_default();
            let mut items = of_type(&class);
            if !methods && let Ok(children) = env.dm.children(*id) {
                for child in children {
                    let name = env.dm.name(child).unwrap_or_default();
                    if name.chars().all(is_name_char) && !name.is_empty() {
                        items.push(Item {
                            label: name.to_owned(),
                            kind: ItemKind::Child,
                            detail: env.class(child).unwrap_or_default(),
                            doc: format!("A child of {}.", env.dm.name(*id).unwrap_or_default()),
                        });
                    }
                }
            }
            items
        }
        Ty::Named(name) => of_type(name),
        Ty::Library(global) => api
            .globals
            .get(global)
            .map(|global| {
                global
                    .fields
                    .iter()
                    .filter(|field| !methods || field.is_function())
                    .map(|field| {
                        let mut item = member_item(field);
                        if global.name == "Enum" {
                            item.kind = ItemKind::Property;
                            item.detail = format!("Enum.{}", field.name);
                        }
                        item
                    })
                    .collect()
            })
            .unwrap_or_default(),
        Ty::Table(owner) => table_items(analysis, *owner, env),
        Ty::ModuleTable(module, owner) => (env.module)(*module)
            .map(|code| {
                let inner = Env {
                    dm: env.dm,
                    script: Some(*module),
                    module: env.module,
                };
                table_items(&code, *owner, &inner)
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The fields of the table declared by `owner` in `analysis`.
fn table_items(analysis: &Analysis, owner: usize, env: &Env<'_>) -> Vec<Item> {
    analysis
        .fields(owner)
        .into_iter()
        .map(|field| {
            let decl = &analysis.decls[field];
            Item {
                label: decl.name.clone(),
                kind: if decl.function {
                    ItemKind::Method
                } else {
                    ItemKind::Property
                },
                detail: type_name(&analysis.decl_type(field, env), env),
                doc: String::new(),
            }
        })
        .collect()
}

/// A member of the API as a suggestion.
fn member_item(member: &Member) -> Item {
    Item {
        label: member.name.clone(),
        kind: if member.is_function() {
            ItemKind::Method
        } else if member.ty.starts_with("ScriptSignal") {
            ItemKind::Event
        } else {
            ItemKind::Property
        },
        detail: member.ty.clone(),
        doc: member.doc.clone(),
    }
}

/// The suggestions in the string opening at byte `quote`, with the cursor at `at`: services for `GetService("`, the
/// classes that can be made for `Instance.new("` and `IsA("`, children for `FindFirstChild("` and `WaitForChild("`.
fn in_string(analysis: &Analysis, quote: usize, at: usize, env: &Env<'_>) -> Option<Completion> {
    let source = &analysis.source;
    let open = source[..quote].trim_end();
    let call = open.strip_suffix('(')?.trim_end();
    let name = call.rsplit(|c: char| !is_name_char(c)).next().unwrap_or_default();
    let typed = &source[quote + 1..at];
    let registry = env.dm.registry();
    let classes = |filter: &dyn Fn(&metallic_engine::metallic_core::ClassInfo) -> bool| -> Vec<Item> {
        let api = env.api();
        registry
            .classes()
            .iter()
            .filter(|info| filter(info))
            .map(|info| Item {
                label: info.name().to_owned(),
                kind: ItemKind::Class,
                detail: "class".to_owned(),
                doc: api
                    .types
                    .get(info.name())
                    .map(|def| def.doc.clone())
                    .unwrap_or_default(),
            })
            .collect()
    };
    let mut items = match name {
        "GetService" => classes(&|info| info.service_order().is_some()),
        "new" if call.ends_with("Instance.new") => classes(&|info| info.creatable() && info.service_order().is_none()),
        "IsA" | "FindFirstChildOfClass" | "FindFirstChildWhichIsA" | "FindFirstAncestorOfClass" => classes(&|_| true),
        "FindFirstChild" | "WaitForChild" => {
            // The children of what it is called on.
            let colon = call.len() - name.len();
            let dot = analysis
                .tokens
                .iter()
                .position(|token| token.end == colon && token.start + 1 == colon)?;
            let first = analysis.chain_start(dot)?;
            match analysis.type_before(first, dot, env) {
                Ty::Live(id) => env
                    .dm
                    .children(id)
                    .ok()?
                    .map(|child| Item {
                        label: env.dm.name(child).unwrap_or_default().to_owned(),
                        kind: ItemKind::Child,
                        detail: env.class(child).unwrap_or_default(),
                        doc: String::new(),
                    })
                    .collect(),
                _ => Vec::new(),
            }
        }
        _ => return None,
    };
    filter(&mut items, typed);
    Some(Completion {
        items,
        replace: quote + 1..at,
    })
}

/// What a value of type `ty` is, in a few words.
pub fn type_name(ty: &Ty, env: &Env<'_>) -> String {
    match ty {
        Ty::Unknown => "any".to_owned(),
        Ty::Named(name) => name.trim_end_matches("_INTERNAL").to_owned(),
        Ty::Live(id) => format!(
            "{} '{}'",
            env.class(*id).unwrap_or_default(),
            env.dm.name(*id).unwrap_or_default()
        ),
        Ty::Library(name) => name.clone(),
        Ty::Table(_) | Ty::ModuleTable(..) => "table".to_owned(),
        Ty::Number => "number".to_owned(),
        Ty::String => "string".to_owned(),
        Ty::Boolean => "boolean".to_owned(),
        Ty::Function => "function".to_owned(),
    }
}

/// What the name at byte `at` is: an API member, global or type (with its description), or the file's local (with its
/// type). `None` for anything else.
pub fn hover(analysis: &Analysis, at: usize, env: &Env<'_>) -> Option<Hover> {
    let index = analysis.token_at(at)?;
    let token = *analysis.tokens.get(index)?;
    if token.kind != TokenKind::Name {
        return None;
    }
    let name = token.text(&analysis.source);
    let api = env.api();
    // A member: of what is before its `.` or `:`.
    if index > 0 && (analysis.text(index - 1) == "." || analysis.text(index - 1) == ":") {
        let first = analysis.chain_start(index - 1)?;
        let ty = analysis.type_before(first, index - 1, env);
        let class = match &ty {
            Ty::Live(id) => env.class(*id),
            Ty::Named(name) => Some(name.clone()),
            _ => None,
        };
        if let Some(class) = class
            && let Some(member) = api.member(&class, name)
        {
            return Some(member_hover(member, &class));
        }
        if let Ty::Library(global) = &ty
            && let Some(field) = api
                .globals
                .get(global)
                .and_then(|global| global.fields.iter().find(|field| field.name == name))
        {
            return Some(Hover {
                title: format!("{global}.{name}: {}", field.ty),
                doc: field.doc.clone(),
            });
        }
        let member = analysis.member_type(&ty, name, env, 0);
        if member != Ty::Unknown {
            return Some(Hover {
                title: format!("{name}: {}", type_name(&member, env)),
                doc: String::new(),
            });
        }
        return None;
    }
    if analysis.type_tokens.contains(&index) {
        // A type annotation: the API's type.
        let def = api.types.get(name)?;
        return Some(Hover {
            title: format!("type {name}"),
            doc: def.doc.clone(),
        });
    }
    match analysis
        .refs
        .get(&index)
        .copied()
        .or_else(|| analysis.decl_of_token(index).map(Ref::Decl))
    {
        Some(Ref::Decl(decl)) => {
            let declaration = &analysis.decls[decl];
            let what = match declaration.kind {
                DeclKind::Global => "global",
                DeclKind::Param => "parameter",
                DeclKind::Field { .. } => "field",
                _ if declaration.function => "local function",
                _ => "local",
            };
            Some(Hover {
                title: format!("{what} {name}: {}", type_name(&analysis.decl_type(decl, env), env)),
                doc: String::new(),
            })
        }
        Some(Ref::Global) | None => {
            let global = api.globals.get(name)?;
            let title = if global.function {
                global.ty.clone()
            } else {
                format!("{name}: {}", global.ty)
            };
            Some(Hover {
                title,
                doc: global.doc.clone(),
            })
        }
    }
}

/// What hovering the API's member says.
fn member_hover(member: &Member, class: &str) -> Hover {
    let title = if member.method {
        member.ty.replacen("function ", &format!("function {class}:"), 1)
    } else {
        format!("{class}.{}: {}", member.name, member.ty)
    };
    Hover {
        title,
        doc: member.doc.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use metallic_engine::metallic_core::{DataModel, Id};

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

    fn no_modules(_: Id) -> Option<Rc<Analysis>> {
        None
    }

    /// The labels suggested where `|` is in `code`.
    fn labels(dm: &DataModel, code: &str) -> Option<Vec<String>> {
        let at = code.find('|').unwrap();
        let source = code.replace('|', "");
        let analysis = Analysis::new(&source);
        let env = Env {
            dm,
            script: None,
            module: &no_modules,
        };
        complete(&analysis, at, &env).map(|completion| completion.items.into_iter().map(|item| item.label).collect())
    }

    #[test]
    fn members_after_a_dot_or_a_colon() {
        let dm = game();
        let after = labels(&dm, "workspace.Base|").unwrap();
        assert_eq!(after.first().map(String::as_str), Some("Baseplate"), "{after:?}");
        let part = labels(&dm, "local p = Instance.new(\"Part\")\np.Anch|").unwrap();
        assert_eq!(part, ["Anchored"]);
        // A colon offers methods only.
        let methods = labels(&dm, "workspace.Baseplate:|").unwrap();
        assert!(
            methods.contains(&"Destroy".to_owned()) && !methods.contains(&"Anchored".to_owned()),
            "{methods:?}"
        );
        // A data type, a library, an enum.
        assert!(
            labels(&dm, "local v = Vector3.new()\nv.Mag|")
                .unwrap()
                .contains(&"Magnitude".to_owned())
        );
        assert!(labels(&dm, "math.cl|").unwrap().contains(&"clamp".to_owned()));
        assert!(labels(&dm, "Enum.Mat|").unwrap().contains(&"Material".to_owned()));
        assert!(
            labels(&dm, "local m = Enum.Material.Pl|")
                .unwrap()
                .contains(&"Plastic".to_owned())
        );
        // A local table's fields.
        let mut fields = labels(&dm, "local M = { speed = 1 }\nfunction M.open() end\nM.|").unwrap();
        fields.sort();
        assert_eq!(fields, ["open", "speed"]);
    }

    #[test]
    fn names_keywords_locals_and_globals_elsewhere() {
        let dm = game();
        let got = labels(&dm, "local counter = 1\nlocal function count() end\ncou|").unwrap();
        let mut first = got[..2].to_vec();
        first.sort();
        assert_eq!(first, ["count", "counter"]);
        assert!(labels(&dm, "pri|").unwrap().contains(&"print".to_owned()));
        assert!(labels(&dm, "ret|").unwrap().contains(&"return".to_owned()));
        // A new name, a number, nothing typed, a comment: nothing.
        assert_eq!(labels(&dm, "local na|"), None);
        assert_eq!(labels(&dm, "x = 1|"), None);
        assert_eq!(labels(&dm, "print(|"), None);
        assert_eq!(labels(&dm, "-- workspace.Ba|"), None);
    }

    #[test]
    fn strings_that_name_services_classes_and_children() {
        let dm = game();
        let services = labels(&dm, "game:GetService(\"Rep|").unwrap();
        assert_eq!(services, ["ReplicatedStorage"]);
        let classes = labels(&dm, "Instance.new(\"Poi|").unwrap();
        assert_eq!(classes, ["PointLight"]);
        let children = labels(&dm, "workspace:FindFirstChild(\"Spa|").unwrap();
        assert_eq!(children, ["SpawnLocation"]);
        assert_eq!(labels(&dm, "print(\"Rep|"), None);
    }

    #[test]
    fn hovering_says_what_a_name_is() {
        let dm = game();
        let env = Env {
            dm: &dm,
            script: None,
            module: &no_modules,
        };
        let source = "local part = workspace.Baseplate\npart:FindFirstChild(\"x\")\nprint(part.Anchored, math.pi)\n";
        let analysis = Analysis::new(source);
        let at = |needle: &str| source.find(needle).unwrap();
        let hover = |needle: &str| hover(&analysis, at(needle), &env).unwrap();
        assert!(
            hover("FindFirstChild")
                .title
                .starts_with("function Part:FindFirstChild(self")
        );
        assert!(hover("FindFirstChild").doc.contains("first child"));
        assert_eq!(hover("Anchored").title, "Part.Anchored: boolean");
        assert_eq!(hover("part").title, "local part: Part 'Baseplate'");
        assert!(hover("print").doc.contains("output"));
        assert!(hover("pi").title.starts_with("math.pi"));
        assert_eq!(hover("workspace").title, "workspace: Workspace");
    }
}
