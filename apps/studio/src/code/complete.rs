//! Completion and hover (#531): what fits where the cursor is, and what a name under the pointer is.
//!
//! After a `.` or `:` the list is the members of what is before it, worked out by the analysis
//! ([`super::analysis`]): an instance's class's members and, for one of the game, its children by name; a data type's
//! or a library's members; an enum's items; a table's fields, a ModuleScript's included. Elsewhere it is the keywords,
//! the locals in scope and the globals. Inside the string of `GetService("`, `Instance.new("` or `FindFirstChild("` it is
//! the services, the classes that can be made, or the children.

use std::ops::Range;

use metallic_engine::metallic_core::{DataModel, Id};

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
    /// A ModuleScript of the game, required when taken (#652).
    Module,
    /// A service, got with `GetService` when taken (#857).
    Service,
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
            ItemKind::Module => "R",
            ItemKind::Service => "S",
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
    /// For a ModuleScript (#652), the `local name = require(path)` line taking it adds near the top; for a service
    /// (#857), its `local Name = game:GetService("Name")`.
    pub require: Option<String>,
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
            require: None,
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
            require: None,
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
            require: None,
        });
    }
    // The game's ModuleScripts (#652), but for the script itself and a name already standing for something here.
    let named: Vec<String> = items
        .iter()
        .filter(|item| item.kind != ItemKind::Keyword)
        .map(|item| item.label.clone())
        .collect();
    items.extend(module_items(env, &named));
    items.extend(service_items(env, &named));
    items
}

/// The services as suggestions that get them when taken (#857), but for those whose name is in `named` already.
fn service_items(env: &Env<'_>, named: &[String]) -> Vec<Item> {
    env.dm
        .registry()
        .classes()
        .iter()
        .filter(|info| info.service_order().is_some())
        .map(|info| info.name())
        .filter(|name| is_identifier(name) && !named.iter().any(|taken| taken == name))
        .map(|name| {
            let line = format!("local {name} = game:GetService(\"{name}\")");
            Item {
                label: name.to_owned(),
                kind: ItemKind::Service,
                detail: "service".to_owned(),
                doc: format!("Adds `{line}` near the top."),
                require: Some(line),
            }
        })
        .collect()
}

/// Whether `name` can be written as a name in Luau: not empty, not a keyword, no character a name cannot have.
fn is_identifier(name: &str) -> bool {
    name.chars().next().is_some_and(|c| !c.is_ascii_digit())
        && name.chars().all(is_name_char)
        && !KEYWORDS.contains(&name)
}

/// The game's ModuleScripts as suggestions that require them when taken (#652): each but the script itself and those
/// whose name is not a Luau name or is in `named` already.
fn module_items(env: &Env<'_>, named: &[String]) -> Vec<Item> {
    let dm = env.dm;
    let Some(root) = dm.root() else {
        return Vec::new();
    };
    let Ok(all) = dm.descendants(root) else {
        return Vec::new();
    };
    all.filter(|&id| Some(id) != env.script && dm.class_name(id).ok() == Some("ModuleScript"))
        .filter_map(|id| {
            let name = dm.name(id).ok()?;
            if !is_identifier(name) || named.iter().any(|taken| taken == name) {
                return None;
            }
            let (path, shown) = require_path(dm, root, id)?;
            // From the script itself when the module is beside it or inside it (#693), as a Roblox script does.
            let path = relative_path(dm, env.script, id).unwrap_or(path);
            let line = format!("local {name} = require({path})");
            Some(Item {
                label: name.to_owned(),
                kind: ItemKind::Module,
                detail: shown,
                doc: format!("Adds `{line}` near the top."),
                require: Some(line),
            })
        })
        .collect()
}

/// How the script `script` reaches the module `id` from itself (#693): `script.cheese` for one inside it,
/// `script.Parent.cheese` for one beside it (in the same parent), with `:WaitForChild("…")` for a name that is not a
/// Luau name; `None` for a module anywhere else, which [`require_path`] reaches from `game`.
fn relative_path(dm: &DataModel, script: Option<Id>, id: Id) -> Option<String> {
    let script = script?;
    let parent = dm.parent(id).ok()??;
    let base = if parent == script {
        "script"
    } else if dm.parent(script).ok()? == Some(parent) {
        "script.Parent"
    } else {
        return None;
    };
    let name = dm.name(id).ok()?;
    Some(if is_identifier(name) {
        format!("{base}.{name}")
    } else {
        let quoted = name.replace('\\', "\\\\").replace('"', "\\\"");
        format!("{base}:WaitForChild(\"{quoted}\")")
    })
}

