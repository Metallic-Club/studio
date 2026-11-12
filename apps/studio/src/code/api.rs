//! What the script editor knows of the API (#530, #531): Metallic's classes, data types, enums and globals, read from
//! `api/metallic.d.luau` (generated from the class registry by `cargo xtask api-dump`, which CI keeps up to date), and
//! Luau's own libraries ([`LUAU`]), in the same syntax. Completion, hover and the member warnings ask it.
//!
//! Only the subset of luau-lsp's definitions syntax that file uses is read: `--- doc` lines, `declare extern type X
//! [extends Y] with ... end`, `export type X<...> = { ... }`, `declare x: T`, `declare x: { ... }` and `declare function
//! f(...): R`. A member declared more than once (`GetService`, `Instance.new`) has each overload kept.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

/// Metallic's definitions, as generated.
const DEFINITIONS: &str = include_str!("../../../../api/metallic.d.luau");

/// Luau's own globals and libraries, which luau-lsp knows without a definitions file.
pub const LUAU: &str = r##"
--- Writes its arguments to the output, separated by spaces.
declare function print(...: any): ()
--- Stops the thread with `message` as its error. `level` 1 (the default) names where error was called.
declare function error(message: any, level: number?): never
--- Returns its arguments when `value` is true; otherwise raises `message` ("assertion failed!").
declare function assert<T>(value: T, message: string?): T
--- Calls `f` with the arguments; returns true and its results, or false and the error.
declare function pcall(f: (...any) -> ...any, ...: any): (boolean, ...any)
--- As pcall, calling `handler` with the error to make what is returned.
declare function xpcall(f: (...any) -> ...any, handler: (any) -> any, ...: any): (boolean, ...any)
--- The arguments from the `index`th on, or with "#" how many there are.
declare function select(index: number | string, ...: any): ...any
--- The name of the value's Luau type: "nil", "number", "string", "boolean", "table", "function", "thread", "userdata", "vector" or "buffer".
declare function type(value: any): string
--- As type, but Metallic's types by name: "Instance", "Vector3", "CFrame", "EnumItem"...
declare function typeof(value: any): string
--- The value as text.
declare function tostring(value: any): string
--- The text as a number in `base` (10 by default), or nil.
declare function tonumber(value: any, base: number?): number?
--- Every key and value of a table, for a generic for: `for key, value in pairs(t) do`.
declare function pairs<K, V>(t: {[K]: V}): ((t: {[K]: V}, key: K?) -> (K, V), {[K]: V}, nil)
--- An array's indices and values in order, from 1 to the first nil.
declare function ipairs<V>(t: {V}): ((t: {V}, index: number) -> (number, V), {V}, number)
--- The key after `key` in the table and its value, or nil at the end.
declare function next<K, V>(t: {[K]: V}, key: K?): (K?, V)
--- Sets the table's metatable, and returns the table.
declare function setmetatable<T>(t: T, metatable: {}?): T
--- The value's metatable, or its `__metatable` field.
declare function getmetatable(value: any): {}?
--- The table's field without its metatable.
declare function rawget(t: {}, key: any): any
--- Sets the table's field without its metatable.
declare function rawset(t: {}, key: any, value: any): {}
--- Whether the two values are equal, without their metatables.
declare function rawequal(a: any, b: any): boolean
--- The table's or string's length, without its metatable.
declare function rawlen(value: {} | string): number
--- The array's values from `i` to `j`.
declare function unpack<V>(t: {V}, i: number?, j: number?): ...V
--- A table every script of the VM shares.
declare _G: { [any]: any }
--- Luau's version, as text.
declare _VERSION: string

