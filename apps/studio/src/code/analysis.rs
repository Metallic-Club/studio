//! What a script's code says (#530): its scopes, what is declared in them (locals, functions, parameters, loop
//! variables, the fields of local tables, the file's globals), what each name refers to, the types of what can be
//! worked out, and the warnings: unknown globals, unused locals, and members a known class does not have.
//!
//! It reads the tokens with a light parser that never fails, since code being typed is unfinished: it follows the
//! blocks (`function`, `do`, `then`, `repeat` ... `end` or `until`) and the declarations, and skips type annotations.
//! It is not Luau's type checker: what it cannot work out is unknown, and unknown is never warned about.
//!
//! Types come from the API ([`super::api`]), and from the game itself: `workspace.Baseplate` is that part, so its
//! members and children are known, as Roblox Studio knows them. A ModuleScript's members come from its own code,
//! through `require(...)` ([`Env::module`]).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use metallic_engine::metallic_core::{DataModel, Id};

use super::api::{Api, api};
use super::lexer::{Token, TokenKind, lex};

/// What a declaration is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclKind {
    /// `local x`.
    Local,
    /// `local function f`.
    LocalFunction,
    /// A function's parameter (or a method's `self`).
    Param,
    /// A `for` loop's variable.
    LoopVar,
    /// A global the file sets: `function f()` or `x = ...` outside any local.
    Global,
    /// A field of a local table (`function M.f()`, `M.x = 1`, `{ x = 1 }`): `owner` is the table's declaration.
    Field {
        /// The table's declaration.
        owner: usize,
    },
}

/// Something the code declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    /// Its name.
    pub name: String,
    /// Where its name is, in bytes.
    pub range: Range<usize>,
    /// What it is.
    pub kind: DeclKind,
    /// The scope it is in.
    pub scope: usize,
    /// From which byte on it can be named.
    pub visible_from: usize,
    /// The token (an index into [`Analysis::tokens`]) its value starts at, for its type: what follows its `=`.
    pub value: Option<usize>,
    /// Whether it is a function (declared with `function`).
    pub function: bool,
}

/// A block of code and the scope it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    /// The scope it is in.
    pub parent: Option<usize>,
    /// Its bytes.
    pub range: Range<usize>,
}

/// What a name read refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ref {
    /// A declaration of the file.
    Decl(usize),
    /// A global the file does not declare: the API's, or unknown.
    Global,
}

/// How bad a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    /// The code may not do what it says.
    Warning,
    /// The code does not compile.
    Error,
}

/// An error or warning about part of the code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The bytes it is about.
    pub range: Range<usize>,
    /// How bad it is.
    pub severity: Severity,
    /// What is wrong.
    pub message: String,
}

/// What a value is, as far as the code says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    /// Not known.
    Unknown,
    /// A type of the API: a class (no instance known), a data type, an enum.
    Named(String),
    /// An instance of the game, known: its members are its class's, and its children are known too.
    Live(Id),
    /// A global table of the API: a library (`math`), a data type's constructors (`Vector3`), `Enum`, `Instance`.
    Library(String),
    /// A table declared in this file (its declaration), whose fields are its [`DeclKind::Field`]s.
    Table(usize),
    /// A table another script returns: that ModuleScript, and the table's declaration in its code.
    ModuleTable(Id, usize),
    /// A ModuleScript, as an instance (what `require` takes).
    Number,
    /// A string.
    String,
    /// `true` or `false`.
    Boolean,
    /// A function.
    Function,
}

/// What a script's code says.
#[derive(Debug, Default)]
pub struct Analysis {
    /// The code.
    pub source: String,
    /// Its tokens, comments left out, the expressions inside interpolated strings in.
    pub tokens: Vec<Token>,
    /// What it declares, in order.
    pub decls: Vec<Decl>,
    /// Its scopes, the file's first.
    pub scopes: Vec<Scope>,
    /// What each name read refers to, by its token.
    pub refs: HashMap<usize, Ref>,
    /// The tokens that are read: each declaration's uses.
    pub uses: Vec<usize>,
    /// The tokens of type annotations, which name types and not values.
    pub type_tokens: HashSet<usize>,
    /// The token the file's last top-level `return` returns, for a ModuleScript's members.
    pub returned: Option<usize>,
}

/// The statement keywords that cannot follow an expression on the same statement.
const STATEMENTS: [&str; 12] = [
    "local", "function", "return", "if", "for", "while", "repeat", "do", "end", "else", "elseif", "until",
];

impl Analysis {
    /// Reads `source`.
    pub fn new(source: &str) -> Self {
        let mut analysis = Analysis {
            source: source.to_owned(),
            tokens: tokens(source),
            scopes: vec![Scope {
                parent: None,
                range: 0..source.len(),
            }],
            ..Analysis::default()
        };
        Parser::new(&mut analysis).run();
        analysis
    }

    /// The text of token `index`.
    pub fn text(&self, index: usize) -> &str {
        self.tokens.get(index).map_or("", |token| token.text(&self.source))
    }

    /// Whether token `index` is the symbol or keyword `text`.
    fn is(&self, index: usize, text: &str) -> bool {
        self.tokens.get(index).is_some_and(|token| token.is(&self.source, text))
    }