/// How code reaches the instance `id` from `game` (`root`): `game:GetService("ReplicatedStorage").Folder.cheese`, with
/// `:WaitForChild("…")` for a name that is not a Luau name; and the same path shown short,
/// `ReplicatedStorage.Folder.cheese`.
fn require_path(dm: &DataModel, root: Id, id: Id) -> Option<(String, String)> {
    let mut chain = vec![id];
    let mut at = id;
    loop {
        let parent = dm.parent(at).ok()??;
        if parent == root {
            break;
        }
        chain.push(parent);
        at = parent;
    }
    chain.reverse();
    let (service, rest) = chain.split_first()?;
    let mut path = format!("game:GetService(\"{}\")", dm.class_name(*service).ok()?);
    let mut shown = dm.name(*service).ok()?.to_owned();
    for &step in rest {
        let name = dm.name(step).ok()?;
        if is_identifier(name) {
            path.push('.');
            path.push_str(name);
        } else {
            let quoted = name.replace('\\', "\\\\").replace('"', "\\\"");
            path.push_str(&format!(":WaitForChild(\"{quoted}\")"));
        }
        shown.push('.');
        shown.push_str(name);
    }
    Some((path, shown))
}

/// Where a `require` line goes in `text` (#652), as a byte offset and the text to put there for `line`: after the
/// comments, `require`s and `GetService`s the file starts with, else at the top with a blank line after it.
pub fn require_insertion(text: &str, line: &str) -> (usize, String) {
    let mut end = None;
    let mut offset = 0;
    for row in text.split_inclusive('\n') {
        let trimmed = row.trim();
        if trimmed.is_empty() {
            offset += row.len();
            continue;
        }
        let header = trimmed.starts_with("--")
            || (trimmed.starts_with("local ") && (trimmed.contains("require(") || trimmed.contains(":GetService(")));
        if !header {
            break;
        }
        offset += row.len();
        end = Some((offset, row.ends_with('\n')));
    }
    match end {
        Some((at, true)) => (at, format!("{line}\n")),
        Some((at, false)) => (at, format!("\n{line}")),
        None if text.is_empty() => (0, format!("{line}\n")),
        None => (0, format!("{line}\n\n")),
    }
}

/// What taking `item` in place of the bytes `replace` of `text` makes of it: the text, and the byte the cursor is then
/// at. A function gets its brackets (#854), the cursor between them when it takes arguments and after them when not,
/// unless a bracket follows already; a ModuleScript its `require` near the top (#652), a service its `GetService`
/// (#857), unless the file has it already. Taken alone on its line, a module or a service is that line alone (#858):
/// the line takes its place when that is where it goes, else its name is not put in.
pub fn taken(text: &str, replace: Range<usize>, item: &Item) -> (String, usize) {
    let end = replace.end.min(text.len());
    let start = replace.start.min(end);
    let line_start = text[..start].rfind('\n').map_or(0, |newline| newline + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |newline| end + newline);
    if let Some(line) = &item.require
        && !text.lines().any(|row| row.trim() == line)
        && text[line_start..start].trim().is_empty()
        && text[end..line_end].trim().is_empty()
    {
        let emptied = format!("{}{}", &text[..start], &text[end..]);
        let (at, insert) = require_insertion(&emptied, line);
        if at == line_start {
            let out = format!("{}{line}{}", &text[..line_start], &text[line_end..]);
            return (out, line_start + line.len());
        }
        let mut out = emptied;
        out.insert_str(at, &insert);
        let cursor = if at <= start { start + insert.len() } else { start };
        return (out, cursor);
    }
    let mut with = item.label.clone();
    let mut cursor = start + with.len();
    if item.kind == ItemKind::Method && !text[end..].starts_with('(') {
        with.push_str("()");
        cursor += if takes_arguments(&item.detail) { 1 } else { 2 };
    }
    let mut out = format!("{}{with}{}", &text[..start], &text[end..]);
    if let Some(line) = &item.require
        && !out.lines().any(|row| row.trim() == line)
    {
        let (at, insert) = require_insertion(&out, line);
        out.insert_str(at, &insert);
        if at <= start {
            cursor += insert.len();
        }
    }
    (out, cursor)
}