--- Mathematics.
declare math: {
    --- The absolute value.
    abs: (x: number) -> number,
    --- The smallest integer at least x.
    ceil: (x: number) -> number,
    --- The largest integer at most x.
    floor: (x: number) -> number,
    --- x rounded to the nearest integer, halves away from zero.
    round: (x: number) -> number,
    --- The greatest of the numbers.
    max: (x: number, ...number) -> number,
    --- The least of the numbers.
    min: (x: number, ...number) -> number,
    --- x kept between min and max.
    clamp: (x: number, min: number, max: number) -> number,
    --- -1, 0 or 1, by the number's sign.
    sign: (x: number) -> number,
    --- The square root.
    sqrt: (x: number) -> number,
    --- x to the power y.
    pow: (x: number, y: number) -> number,
    --- e to the power x.
    exp: (x: number) -> number,
    --- The logarithm of x in `base` (e by default).
    log: (x: number, base: number?) -> number,
    --- The base-10 logarithm.
    log10: (x: number) -> number,
    --- The remainder of x / y, with x's sign.
    fmod: (x: number, y: number) -> number,
    --- The integer and fractional parts.
    modf: (x: number) -> (number, number),
    --- The sine of an angle in radians.
    sin: (x: number) -> number,
    --- The cosine of an angle in radians.
    cos: (x: number) -> number,
    --- The tangent of an angle in radians.
    tan: (x: number) -> number,
    --- The arc sine, in radians.
    asin: (x: number) -> number,
    --- The arc cosine, in radians.
    acos: (x: number) -> number,
    --- The arc tangent, in radians.
    atan: (x: number) -> number,
    --- The angle of the point (x, y) from the X axis, in radians.
    atan2: (y: number, x: number) -> number,
    --- Degrees to radians.
    rad: (x: number) -> number,
    --- Radians to degrees.
    deg: (x: number) -> number,
    --- A random number from 0 up to 1, or an integer from 1 to m, or from m to n.
    random: (m: number?, n: number?) -> number,
    --- Starts math.random's sequence from `seed`.
    randomseed: (seed: number) -> (),
    --- Perlin noise at the point, from about -1 to 1.
    noise: (x: number, y: number?, z: number?) -> number,
    --- x moved towards y by t (0 is x, 1 is y).
    lerp: (x: number, y: number, t: number) -> number,
    --- Pi, about 3.14159.
    pi: number,
    --- Infinity.
    huge: number,
}

--- Text.
declare string: {
    --- The text with `{}`-less C formats filled in: "%d", "%s", "%.2f", "%q"...
    format: (format: string, ...any) -> string,
    --- The text from character i to j (negative counts from the end).
    sub: (s: string, i: number, j: number?) -> string,
    --- How many bytes long the text is.
    len: (s: string) -> number,
    --- The text in lower case.
    lower: (s: string) -> string,
    --- The text in upper case.
    upper: (s: string) -> string,
    --- The text n times, `separator` between.
    rep: (s: string, n: number, separator: string?) -> string,
    --- The text backwards.
    reverse: (s: string) -> string,
    --- Where the pattern first matches, from `init`: its start and end, and its captures.
    find: (s: string, pattern: string, init: number?, plain: boolean?) -> (number?, number?, ...string),
    --- The pattern's first match, or its captures.
    match: (s: string, pattern: string, init: number?) -> ...string?,
    --- Every match of the pattern, for a generic for.
    gmatch: (s: string, pattern: string) -> () -> ...string,
    --- The text with each match of the pattern replaced, and how many there were.
    gsub: (s: string, pattern: string, replacement: string | {} | (...string) -> string, n: number?) -> (string, number),
    --- The bytes from i to j, as numbers.
    byte: (s: string, i: number?, j: number?) -> ...number,
    --- The text of the bytes.
    char: (...number) -> string,
    --- The text cut at each `separator` (",", by default).
    split: (s: string, separator: string?) -> {string},
}

--- Tables.
declare table: {
    --- Puts the value at the end of the array, or at `pos`, moving the rest up.
    insert: (t: {any}, pos: any, value: any?) -> (),
    --- Takes the value at `pos` (the last by default) out of the array, moving the rest down, and returns it.
    remove: (t: {any}, pos: number?) -> any,
    --- The index of the value in the array, from `init`, or nil.
    find: (t: {any}, value: any, init: number?) -> number?,
    --- Sorts the array in place, by `<` or by `comp`.
    sort: (t: {any}, comp: ((any, any) -> boolean)?) -> (),
    --- The array's strings and numbers joined, `separator` between.
    concat: (t: {any}, separator: string?, i: number?, j: number?) -> string,
    --- The array's values from i to j.
    unpack: (t: {any}, i: number?, j: number?) -> ...any,
    --- The arguments as an array, with `n` their count.
    pack: (...any) -> {any},
    --- Removes every key of the table, keeping it.
    clear: (t: {any}) -> (),
    --- A copy of the table (not of what it holds).
    clone: (t: {any}) -> {any},
    --- Makes the table read-only, and returns it.
    freeze: (t: {any}) -> {any},
    --- Whether the table is read-only.
    isfrozen: (t: {any}) -> boolean,
    --- An array of n values, each `value`.
    create: (n: number, value: any?) -> {any},
    --- Copies a1's values from f to e into a2 from t.
    move: (a1: {any}, f: number, e: number, t: number, a2: {any}?) -> {any},
}