    /// Whether token `index` is a name.
    fn is_name(&self, index: usize) -> bool {
        self.tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::Name)
    }

    /// The token at byte `at` (a name being typed ends at the cursor: the token ending there counts).
    pub fn token_at(&self, at: usize) -> Option<usize> {
        self.tokens
            .iter()
            .position(|token| token.start <= at && at < token.end)
            .or_else(|| {
                self.tokens
                    .iter()
                    .position(|token| token.end == at && token.end > token.start)
            })
    }

    /// The innermost scope holding byte `at`.
    pub fn scope_at(&self, at: usize) -> usize {
        let mut best = 0;
        for (index, scope) in self.scopes.iter().enumerate() {
            if scope.range.start <= at && at < scope.range.end.max(scope.range.start + 1) {
                let inner = self.scopes[best].range.len() >= scope.range.len();
                if inner {
                    best = index;
                }
            }
        }
        best
    }

    /// Whether `scope` is `outer` or inside it.
    fn within(&self, mut scope: usize, outer: usize) -> bool {
        loop {
            if scope == outer {
                return true;
            }
            match self.scopes[scope].parent {
                Some(parent) => scope = parent,
                None => return false,
            }
        }
    }

    /// The declaration `name` refers to at byte `at`: the innermost one visible there, or a global of the file.
    pub fn resolve(&self, name: &str, at: usize) -> Option<usize> {
        let scope = self.scope_at(at);
        let local = self
            .decls
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, decl)| decl.name == name && !matches!(decl.kind, DeclKind::Field { .. } | DeclKind::Global))
            .filter(|(_, decl)| decl.visible_from <= at && self.within(scope, decl.scope))
            .max_by_key(|(_, decl)| decl.visible_from)
            .map(|(index, _)| index);
        local.or_else(|| {
            self.decls
                .iter()
                .position(|decl| decl.name == name && decl.kind == DeclKind::Global)
        })
    }

    /// Every declaration visible at byte `at`, each name once (the innermost): what completion offers.
    pub fn visible(&self, at: usize) -> Vec<usize> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let scope = self.scope_at(at);
        let mut candidates: Vec<usize> = (0..self.decls.len())
            .filter(|&index| {
                let decl = &self.decls[index];
                match decl.kind {
                    DeclKind::Field { .. } => false,
                    DeclKind::Global => true,
                    _ => decl.visible_from <= at && self.within(scope, decl.scope),
                }
            })
            .collect();
        candidates.sort_by_key(|&index| std::cmp::Reverse(self.decls[index].visible_from));
        for index in candidates {
            if seen.insert(self.decls[index].name.clone()) {
                out.push(index);
            }
        }
        out
    }

    /// The fields of the table declared by `owner`, each name once (its first declaration).
    pub fn fields(&self, owner: usize) -> Vec<usize> {
        let mut seen = HashSet::new();
        (0..self.decls.len())
            .filter(|&index| self.decls[index].kind == DeclKind::Field { owner })
            .filter(|&index| seen.insert(self.decls[index].name.clone()))
            .collect()
    }

    /// The field `name` of the table declared by `owner`.
    pub fn field(&self, owner: usize, name: &str) -> Option<usize> {
        self.fields(owner)
            .into_iter()
            .find(|&index| self.decls[index].name == name)
    }

    /// The declaration whose name is token `index`, if it is one.
    pub fn decl_of_token(&self, index: usize) -> Option<usize> {
        let range = self.tokens.get(index)?.start..self.tokens.get(index)?.end;
        self.decls.iter().position(|decl| decl.range == range)
    }
}