/// Whether a function whose type or signature is `detail` takes arguments: anything in its first brackets but
/// `self`; yes when that is not known.
fn takes_arguments(detail: &str) -> bool {
    let Some(open) = detail.find('(') else {
        return true;
    };
    let inner = &detail[open + 1..];
    let close = inner.find(')').unwrap_or(inner.len());
    inner[..close]
        .split(',')
        .map(str::trim)
        .any(|parameter| !parameter.is_empty() && parameter != "self" && !parameter.starts_with("self:"))
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
                            require: None,
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
                require: None,
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
        require: None,
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
                require: None,
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
                        require: None,
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

    /// #854: a function taken gets its brackets, the cursor between them when it takes arguments, after them when
    /// not; not when a bracket follows, nor for what is not a function.
    #[test]
    fn a_function_is_taken_with_its_brackets() {
        let item = |label: &str, kind: ItemKind, detail: &str| Item {
            label: label.to_owned(),
            kind,
            detail: detail.to_owned(),
            doc: String::new(),
            require: None,
        };
        let find = item(
            "FindFirstChild",
            ItemKind::Method,
            "function FindFirstChild(self, name: string): Instance?",
        );
        let shown = |(text, at): (String, usize)| format!("{}|{}", &text[..at], &text[at..]);
        assert_eq!(shown(taken("part:Fi", 5..7, &find)), "part:FindFirstChild(|)");
        let destroy = item("Destroy", ItemKind::Method, "function Destroy(self): ()");
        assert_eq!(shown(taken("part:De", 5..7, &destroy)), "part:Destroy()|");
        assert_eq!(shown(taken("part:Fi(x)", 5..7, &find)), "part:FindFirstChild|(x)");
        let print = item("print", ItemKind::Method, "(...any) -> ()");
        assert_eq!(shown(taken("pr", 0..2, &print)), "print(|)");
        let anchored = item("Anchored", ItemKind::Property, "boolean");
        assert_eq!(shown(taken("p.An", 2..4, &anchored)), "p.Anchored|");
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
        assert!(labels(&dm, "Enum.Par|").unwrap().contains(&"PartShape".to_owned()));
        assert!(
            labels(&dm, "local m = Enum.PartShape.Ba|")
                .unwrap()
                .contains(&"Ball".to_owned())
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
        assert_eq!(services, ["ReplicatedFirst", "ReplicatedStorage"]);
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

    /// A ModuleScript named `name` in a folder `folder` made in ReplicatedStorage.
    fn add_module(dm: &mut DataModel, folder: &str, name: &str) -> Id {
        let class = |dm: &DataModel, name: &str| dm.registry().class_id(name).unwrap();
        let storage = dm.service(class(dm, "ReplicatedStorage")).unwrap();
        let parent = dm.create(class(dm, "Folder")).unwrap();
        dm.set_name(parent, folder.into()).unwrap();
        dm.set_parent(parent, Some(storage)).unwrap();
        let module = dm.create(class(dm, "ModuleScript")).unwrap();
        dm.set_name(module, name.into()).unwrap();
        dm.set_parent(module, Some(parent)).unwrap();
        module
    }

    /// The suggestions where `|` is in `code`, in the script `script`.
    fn items(dm: &DataModel, code: &str, script: Option<Id>) -> Vec<Item> {
        let at = code.find('|').unwrap();
        let source = code.replace('|', "");
        let analysis = Analysis::new(&source);
        let env = Env {
            dm,
            script,
            module: &no_modules,
        };
        complete(&analysis, at, &env).map(|c| c.items).unwrap_or_default()
    }

    /// #652: a ModuleScript is suggested by its name, with the `require` taking it adds; not where a local has the
    /// name already, nor in the module itself; a folder whose name is not a Luau name is reached by `WaitForChild`.
    #[test]
    fn modules_are_suggested_with_their_require() {
        let mut dm = game();
        let cheese = add_module(&mut dm, "Food", "cheese");
        let found = items(&dm, "chee|", None);
        let item = found
            .iter()
            .find(|item| item.label == "cheese")
            .expect("cheese is suggested");
        assert_eq!(item.kind, ItemKind::Module);
        assert_eq!(item.detail, "ReplicatedStorage.Food.cheese");
        assert_eq!(
            item.require.as_deref(),
            Some(r#"local cheese = require(game:GetService("ReplicatedStorage").Food.cheese)"#)
        );
        let shadowed = items(&dm, "local cheese = 1\nchee|", None);
        assert!(
            shadowed.iter().all(|item| item.kind != ItemKind::Module),
            "{shadowed:?}"
        );
        assert!(
            items(&dm, "chee|", Some(cheese))
                .iter()
                .all(|item| item.label != "cheese")
        );
        add_module(&mut dm, "my folder", "Util");
        let found = items(&dm, "Uti|", None);
        let util = found.iter().find(|item| item.label == "Util").unwrap();
        assert_eq!(
            util.require.as_deref(),
            Some(r#"local Util = require(game:GetService("ReplicatedStorage"):WaitForChild("my folder").Util)"#)
        );
    }

    /// #857: a service is suggested by its name, with the `GetService` taking it adds; not where a local has the name.
    #[test]
    fn services_are_suggested_with_their_get_service() {
        let dm = game();
        let found = items(&dm, "ReplicatedSt|", None);
        let storage = found
            .iter()
            .find(|item| item.label == "ReplicatedStorage")
            .expect("ReplicatedStorage is suggested");
        assert_eq!(storage.kind, ItemKind::Service);
        assert_eq!(
            storage.require.as_deref(),
            Some(r#"local ReplicatedStorage = game:GetService("ReplicatedStorage")"#)
        );
        let (text, at) = taken("print(ReplicatedSt)", 6..18, storage);
        assert_eq!(
            text,
            "local ReplicatedStorage = game:GetService(\"ReplicatedStorage\")\n\nprint(ReplicatedStorage)"
        );
        assert_eq!(at, text.len() - 1);
        let shadowed = items(
            &dm,
            "local ReplicatedStorage = game:GetService(\"ReplicatedStorage\")\nReplicatedSt|",
            None,
        );
        let local = shadowed.iter().find(|item| item.label == "ReplicatedStorage").unwrap();
        assert_eq!(local.kind, ItemKind::Local);
        assert!(shadowed.iter().all(|item| item.kind != ItemKind::Service));
    }

    /// #858: a module or a service taken alone on its line is its `require` line alone, in that line's place when
    /// that is where it goes, else with the others at the top and the line left empty.
    #[test]
    fn a_module_alone_on_its_line_is_its_require_alone() {
        let item = Item {
            label: "cheese".to_owned(),
            kind: ItemKind::Module,
            detail: String::new(),
            doc: String::new(),
            require: Some("local cheese = require(x)".to_owned()),
        };
        let shown = |(text, at): (String, usize)| format!("{}|{}", &text[..at], &text[at..]);
        assert_eq!(shown(taken("chee", 0..4, &item)), "local cheese = require(x)|");
        assert_eq!(
            shown(taken("local a = require(y)\nchee\nprint()", 21..25, &item)),
            "local a = require(y)\nlocal cheese = require(x)|\nprint()"
        );
        assert_eq!(
            shown(taken("x()\n  chee", 6..10, &item)),
            "local cheese = require(x)\n\nx()\n  |"
        );
        // Not alone: the name too.
        assert_eq!(
            shown(taken("print(chee)", 6..10, &item)),
            "local cheese = require(x)\n\nprint(cheese|)"
        );
    }

    /// #693: a module beside the script is required from `script.Parent`, one inside it from `script`; one elsewhere
    /// from `game` as before.
    #[test]
    fn a_module_near_the_script_is_required_from_it() {
        let mut dm = game();
        let class = |dm: &DataModel, name: &str| dm.registry().class_id(name).unwrap();
        let tools = add_module(&mut dm, "Food", "Tools");
        let folder = dm.parent(tools).unwrap().unwrap();
        let script = dm.create(class(&dm, "LocalScript")).unwrap();
        dm.set_name(script, "Hud".into()).unwrap();
        dm.set_parent(script, Some(folder)).unwrap();
        let inner = dm.create(class(&dm, "ModuleScript")).unwrap();
        dm.set_name(inner, "inner".into()).unwrap();
        dm.set_parent(inner, Some(script)).unwrap();
        add_module(&mut dm, "Other", "far");
        let require = |label: &str| {
            items(&dm, &format!("{}|", &label[..2]), Some(script))
                .into_iter()
                .find(|item| item.label == label)
                .and_then(|item| item.require)
                .unwrap_or_else(|| panic!("{label} is suggested"))
        };
        assert_eq!(require("Tools"), "local Tools = require(script.Parent.Tools)");
        assert_eq!(require("inner"), "local inner = require(script.inner)");
        assert_eq!(
            require("far"),
            r#"local far = require(game:GetService("ReplicatedStorage").Other.far)"#
        );
        // Without a script (a file no script runs), from `game`.
        let detached = items(&dm, "To|", None);
        let tools_item = detached.iter().find(|item| item.label == "Tools").unwrap();
        assert!(tools_item.require.as_deref().unwrap().contains("GetService"));
    }

    /// #652: the `require` goes after the comments, requires and services the file starts with, else on top.
    #[test]
    fn a_require_goes_after_the_file_s_header() {
        let line = "local cheese = require(x)";
        assert_eq!(require_insertion("", line), (0, format!("{line}\n")));
        assert_eq!(require_insertion("print(1)\n", line), (0, format!("{line}\n\n")));
        let header = "--!strict\nlocal Players = game:GetService(\"Players\")\n\nprint(1)\n";
        let (at, with) = require_insertion(header, line);
        assert_eq!(
            &header[..at],
            "--!strict\nlocal Players = game:GetService(\"Players\")\n"
        );
        assert_eq!(with, format!("{line}\n"));
        assert_eq!(require_insertion("-- hi", line), (5, format!("\n{line}")));
    }
}