--- Threads that can yield.
declare coroutine: {
    --- A new thread that runs f.
    create: (f: (...any) -> ...any) -> thread,
    --- Runs the thread until it yields or ends: true and what it gave, or false and the error.
    resume: (co: thread, ...any) -> (boolean, ...any),
    --- Yields the running thread, giving its arguments to resume.
    yield: (...any) -> ...any,
    --- "running", "suspended", "normal" or "dead".
    status: (co: thread) -> string,
    --- A function that resumes a new thread of f each call, raising its errors.
    wrap: (f: (...any) -> ...any) -> (...any) -> ...any,
    --- The running thread.
    running: () -> thread,
    --- Whether the running thread can yield.
    isyieldable: () -> boolean,
    --- Closes a suspended or dead thread.
    close: (co: thread) -> (boolean, any),
}

--- The clock and the date.
declare os: {
    --- Seconds since the Unix epoch, or of the given date.
    time: (date: {}?) -> number,
    --- Seconds of CPU time, for measuring.
    clock: () -> number,
    --- The date as text, or as a table with "*t".
    date: (format: string?, time: number?) -> any,
    --- Seconds from t1 to t2.
    difftime: (t2: number, t1: number?) -> number,
}
"##;

/// A member of a type: a property, an event, a field, or a method (one overload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// Its name.
    pub name: String,
    /// Its type, or for a method its whole signature (`function Name(self, ...): Ret`).
    pub ty: String,
    /// Whether it is a method, called with `:`.
    pub method: bool,
    /// What it is, from its `---` lines.
    pub doc: String,
}

impl Member {
    /// The type its value has, or a method's (or a function field's) result: a type's name when that is all it is.
    pub fn result(&self) -> Option<String> {
        if self.method {
            return return_type(&self.ty);
        }
        if self.ty.contains("->") {
            return function_types(&self.ty).last().and_then(|(_, ret)| simple_type(ret));
        }
        simple_type(&self.ty)
    }

    /// The result when called with `literal` as its first argument, for a function field (`Instance.new`): the
    /// overload that takes that string.
    pub fn result_for(&self, literal: Option<&str>) -> Option<String> {
        let overloads = function_types(&self.ty);
        let matched = literal.and_then(|literal| {
            overloads
                .iter()
                .find(|(first, _)| first.as_deref() == Some(literal))
                .and_then(|(_, ret)| simple_type(ret))
        });
        matched.or_else(|| self.result())
    }

    /// Whether it is a function (a method, or a field whose type is one): what completion puts `(` after.
    pub fn is_function(&self) -> bool {
        self.method || self.ty.trim_start().starts_with('(')
    }
}

/// A type: a class, a data type, an enum, or a table type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeDef {
    /// Its name.
    pub name: String,
    /// The type it extends.
    pub base: Option<String>,
    /// What it is.
    pub doc: String,
    /// Its own members, in order (a method once per overload).
    pub members: Vec<Member>,
}

/// A global: a value, a function, or a table of fields (a library, a data type's constructors).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Global {
    /// Its name.
    pub name: String,
    /// Its type: a type's name, a table, or for a function its signature.
    pub ty: String,
    /// Whether it is a function.
    pub function: bool,
    /// What it is.
    pub doc: String,
    /// A table's fields.
    pub fields: Vec<Member>,
}

/// The API.
#[derive(Clone, Debug, Default)]
pub struct Api {
    /// Every type, by name.
    pub types: HashMap<String, TypeDef>,
    /// Every global, by name.
    pub globals: BTreeMap<String, Global>,
}