/// `source`'s tokens without its comments, with the expressions of its interpolated strings (`` `{x}` ``) as tokens of
/// their own.
fn tokens(source: &str) -> Vec<Token> {
    let mut out = Vec::new();
    for token in lex(source) {
        match token.kind {
            TokenKind::Comment => {}
            TokenKind::String if token.text(source).starts_with('`') => {
                out.push(token);
                let text = token.text(source);
                let mut depth = 0usize;
                let mut start = 0;
                for (at, c) in text.char_indices() {
                    match c {
                        '{' => {
                            if depth == 0 {
                                start = at + 1;
                            }
                            depth += 1;
                        }
                        '}' if depth > 0 => {
                            depth -= 1;
                            if depth == 0 {
                                let inner = &text[start..at];
                                let offset = token.start + start;
                                out.extend(
                                    lex(inner)
                                        .into_iter()
                                        .filter(|t| t.kind != TokenKind::Comment)
                                        .map(|t| Token {
                                            start: t.start + offset,
                                            end: t.end + offset,
                                            ..t
                                        }),
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => out.push(token),
        }
    }
    out
}

/// The parser's state over one [`Analysis`].
struct Parser<'a> {
    a: &'a mut Analysis,
    /// The scopes open, innermost last.
    open: Vec<usize>,
    /// Whether the next `do` belongs to a `for` or `while`, whose scope is open already.
    loop_do: Vec<bool>,
    /// The brackets open (`(`, `[`, `{`), each `{` with the table declaration it makes, if any.
    brackets: Vec<(u8, Option<usize>)>,
    /// The declaration whose value starts at the next token: a table constructor there is its table.
    value_of: Option<(usize, usize)>,
    /// What is a block's kind, for its `end`: whether it is a function's (its scope).
    blocks: Vec<usize>,
}

impl<'a> Parser<'a> {
    fn new(a: &'a mut Analysis) -> Self {
        Self {
            a,
            open: vec![0],
            loop_do: Vec::new(),
            brackets: Vec::new(),
            value_of: None,
            blocks: Vec::new(),
        }
    }

    fn scope(&self) -> usize {
        self.open.last().copied().unwrap_or(0)
    }

    fn start(&self, index: usize) -> usize {
        self.a
            .tokens
            .get(index)
            .map_or(self.a.source.len(), |token| token.start)
    }

    fn end_of(&self, index: usize) -> usize {
        self.a.tokens.get(index).map_or(self.a.source.len(), |token| token.end)
    }

    /// Opens a scope at byte `at`.
    fn push(&mut self, at: usize) {
        let index = self.a.scopes.len();
        self.a.scopes.push(Scope {
            parent: Some(self.scope()),
            range: at..self.a.source.len(),
        });
        self.open.push(index);
        self.blocks.push(index);
    }

    /// Closes the innermost scope at byte `at` (never the file's).
    fn pop(&mut self, at: usize) {
        if self.open.len() > 1
            && let Some(index) = self.open.pop()
        {
            self.a.scopes[index].range.end = at;
            self.blocks.pop();
        }
    }

    fn declare(&mut self, name_token: usize, kind: DeclKind, visible_from: usize, function: bool) -> usize {
        let index = self.a.decls.len();
        let range = self.start(name_token)..self.end_of(name_token);
        self.a.decls.push(Decl {
            name: self.a.text(name_token).to_owned(),
            range,
            kind,
            scope: self.scope(),
            visible_from,
            value: None,
            function,
        });
        index
    }

    /// Marks the type annotation starting at token `index` (after a `:` or `::`), and returns the token after it: it
    /// runs to a `=`, `,` or `)` outside its own brackets, or to the end of its line outside them.
    fn skip_type(&mut self, mut index: usize) -> usize {
        let line = self.line(index);
        let mut depth = 0i32;
        while index < self.a.tokens.len() {
            let text = self.a.text(index);
            let token = self.a.tokens[index];
            if depth == 0 {
                if matches!(text, "=" | "," | ")" | "]" | "}" | ";") && token.kind == TokenKind::Symbol {
                    break;
                }
                if self.line(index) != line && !matches!(text, "|" | "&" | "->" | "?") {
                    break;
                }
                if token.kind == TokenKind::Keyword && text != "nil" && text != "typeof" {
                    break;
                }
            }
            match text {
                "(" | "{" | "[" | "<" => depth += 1,
                ")" | "}" | "]" | ">" => depth -= 1,
                _ => {}
            }
            if depth < 0 {
                break;
            }
            self.a.type_tokens.insert(index);
            index += 1;
        }
        index
    }

    /// The line token `index` is on.
    fn line(&self, index: usize) -> usize {
        let at = self.start(index);
        self.a.source[..at.min(self.a.source.len())].matches('\n').count()
    }

    /// Reads a function's parameters from its `(` at `index` (and a method's `self`), and its return type; returns
    /// the token after them.
    fn params(&mut self, mut index: usize, method: bool) -> usize {
        // `function f<T>(...)`: the generic parameters are types.
        if self.a.is(index, "<") {
            while index < self.a.tokens.len() && !self.a.is(index, ">") {
                self.a.type_tokens.insert(index);
                index += 1;
            }
            self.a.type_tokens.insert(index);
            index += 1;
        }
        if method {
            let at = self.start(index);
            let decl = self.a.decls.len();
            self.a.decls.push(Decl {
                name: "self".to_owned(),
                range: at..at,
                kind: DeclKind::Param,
                scope: self.scope(),
                visible_from: at,
                value: None,
                function: false,
            });
            let _ = decl;
        }
        if !self.a.is(index, "(") {
            return index;
        }
        index += 1;
        while index < self.a.tokens.len() && !self.a.is(index, ")") {
            if self.a.is_name(index) {
                let at = self.start(index);
                self.declare(index, DeclKind::Param, at, false);
                index += 1;
                if self.a.is(index, ":") {
                    index = self.skip_type(index + 1);
                }
            } else {
                // `...`, `,`, or `...: type`.
                if self.a.is(index, ":") {
                    index = self.skip_type(index + 1);
                    continue;
                }
                index += 1;
            }
        }
        index += 1;
        if self.a.is(index, ":") {
            index = self.skip_type(index + 1);
        }
        index
    }

    fn run(&mut self) {
        let mut index = 0;
        while index < self.a.tokens.len() {
            index = self.step(index);
        }
        let end = self.a.source.len();
        while self.open.len() > 1 {
            self.pop(end);
        }
    }

    /// Reads from token `index`; returns the next token to read.
    fn step(&mut self, index: usize) -> usize {
        let token = self.a.tokens[index];
        let text = self.a.text(index).to_owned();
        let value_start = self.value_of.filter(|(at, _)| *at == index).map(|(_, decl)| decl);
        if self.value_of.is_some_and(|(at, _)| at <= index) {
            self.value_of = None;
        }
        match (token.kind, text.as_str()) {
            (TokenKind::Keyword, "local") => {
                if self.a.is(index + 1, "function") && self.a.is_name(index + 2) {
                    let at = self.start(index + 2);
                    self.declare(index + 2, DeclKind::LocalFunction, at, true);
                    self.push(self.start(index + 1));
                    return self.params(index + 3, false);
                }
                // `local a: T, b <const> = ...`
                let mut at = index + 1;
                let mut declared = Vec::new();
                while self.a.is_name(at) {
                    let decl = self.declare(at, DeclKind::Local, self.end_of(at), false);
                    declared.push(decl);
                    at += 1;
                    if self.a.is(at, "<") {
                        while at < self.a.tokens.len() && !self.a.is(at, ">") {
                            at += 1;
                        }
                        at += 1;
                    }
                    if self.a.is(at, ":") {
                        at = self.skip_type(at + 1);
                    }
                    if self.a.is(at, ",") {
                        at += 1;
                    } else {
                        break;
                    }
                }
                if self.a.is(at, "=") {
                    let values = self.values(at + 1);
                    for (decl, value) in declared.iter().zip(values) {
                        self.a.decls[*decl].value = Some(value);
                    }
                    if let (Some(&first), Some(value)) = (declared.first(), self.a.decls[declared[0]].value) {
                        self.value_of = Some((value, first));
                    }
                    // What follows the names is visible to nothing declared here: they are seen from the statement's
                    // value on, near enough.
                }
                at
            }
            (TokenKind::Keyword, "function") => self.function(index),
            (TokenKind::Keyword, "for") => {
                self.push(token.start);
                self.loop_do.push(true);
                let mut at = index + 1;
                while self.a.is_name(at) {
                    let from = self.end_of(at);
                    self.declare(at, DeclKind::LoopVar, from, false);
                    at += 1;
                    if self.a.is(at, ":") {
                        at = self.skip_type(at + 1);
                    }
                    if self.a.is(at, ",") {
                        at += 1;
                    } else {
                        break;
                    }
                }
                at
            }
            (TokenKind::Keyword, "while") => {
                self.push(token.start);
                self.loop_do.push(true);
                index + 1
            }
            (TokenKind::Keyword, "do") => {
                if self.loop_do.pop() != Some(true) {
                    self.push(token.start);
                }
                index + 1
            }
            (TokenKind::Keyword, "then") => {
                self.push(token.end);
                index + 1
            }
            (TokenKind::Keyword, "elseif") => {
                self.pop(token.start);
                index + 1
            }
            (TokenKind::Keyword, "else") => {
                self.pop(token.start);
                self.push(token.end);
                index + 1
            }
            (TokenKind::Keyword, "end") => {
                self.pop(token.end);
                index + 1
            }
            (TokenKind::Keyword, "repeat") => {
                self.push(token.end);
                index + 1
            }
            (TokenKind::Keyword, "until") => {
                self.pop(token.start);
                index + 1
            }
            (TokenKind::Keyword, "return") => {
                if self.open.len() == 1 && index + 1 < self.a.tokens.len() {
                    self.a.returned = Some(index + 1);
                }
                index + 1
            }
            (TokenKind::Symbol, "::") => self.skip_type(index + 1),
            (TokenKind::Symbol, "(" | "[") => {
                self.brackets.push((text.as_bytes()[0], None));
                index + 1
            }
            (TokenKind::Symbol, "{") => {
                self.brackets.push((b'{', value_start));
                index + 1
            }
            (TokenKind::Symbol, ")" | "]" | "}") => {
                self.brackets.pop();
                index + 1
            }
            (TokenKind::Name, _) => self.name(index),
            _ => index + 1,
        }
    }

    /// The first token of each value of a list starting at token `index` (commas outside brackets part them), up to the
    /// statement's end.
    fn values(&self, index: usize) -> Vec<usize> {
        let mut out = vec![index];
        let mut depth = 0i32;
        let mut at = index;
        let line = self.line(index);
        while at < self.a.tokens.len() {
            let text = self.a.text(at);
            match text {
                "(" | "{" | "[" => depth += 1,
                ")" | "}" | "]" => depth -= 1,
                "," if depth == 0 => out.push(at + 1),
                _ => {}
            }
            if depth < 0 || (depth == 0 && self.line(at) != line && STATEMENTS.contains(&text)) {
                break;
            }
            if depth == 0 && self.line(at) > line + 1 {
                break;
            }
            at += 1;
        }
        out
    }

    /// `function` at `index`: a declaration (`function f`, `function M.f`, `function M:f`) or an expression.
    fn function(&mut self, index: usize) -> usize {
        let mut at = index + 1;
        if !self.a.is_name(at) {
            // An anonymous function.
            self.push(self.start(index));
            return self.params(at, false);
        }
        // `a.b.c` or `a:b`: the first name is read, the last is declared.
        let first = at;
        let mut path = vec![at];
        let mut method = false;
        while (self.a.is(at + 1, ".") || self.a.is(at + 1, ":")) && self.a.is_name(at + 2) {
            method = self.a.is(at + 1, ":");
            at += 2;
            path.push(at);
        }
        if path.len() == 1 {
            let name = self.a.text(first).to_owned();
            match self.a.resolve(&name, self.start(first)) {
                Some(decl) if self.a.decls[decl].kind != DeclKind::Global => {
                    self.a.refs.insert(first, Ref::Decl(decl));
                }
                _ => {
                    self.declare(first, DeclKind::Global, 0, true);
                }
            }
        } else {
            self.read(first);
            // The table's field, when the table is a local of this file and the path is one step.
            let owner = match self.a.refs.get(&first) {
                Some(Ref::Decl(decl)) if path.len() == 2 => Some(*decl),
                _ => None,
            };
            let last = *path.last().unwrap_or(&first);
            if let Some(owner) = owner {
                let from = self.start(last);
                self.declare(last, DeclKind::Field { owner }, from, true);
            }
        }
        self.push(self.start(index));
        self.params(at + 1, method)
    }

    /// A name at `index` that no keyword took: a member, a table constructor's field, an assignment, or a name read.
    fn name(&mut self, index: usize) -> usize {
        let after_dot = index > 0 && (self.a.is(index - 1, ".") || self.a.is(index - 1, ":"));
        let assigns = self.a.is(index + 1, "=");
        if after_dot {
            // `M.x = ...`: a field of the local table M.
            if assigns && self.a.is(index - 1, ".") && index >= 2 && self.a.is_name(index - 2) {
                let starts = index < 3 || !(self.a.is(index - 3, ".") || self.a.is(index - 3, ":"));
                if starts && let Some(Ref::Decl(owner)) = self.a.refs.get(&(index - 2)).copied() {
                    let from = self.start(index);
                    let decl = self.declare(index, DeclKind::Field { owner }, from, false);
                    self.a.decls[decl].value = Some(index + 2);
                    self.value_of = Some((index + 2, decl));
                }
            }
            return index + 1;
        }
        if self.a.type_tokens.contains(&index) {
            return index + 1;
        }
        // `{ x = 1 }`: a field of the table, if the table is a declaration's.
        if let Some(&(b'{', owner)) = self.brackets.last()
            && assigns
            && index > 0
            && (self.a.is(index - 1, "{") || self.a.is(index - 1, ",") || self.a.is(index - 1, ";"))
        {
            if let Some(owner) = owner {
                let from = self.start(index);
                let decl = self.declare(index, DeclKind::Field { owner }, from, false);
                self.a.decls[decl].value = Some(index + 2);
                self.value_of = Some((index + 2, decl));
            }
            return index + 1;
        }
        // `type X = ...` and `export type X = ...`: a type, to the end of its line or brackets.
        let text = self.a.text(index);
        if (text == "type" || text == "export") && self.statement_start(index) {
            let mut at = index + 1;
            if text == "export" && self.a.text(at) == "type" {
                at += 1;
            }
            if self.a.is_name(at) && (self.a.is(at + 1, "=") || self.a.is(at + 1, "<")) {
                self.a.type_tokens.insert(index);
                let mut next = at;
                while next < self.a.tokens.len() && !self.a.is(next, "=") {
                    self.a.type_tokens.insert(next);
                    next += 1;
                }
                return self.skip_type(next + 1);
            }
        }
        // `x = ...` with no local x: a global of the file.
        if assigns && self.brackets.is_empty() && self.statement_start(index) {
            let name = self.a.text(index).to_owned();
            match self.a.resolve(&name, self.start(index)) {
                Some(decl) => {
                    // An assignment, not a use.
                    self.a.refs.insert(index, Ref::Decl(decl));
                }
                None => {
                    let decl = self.declare(index, DeclKind::Global, 0, false);
                    self.a.decls[decl].value = Some(index + 2);
                }
            }
            return index + 1;
        }
        self.read(index);
        index + 1
    }

    /// Whether the token at `index` starts a statement, near enough: the file's first, or after a keyword that ends
    /// one or a `;`, or on a new line after a token that can end an expression.
    fn statement_start(&self, index: usize) -> bool {
        if index == 0 {
            return true;
        }
        let before = index - 1;
        let text = self.a.text(before);
        if matches!(text, "do" | "then" | "else" | "end" | ";" | "repeat") {
            return true;
        }
        if matches!(
            text,
            "=" | "," | "(" | "{" | "[" | "." | ":" | "and" | "or" | "not" | "local" | "return"
        ) {
            return false;
        }
        self.line(before) != self.line(index)
    }

    /// A name read at `index`: what it refers to, and a use of that.
    fn read(&mut self, index: usize) {
        let name = self.a.text(index).to_owned();
        match self.a.resolve(&name, self.start(index)) {
            Some(decl) => {
                self.a.refs.insert(index, Ref::Decl(decl));
                self.a.uses.push(index);
            }
            None => {
                self.a.refs.insert(index, Ref::Global);
            }
        }
    }
}

/// What resolving types may look at: the game, the script whose code it is, and other scripts' code.
pub struct Env<'a> {
    /// The game the script is in.
    pub dm: &'a DataModel,
    /// The script whose code it is (`script`).
    pub script: Option<Id>,
    /// The code of a ModuleScript, analysed: what `require` of it returns.
    pub module: &'a dyn Fn(Id) -> Option<Rc<Analysis>>,
}

impl Env<'_> {
    /// The API.
    pub fn api(&self) -> &'static Api {
        api()
    }

    /// The class name of the instance `id`.
    pub fn class(&self, id: Id) -> Option<String> {
        self.dm.class_name(id).ok().map(str::to_owned)
    }

    /// The child of `id` called `name`.
    pub fn child(&self, id: Id, name: &str) -> Option<Id> {
        self.dm
            .children(id)
            .ok()?
            .find(|&child| self.dm.name(child).is_ok_and(|child| child == name))
    }

    /// The service whose class is `class`.
    fn service(&self, class: &str) -> Option<Id> {
        let class = self.dm.registry().class_id(class)?;
        self.dm.service(class)
    }
}

/// How deep types are followed: a local's value's local's value... and modules requiring modules.
const DEPTH: usize = 24;

impl Analysis {
    /// The type of the value of declaration `decl`.
    pub fn decl_type(&self, decl: usize, env: &Env<'_>) -> Ty {
        self.decl_type_at(decl, env, 0)
    }

    fn decl_type_at(&self, decl: usize, env: &Env<'_>, depth: usize) -> Ty {
        if depth > DEPTH {
            return Ty::Unknown;
        }
        let Some(declaration) = self.decls.get(decl) else {
            return Ty::Unknown;
        };
        if declaration.name == "self" && declaration.kind == DeclKind::Param {
            // A method's self: the table it is a method of (`function M:f()`).
            let owner = self.decls.iter().rev().find_map(|other| match other.kind {
                DeclKind::Field { owner } if other.function && other.range.end <= declaration.range.start => {
                    Some(owner)
                }
                _ => None,
            });
            return owner.map_or(Ty::Unknown, Ty::Table);
        }
        if declaration.function {
            return Ty::Function;
        }
        match declaration.value {
            Some(value) => {
                if self.is(value, "{") {
                    // A table constructor: its fields are this declaration's.
                    return Ty::Table(decl);
                }
                let ty = self.expr_type_at(value, env, depth + 1).0;
                // `local M = {}` then `M.f = ...`: a table whose fields are its own, as well as anything it was set to.
                if ty == Ty::Unknown && !self.fields(decl).is_empty() {
                    return Ty::Table(decl);
                }
                ty
            }
            None => Ty::Unknown,
        }
    }

    /// The type of the expression starting at token `start`, and the token after it: a name or literal, then any
    /// `.name`, `:name(...)`, `(...)` and `[...]` after it.
    pub fn expr_type(&self, start: usize, env: &Env<'_>) -> (Ty, usize) {
        self.expr_type_at(start, env, 0)
    }

    fn expr_type_at(&self, start: usize, env: &Env<'_>, depth: usize) -> (Ty, usize) {
        self.expr_type_until(start, usize::MAX, env, depth)
    }

    /// As [`Analysis::expr_type`], reading no suffix at or past token `limit`.
    fn expr_type_until(&self, start: usize, limit: usize, env: &Env<'_>, depth: usize) -> (Ty, usize) {
        if depth > DEPTH {
            return (Ty::Unknown, start + 1);
        }
        let Some(token) = self.tokens.get(start) else {
            return (Ty::Unknown, start);
        };
        let mut at = start + 1;
        let mut ty = match token.kind {
            TokenKind::Number => Ty::Number,
            TokenKind::String => Ty::String,
            TokenKind::Keyword if self.is(start, "true") || self.is(start, "false") => Ty::Boolean,
            TokenKind::Keyword if self.is(start, "function") => Ty::Function,
            TokenKind::Name => self.name_type(start, env, depth),
            TokenKind::Symbol if self.is(start, "(") => {
                let (inner, after) = self.expr_type_at(start + 1, env, depth + 1);
                at = if self.is(after, ")") { after + 1 } else { after };
                inner
            }
            _ => Ty::Unknown,
        };
        loop {
            if at >= limit {
                break;
            }
            if (self.is(at, ".") || self.is(at, ":")) && self.is_name(at + 1) {
                let method = self.is(at, ":");
                let name = self.text(at + 1).to_owned();
                at += 2;
                if method {
                    let (literal, after) = self.call_args(at);
                    ty = self.method_result(&ty, &name, literal.as_deref(), env, depth);
                    at = after;
                } else {
                    ty = self.member_type(&ty, &name, env, depth);
                }
            } else if self.is(at, "(") || (self.tokens.get(at).is_some_and(|t| t.kind == TokenKind::String)) {
                let (literal, after) = self.call_args(at);
                ty = self.call_result(&ty, literal.as_deref(), start, at, env, depth);
                at = after;
            } else if self.is(at, "[") {
                at = self.close(at);
                ty = Ty::Unknown;
            } else {
                break;
            }
        }
        (ty, at)
    }

    /// The token after the bracket opening at `open` closes.
    fn close(&self, open: usize) -> usize {
        let mut depth = 0i32;
        let mut at = open;
        while at < self.tokens.len() {
            match self.text(at) {
                "(" | "{" | "[" => depth += 1,
                ")" | "}" | "]" => {
                    depth -= 1;
                    if depth == 0 {
                        return at + 1;
                    }
                }
                _ => {}
            }
            at += 1;
        }
        at
    }

    /// A call's arguments from `at` (`(...)` or a string): the first argument when it is a string literal, unquoted,
    /// and the token after them.
    fn call_args(&self, at: usize) -> (Option<String>, usize) {
        if self.tokens.get(at).is_some_and(|token| token.kind == TokenKind::String) {
            return (unquote(self.text(at)), at + 1);
        }
        if !self.is(at, "(") {
            return (None, at);
        }
        let literal = self
            .tokens
            .get(at + 1)
            .filter(|token| token.kind == TokenKind::String)
            .and_then(|_| unquote(self.text(at + 1)));
        (literal, self.close(at))
    }

    /// The type of the name at token `index`.
    fn name_type(&self, index: usize, env: &Env<'_>, depth: usize) -> Ty {
        let name = self.text(index);
        let decl = match self.refs.get(&index) {
            Some(Ref::Decl(decl)) => Some(*decl),
            Some(Ref::Global) => None,
            None => self.resolve(name, self.tokens[index].start),
        };
        if let Some(decl) = decl {
            return self.decl_type_at(decl, env, depth + 1);
        }
        global_type(name, env)
    }

    /// The type of `ty`'s member `name`.
    pub fn member_type(&self, ty: &Ty, name: &str, env: &Env<'_>, depth: usize) -> Ty {
        let api = env.api();
        match ty {
            Ty::Live(id) => {
                let class = env.class(*id).unwrap_or_default();
                if let Some(member) = api.member(&class, name) {
                    // `Parent`, an ObjectValue's `Value`...: the instance itself where the game says.
                    if name == "Parent"
                        && let Ok(Some(parent)) = env.dm.parent(*id)
                    {
                        return Ty::Live(parent);
                    }
                    return member.result().map_or(Ty::Unknown, |ty| named(&ty));
                }
                env.child(*id, name).map_or(Ty::Unknown, Ty::Live)
            }
            Ty::Named(class) => api
                .member(class, name)
                .and_then(|member| member.result())
                .map_or(Ty::Unknown, |ty| named(&ty)),
            Ty::Library(global) => api
                .globals
                .get(global)
                .and_then(|global| global.fields.iter().find(|field| field.name == name))
                .map_or(Ty::Unknown, |field| {
                    if field.is_function() {
                        Ty::Function
                    } else {
                        field.result().map_or(Ty::Unknown, |ty| named(&ty))
                    }
                }),
            Ty::Table(owner) => self
                .field(*owner, name)
                .map_or(Ty::Unknown, |field| self.decl_type_at(field, env, depth + 1)),
            Ty::ModuleTable(module, owner) => {
                let Some(other) = (env.module)(*module) else {
                    return Ty::Unknown;
                };
                let inner = Env {
                    dm: env.dm,
                    script: Some(*module),
                    module: env.module,
                };
                match other.field(*owner, name) {
                    Some(field) => match other.decl_type_at(field, &inner, depth + 1) {
                        // A table of the module's own is the module's.
                        Ty::Table(owner) => Ty::ModuleTable(*module, owner),
                        ty => ty,
                    },
                    None => Ty::Unknown,
                }
            }
            _ => Ty::Unknown,
        }
    }

    /// What `ty:name(literal, ...)` returns.
    fn method_result(&self, ty: &Ty, name: &str, literal: Option<&str>, env: &Env<'_>, _depth: usize) -> Ty {
        let api = env.api();
        let class = match ty {
            Ty::Live(id) => env.class(*id),
            Ty::Named(class) => Some(class.clone()),
            _ => None,
        };
        let Some(class) = class else {
            return Ty::Unknown;
        };
        // What the game holds: its service, its child.
        if let Ty::Live(id) = ty {
            match name {
                "GetService" => {
                    if let Some(service) = literal.and_then(|class| env.service(class)) {
                        return Ty::Live(service);
                    }
                }
                "FindFirstChild" | "WaitForChild" => {
                    if let Some(child) = literal.and_then(|child| env.child(*id, child)) {
                        return Ty::Live(child);
                    }
                }
                "Clone" => return Ty::Named(class),
                _ => {}
            }
        }
        let overloads = api.overloads(&class, name);
        let chosen = literal
            .and_then(|literal| {
                overloads
                    .iter()
                    .find(|member| member.ty.contains(&format!("\"{literal}\"")))
            })
            .or(overloads.last());
        chosen
            .and_then(|member| member.result())
            .map_or(Ty::Unknown, |ty| named(&ty))
    }

    /// What calling `ty` with `literal` first returns: `Instance.new("Part")`, `Vector3.new()`, `require(module)`.
    fn call_result(
        &self,
        ty: &Ty,
        literal: Option<&str>,
        start: usize,
        open: usize,
        env: &Env<'_>,
        depth: usize,
    ) -> Ty {
        // `require(x)`: what x's code returns.
        if self.text(start) == "require" && open == start + 1 && self.refs.get(&start) == Some(&Ref::Global) {
            let (arg, _) = self.expr_type_at(open + 1, env, depth + 1);
            if let Ty::Live(module) = arg {
                return module_type(module, env, depth);
            }
            return Ty::Unknown;
        }
        // `Library.field(...)`: the field's result, by the literal.
        if open >= 2 && self.is(open - 2, ".") {
            let field = self.text(open - 1);
            if let Some(library) = self.library_of(start, open - 2, env, depth)
                && let Some(member) = env
                    .api()
                    .globals
                    .get(&library)
                    .and_then(|global| global.fields.iter().find(|candidate| candidate.name == field))
            {
                return member.result_for(literal).map_or(Ty::Unknown, |ty| named(&ty));
            }
        }
        let _ = ty;
        Ty::Unknown
    }

    /// The library a call's table is, when the expression from `start` to the `.` at `dot` is a global library.
    fn library_of(&self, start: usize, dot: usize, env: &Env<'_>, depth: usize) -> Option<String> {
        if dot != start + 1 || !self.is_name(start) {
            return None;
        }
        match self.name_type(start, env, depth + 1) {
            Ty::Library(name) => Some(name),
            _ => None,
        }
    }

    /// The warnings of the code: unknown globals, unused locals, and members a known class does not have.
    pub fn warnings(&self, env: &Env<'_>) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let api = env.api();
        for (&index, reference) in &self.refs {
            if *reference != Ref::Global || self.type_tokens.contains(&index) {
                continue;
            }
            let name = self.text(index);
            if !api.globals.contains_key(name) && !super::highlight::BUILTINS.contains(&name) {
                let token = self.tokens[index];
                out.push(Diagnostic {
                    range: token.start..token.end,
                    severity: Severity::Warning,
                    message: format!("Unknown global '{name}'"),
                });
            }
        }
        let used: HashSet<usize> = self
            .uses
            .iter()
            .filter_map(|index| match self.refs.get(index) {
                Some(Ref::Decl(decl)) => Some(*decl),
                _ => None,
            })
            .collect();
        for (index, decl) in self.decls.iter().enumerate() {
            let unused = matches!(decl.kind, DeclKind::Local | DeclKind::LocalFunction)
                && !used.contains(&index)
                && !decl.name.starts_with('_');
            if unused {
                let what = if decl.function { "Function" } else { "Variable" };
                out.push(Diagnostic {
                    range: decl.range.clone(),
                    severity: Severity::Warning,
                    message: format!(
                        "{what} '{}' is never used; begin its name with _ if it is meant to be",
                        decl.name
                    ),
                });
            }
        }
        // `x.y` where x's class is known and has no y (nor, for an instance of the game, a child y).
        for index in 0..self.tokens.len() {
            if !(self.is(index, ".") || self.is(index, ":")) || !self.is_name(index + 1) || index == 0 {
                continue;
            }
            let Some(start) = self.chain_start(index) else {
                continue;
            };
            let ty = self.type_before(start, index, env);
            let name = self.text(index + 1);
            let message = match &ty {
                Ty::Live(id) => {
                    let class = env.class(*id).unwrap_or_default();
                    let known = api.member(&class, name).is_some() || env.child(*id, name).is_some();
                    (!known && api.types.contains_key(&class)).then(|| {
                        let named = env.dm.name(*id).unwrap_or_default();
                        format!("'{name}' is not a member of {class} '{named}', nor one of its children")
                    })
                }
                Ty::Named(class) if api.types.contains_key(class) && !api.is_a(class, "Instance") => {
                    (api.member(class, name).is_none()).then(|| format!("'{name}' is not a member of {class}"))
                }
                Ty::Named(class) if api.types.contains_key(class) => {
                    (api.member(class, name).is_none()).then(|| format!("'{name}' is not a member of {class}"))
                }
                _ => None,
            };
            if let Some(message) = message {
                let token = self.tokens[index + 1];
                out.push(Diagnostic {
                    range: token.start..token.end,
                    severity: Severity::Warning,
                    message,
                });
            }
        }
        out.sort_by_key(|diagnostic| diagnostic.range.start);
        out
    }

    /// The first token of the expression whose member access is at `dot` (a `.` or `:`): back over names, `.x`, `:x(...)`,
    /// calls and indexing. `None` when it does not start with a name.
    pub fn chain_start(&self, dot: usize) -> Option<usize> {
        let mut at = dot;
        loop {
            let before = at.checked_sub(1)?;
            if self.is(before, ")") || self.is(before, "]") {
                // Back to the bracket that opens it.
                let mut depth = 0i32;
                let mut back = before;
                loop {
                    match self.text(back) {
                        ")" | "]" | "}" => depth += 1,
                        "(" | "[" | "{" => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    back = back.checked_sub(1)?;
                }
                at = back;
                continue;
            }
            if self
                .tokens
                .get(before)
                .is_some_and(|token| token.kind == TokenKind::String)
                && before > 0
                && self.is_name(before - 1)
            {
                // `f"x"`.
                at = before;
                continue;
            }
            if self.is_name(before) {
                if before > 0 && (self.is(before - 1, ".") || self.is(before - 1, ":")) {
                    at = before - 1;
                    continue;
                }
                return Some(before);
            }
            return None;
        }
    }

    /// The type of the expression from token `start` up to (not including) token `end`.
    pub fn type_before(&self, start: usize, end: usize, env: &Env<'_>) -> Ty {
        // Read the expression as far as `end`: its suffixes past `end` are not part of it.
        let (ty, after) = self.expr_type_until(start, end, env, 0);
        if after >= end { ty } else { Ty::Unknown }
    }

    /// The type of what the file returns: a ModuleScript's value.
    pub fn returned_type(&self, env: &Env<'_>) -> Ty {
        self.returned.map_or(Ty::Unknown, |at| self.expr_type(at, env).0)
    }
}

/// What `require` of the ModuleScript `module` returns.
fn module_type(module: Id, env: &Env<'_>, depth: usize) -> Ty {
    if depth > DEPTH {
        return Ty::Unknown;
    }
    let Some(code) = (env.module)(module) else {
        return Ty::Unknown;
    };
    let inner = Env {
        dm: env.dm,
        script: Some(module),
        module: env.module,
    };
    match code.returned.map(|at| code.expr_type_at(at, &inner, depth + 1).0) {
        Some(Ty::Table(owner)) => Ty::ModuleTable(module, owner),
        Some(ty) => ty,
        None => Ty::Unknown,
    }
}

/// The type of the global `name`: the game's own instances for `game`, `workspace` and `script`, the API's otherwise.
pub fn global_type(name: &str, env: &Env<'_>) -> Ty {
    match name {
        "game" => return env.dm.root().map_or(Ty::Named("DataModel".to_owned()), Ty::Live),
        "workspace" => {
            return env
                .service("Workspace")
                .map_or(Ty::Named("Workspace".to_owned()), Ty::Live);
        }
        "script" => return env.script.map_or(Ty::Named("SourceContainer".to_owned()), Ty::Live),
        _ => {}
    }
    let Some(global) = env.api().globals.get(name) else {
        return Ty::Unknown;
    };
    if global.function {
        Ty::Function
    } else if !global.fields.is_empty() {
        Ty::Library(name.to_owned())
    } else {
        super::api::simple_type(&global.ty).map_or(Ty::Unknown, |ty| named(&ty))
    }
}

/// A type of the API by name, or the plain types.
fn named(ty: &str) -> Ty {
    match ty {
        "number" => Ty::Number,
        "string" => Ty::String,
        "boolean" => Ty::Boolean,
        _ => Ty::Named(ty.to_owned()),
    }
}

/// A string literal's text, without its quotes (not a long or interpolated one).
fn unquote(text: &str) -> Option<String> {
    let inner = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| text.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')))?;
    Some(inner.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_modules(_: Id) -> Option<Rc<Analysis>> {
        None
    }

    fn game() -> DataModel {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let scene = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d/Scenes/Main.scene"),
        )
        .unwrap();
        metallic_engine::metallic_project::load_scene_bytes(&mut dm, &scene).unwrap();
        dm
    }

    /// The declaration the name at the `n`th match of `needle` refers to, by its name and line.
    fn target(analysis: &Analysis, needle: &str, n: usize) -> Option<(String, usize)> {
        let at = analysis.source.match_indices(needle).nth(n)?.0;
        let token = analysis.token_at(at)?;
        match analysis.refs.get(&token)? {
            Ref::Decl(decl) => {
                let decl = &analysis.decls[*decl];
                Some((
                    decl.name.clone(),
                    analysis.source[..decl.range.start].matches('\n').count() + 1,
                ))
            }
            Ref::Global => Some(("<global>".to_owned(), 0)),
        }
    }

    #[test]
    fn names_refer_to_the_innermost_declaration_in_scope() {
        let source = "\
local x = 1
local function f(a, b: number): string
\tlocal x = a
\tfor i, v in pairs(b) do print(i, v, x) end
\treturn x
end
print(x, f, undefinedThing)
";
        let analysis = Analysis::new(source);
        assert_eq!(target(&analysis, "a\n", 0), Some(("a".into(), 2)));
        // The inner x in the loop, the outer at the end.
        assert_eq!(target(&analysis, "x)", 0), Some(("x".into(), 3)));
        assert_eq!(target(&analysis, "x, f", 0), Some(("x".into(), 1)));
        assert_eq!(target(&analysis, "f, undefined", 0), Some(("f".into(), 2)));
        assert_eq!(target(&analysis, "undefinedThing", 0), Some(("<global>".into(), 0)));
        // A type annotation names no value.
        let number = analysis.token_at(source.find("number").unwrap()).unwrap();
        assert!(analysis.type_tokens.contains(&number));
        assert!(!analysis.refs.contains_key(&number));
    }

    #[test]
    fn a_table_s_fields_are_declared_every_way() {
        let source = "\
local M = { speed = 4 }
M.name = \"door\"
function M.open(by) return by end
function M:close() return self.speed end
return M
";
        let analysis = Analysis::new(source);
        let m = analysis.resolve("M", source.len()).unwrap();
        let fields: Vec<&str> = analysis
            .fields(m)
            .iter()
            .map(|&f| analysis.decls[f].name.as_str())
            .collect();
        assert_eq!(fields, ["speed", "name", "open", "close"]);
        assert_eq!(analysis.returned.map(|at| analysis.text(at)), Some("M"));
        let dm = game();
        let env = Env {
            dm: &dm,
            script: None,
            module: &no_modules,
        };
        assert_eq!(analysis.returned_type(&env), Ty::Table(m));
        // `self` in a method is the table.
        let self_token = analysis.token_at(source.find("self").unwrap()).unwrap();
        assert_eq!(analysis.type_before(self_token, self_token + 1, &env), Ty::Table(m));
    }

    #[test]
    fn types_come_from_the_api_and_the_game() {
        let dm = game();
        let env = Env {
            dm: &dm,
            script: None,
            module: &no_modules,
        };
        let source = "\
local part = Instance.new(\"Part\")
local base = workspace.Baseplate
local lighting = game:GetService(\"Lighting\")
local size = base.Size
local v = Vector3.new(1, 2, 3)
local mag = v.Magnitude
";
        let analysis = Analysis::new(source);
        let ty = |name: &str| analysis.decl_type(analysis.resolve(name, source.len()).unwrap(), &env);
        assert_eq!(ty("part"), Ty::Named("Part".into()));
        let baseplate = env.child(
            dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap(),
            "Baseplate",
        );
        assert_eq!(ty("base"), Ty::Live(baseplate.unwrap()));
        assert!(matches!(ty("lighting"), Ty::Live(id) if env.class(id).as_deref() == Some("Lighting")));
        assert_eq!(ty("size"), Ty::Named("Vector3".into()));
        assert_eq!(ty("v"), Ty::Named("Vector3".into()));
        assert_eq!(ty("mag"), Ty::Number);
    }

    #[test]
    fn warnings_are_unknown_globals_unused_locals_and_unknown_members() {
        let dm = game();
        let env = Env {
            dm: &dm,
            script: None,
            module: &no_modules,
        };
        let source = "\
local used = 1
local unused = 2
local _ignored = 3
print(used, math.pi, typo)
workspace.Baseplate.Anchored = true
workspace.Baseplate.Colr = nil
workspace.Nothing:Destroy()
local p = Instance.new(\"Part\")
p:Destroyy()
local s = `{used}`
print(s, game.Workspace)
";
        let messages: Vec<String> = Analysis::new(source)
            .warnings(&env)
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect();
        assert_eq!(
            messages,
            [
                "Variable 'unused' is never used; begin its name with _ if it is meant to be",
                "Unknown global 'typo'",
                "'Colr' is not a member of Part 'Baseplate', nor one of its children",
                "'Nothing' is not a member of Workspace 'Workspace', nor one of its children",
                "'Destroyy' is not a member of Part",
            ]
        );
    }

    #[test]
    fn a_module_s_members_come_through_require() {
        let mut dm = game();
        let registry = dm.registry().clone();
        let storage = dm.service(registry.class_id("ReplicatedStorage").unwrap()).unwrap();
        let module = dm.create(registry.class_id("ModuleScript").unwrap()).unwrap();
        dm.set_name(module, "Doors".into()).unwrap();
        dm.set_parent(module, Some(storage)).unwrap();
        let code = Rc::new(Analysis::new(
            "local Doors = {}\nfunction Doors.open() end\nDoors.count = 1\nreturn Doors\n",
        ));
        let modules = move |id: Id| (id == module).then(|| Rc::clone(&code));
        let env = Env {
            dm: &dm,
            script: None,
            module: &modules,
        };
        let source = "local Doors = require(game:GetService(\"ReplicatedStorage\").Doors)\nlocal n = Doors.count\n";
        let analysis = Analysis::new(source);
        let doors = analysis.decl_type(analysis.resolve("Doors", source.len()).unwrap(), &env);
        assert!(matches!(doors, Ty::ModuleTable(id, _) if id == module), "{doors:?}");
        let n = analysis.decl_type(analysis.resolve("n", source.len()).unwrap(), &env);
        assert_eq!(n, Ty::Number);
    }
}