/// The API every script sees: Metallic's and Luau's, read once.
pub fn api() -> &'static Api {
    static API: OnceLock<Api> = OnceLock::new();
    API.get_or_init(|| {
        let mut api = Api::parse(DEFINITIONS);
        let luau = Api::parse(LUAU);
        api.types.extend(luau.types);
        api.globals.extend(luau.globals);
        api
    })
}

impl Api {
    /// The API `text` declares.
    pub fn parse(text: &str) -> Self {
        let mut api = Api::default();
        let mut doc: Vec<String> = Vec::new();
        let mut lines = text.lines().peekable();
        while let Some(raw) = lines.next() {
            let line = raw.trim();
            if let Some(rest) = line.strip_prefix("---") {
                if !rest.starts_with('-') {
                    doc.push(rest.trim().to_owned());
                }
                continue;
            }
            if line.is_empty() || line.starts_with("--") {
                doc.clear();
                continue;
            }
            let taken = std::mem::take(&mut doc).join(" ");
            if let Some(rest) = line.strip_prefix("declare extern type ") {
                let (head, body_follows) = match rest.strip_suffix(" with end") {
                    Some(head) => (head, false),
                    None => (rest.strip_suffix(" with").unwrap_or(rest), true),
                };
                let (name, base) = match head.split_once(" extends ") {
                    Some((name, base)) => (name.trim(), Some(base.trim().to_owned())),
                    None => (head.trim(), None),
                };
                let members = if body_follows {
                    read_members(&mut lines, "end")
                } else {
                    Vec::new()
                };
                api.types.insert(
                    name.to_owned(),
                    TypeDef {
                        name: name.to_owned(),
                        base,
                        doc: taken,
                        members,
                    },
                );
            } else if let Some(rest) = line.strip_prefix("export type ") {
                let name = rest.split(['<', ' ', '=']).next().unwrap_or_default();
                let members = if rest.trim_end().ends_with('{') {
                    read_fields(&mut lines)
                } else {
                    Vec::new()
                };
                api.types.insert(
                    name.to_owned(),
                    TypeDef {
                        name: name.to_owned(),
                        base: None,
                        doc: taken,
                        members,
                    },
                );
            } else if let Some(rest) = line.strip_prefix("declare function ") {
                let name = rest.split(['(', '<']).next().unwrap_or_default().trim();
                api.globals.insert(
                    name.to_owned(),
                    Global {
                        name: name.to_owned(),
                        ty: format!("function {rest}"),
                        function: true,
                        doc: taken,
                        fields: Vec::new(),
                    },
                );
            } else if let Some(rest) = line.strip_prefix("declare ") {
                let Some((name, ty)) = rest.split_once(':') else {
                    continue;
                };
                let (name, ty) = (name.trim(), ty.trim());
                let fields = if ty == "{" { read_fields(&mut lines) } else { Vec::new() };
                api.globals.insert(
                    name.to_owned(),
                    Global {
                        name: name.to_owned(),
                        ty: ty.to_owned(),
                        function: false,
                        doc: taken,
                        fields,
                    },
                );
            }
        }
        api
    }

    /// The member `name` of type `ty` or the nearest type it extends (its first overload).
    pub fn member(&self, ty: &str, name: &str) -> Option<&Member> {
        self.chain(ty)
            .find_map(|def| def.members.iter().find(|member| member.name == name))
    }

    /// Every overload of `ty`'s member `name`, from the nearest type that has it.
    pub fn overloads(&self, ty: &str, name: &str) -> Vec<&Member> {
        self.chain(ty)
            .map(|def| {
                def.members
                    .iter()
                    .filter(|member| member.name == name)
                    .collect::<Vec<_>>()
            })
            .find(|found| !found.is_empty())
            .unwrap_or_default()
    }

    /// Every member of `ty` and the types it extends, each name once (the nearest's), in order, without metamethods.
    pub fn members(&self, ty: &str) -> Vec<&Member> {
        let mut seen = std::collections::HashSet::new();
        self.chain(ty)
            .flat_map(|def| def.members.iter())
            .filter(|member| !member.name.starts_with("__") && seen.insert(member.name.clone()))
            .collect()
    }

    /// `ty` and the types it extends, nearest first.
    pub fn chain<'a>(&'a self, ty: &str) -> impl Iterator<Item = &'a TypeDef> + 'a {
        let mut next = self.types.get(ty);
        let mut steps = 0;
        std::iter::from_fn(move || {
            let def = next?;
            steps += 1;
            next = def
                .base
                .as_deref()
                .and_then(|base| self.types.get(base))
                .filter(|_| steps < 64);
            Some(def)
        })
    }

    /// Whether `ty` is `ancestor` or extends it.
    pub fn is_a(&self, ty: &str, ancestor: &str) -> bool {
        self.chain(ty).any(|def| def.name == ancestor)
    }
}

/// The members of a `declare extern type ... with` block, up to its `end`.
fn read_members<'a>(lines: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>, end: &str) -> Vec<Member> {
    let mut members = Vec::new();
    let mut doc: Vec<String> = Vec::new();
    for raw in lines.by_ref() {
        let line = raw.trim();
        if line == end {
            break;
        }
        if let Some(rest) = line.strip_prefix("---") {
            doc.push(rest.trim().to_owned());
            continue;
        }
        let taken = std::mem::take(&mut doc).join(" ");
        if let Some(rest) = line.strip_prefix("function ") {
            let name = rest.split('(').next().unwrap_or_default().trim();
            members.push(Member {
                name: name.to_owned(),
                ty: line.to_owned(),
                method: rest.contains("(self"),
                doc: taken,
            });
        } else if let Some((name, ty)) = line.trim_start_matches("read ").split_once(':') {
            members.push(Member {
                name: name.trim().to_owned(),
                ty: ty.trim().to_owned(),
                method: false,
                doc: taken,
            });
        }
    }
    members
}

/// The fields of a table type, up to its closing `}`: `name: type,`, a type running over lines until its comma.
fn read_fields<'a>(lines: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>) -> Vec<Member> {
    let mut fields = Vec::new();
    let mut doc: Vec<String> = Vec::new();
    let mut pending: Option<(String, String, String)> = None;
    for raw in lines.by_ref() {
        let line = raw.trim();
        if pending.is_none() {
            if line.starts_with('}') {
                break;
            }
            if let Some(rest) = line.strip_prefix("---") {
                doc.push(rest.trim().to_owned());
                continue;
            }
            let Some((name, ty)) = line.split_once(':') else {
                continue;
            };
            pending = Some((
                name.trim().to_owned(),
                ty.trim().to_owned(),
                std::mem::take(&mut doc).join(" "),
            ));
        } else if let Some((_, ty, _)) = pending.as_mut() {
            ty.push(' ');
            ty.push_str(line);
        }
        let done = pending
            .as_ref()
            .is_some_and(|(_, ty, _)| depth(ty) == 0 && ty.ends_with(','));
        if done && let Some((name, ty, doc)) = pending.take() {
            let ty = ty.trim_end_matches(',').trim().to_owned();
            let method = ty.starts_with("(self");
            fields.push(Member { name, ty, method, doc });
        }
    }
    fields
}

/// How many more brackets `text` opens than it closes.
fn depth(text: &str) -> i32 {
    // The `>` of `->` closes nothing.
    text.replace("->", "  ").chars().fold(0, |depth, c| match c {
        '(' | '{' | '[' | '<' => depth + 1,
        ')' | '}' | ']' | '>' => depth - 1,
        _ => depth,
    })
}

/// The type's name when that is all `text` is: `Part`, `Instance?`, `read Workspace`, `ScriptSignal<Instance>`.
pub fn simple_type(text: &str) -> Option<String> {
    let text = text.trim().trim_start_matches("read ").trim();
    let text = text.split('<').next().unwrap_or(text).trim().trim_end_matches('?');
    (!text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .then(|| text.to_owned())
        .filter(|name| !matches!(name.as_str(), "any" | "nil" | "never"))
}

/// A method signature's result type: `function Clone(self): Instance?` is `Instance`.
pub fn return_type(signature: &str) -> Option<String> {
    let close = closing_paren(signature)?;
    let rest = signature[close + 1..].trim().strip_prefix(':')?;
    simple_type(rest)
}

/// Where the parenthesis opening at `signature`'s first `(` closes.
fn closing_paren(signature: &str) -> Option<usize> {
    let open = signature.find('(')?;
    let mut depth = 0;
    for (at, c) in signature[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + at);
                }
            }
            _ => {}
        }
    }
    None
}

/// The overloads of a function type (`(a: T) -> R`, or `((x: "A") -> A) & ((x: "B") -> B)`): each one's first
/// parameter when it is a string literal (unquoted), and its result.
pub fn function_types(text: &str) -> Vec<(Option<String>, String)> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (at, c) in text.char_indices() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            '&' if depth == 0 => {
                parts.push(&text[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
        .into_iter()
        .filter_map(|part| {
            let mut part = part.trim();
            // `((params) -> R)`: the outer parentheses.
            if part.starts_with("((") && part.ends_with(')') {
                part = &part[1..part.len() - 1];
            }
            let close = closing_paren(part)?;
            let params = &part[1..close];
            let ret = part[close + 1..].trim().strip_prefix("->")?.trim().to_owned();
            let first = params
                .split(',')
                .next()
                .and_then(|param| param.split_once(':'))
                .map(|(_, ty)| ty.trim())
                .and_then(|ty| ty.strip_prefix('"').and_then(|ty| ty.strip_suffix('"')))
                .map(str::to_owned);
            Some((first, ret))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_definitions_are_read() {
        let api = api();
        // A class, its base, its members with their docs.
        let part = &api.types["Part"];
        assert_eq!(part.base.as_deref(), Some("BasePart"));
        assert!(api.is_a("Part", "Instance"));
        let anchored = api.member("Part", "Anchored").unwrap();
        assert_eq!(anchored.ty, "boolean");
        assert!(anchored.doc.contains("stays where it is"), "{}", anchored.doc);
        let clone = api.member("Part", "Clone").unwrap();
        assert!(clone.method && clone.is_function());
        assert_eq!(clone.result().as_deref(), Some("Instance"));
        // Every member once, the nearest first.
        let names: Vec<&str> = api.members("Part").iter().map(|member| member.name.as_str()).collect();
        assert!(names.contains(&"Shape") && names.contains(&"Name") && names.contains(&"FindFirstChild"));
        assert_eq!(names.iter().filter(|name| **name == "Name").count(), 1);
        // GetService's overloads.
        let services = api.overloads("DataModel", "GetService");
        assert!(services.len() > 3, "{services:?}");
        // A data type and its constructors.
        assert_eq!(api.member("Vector3", "Magnitude").unwrap().ty, "number");
        let new = api.globals["Vector3"]
            .fields
            .iter()
            .find(|field| field.name == "new")
            .unwrap();
        assert_eq!(new.result().as_deref(), Some("Vector3"));
        // Instance.new's overloads, by the class named.
        let instance_new = api.globals["Instance"]
            .fields
            .iter()
            .find(|field| field.name == "new")
            .unwrap();
        assert_eq!(instance_new.result_for(Some("Part")).as_deref(), Some("Part"));
        assert_eq!(instance_new.result_for(Some("Folder")).as_deref(), Some("Folder"));
        // Globals: Metallic's and Luau's.
        assert_eq!(api.globals["game"].ty, "DataModel");
        assert!(api.globals["print"].function);
        assert!(api.globals["math"].fields.iter().any(|field| field.name == "clamp"));
        assert!(api.globals["task"].fields.iter().any(|field| field.name == "wait"));
        // Enums.
        assert!(api.globals["Enum"].fields.iter().any(|field| field.name == "Material"));
        assert!(api.member("EnumMaterial_INTERNAL", "Plastic").is_some());
    }

    #[test]
    fn types_are_read_from_their_text() {
        assert_eq!(simple_type("Instance?").as_deref(), Some("Instance"));
        assert_eq!(
            simple_type("ScriptSignal<Instance, Instance?>").as_deref(),
            Some("ScriptSignal")
        );
        assert_eq!(simple_type("{Instance}"), None);
        assert_eq!(simple_type("any"), None);
        assert_eq!(
            return_type("function FindFirstChild(self, name: string, recursive: boolean?): Instance?").as_deref(),
            Some("Instance")
        );
        assert_eq!(
            function_types("((className: \"Part\", parent: Instance?) -> Part) & ((className: \"Folder\") -> Folder)"),
            [
                (Some("Part".to_owned()), "Part".to_owned()),
                (Some("Folder".to_owned()), "Folder".to_owned())
            ]
        );
    }
}
