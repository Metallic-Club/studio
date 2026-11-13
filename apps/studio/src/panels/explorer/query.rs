//! The Explorer's search (#577): words and filters, read as Roblox Studio's Explorer reads them.
//!
//! A query is terms between spaces (double quotes keep spaces inside one: `Name="Spawn Location"`). A term with an
//! operator after a name is a **filter**: `key=value`, `key!=value`, `key<value`, `key>value`, `key<=value` or
//! `key>=value`. Any other term is a **word**, which the instance's name or class must contain, in any letter case.
//! An instance matches when every word and every filter holds.
//!
//! - `ClassName=Part` (or `Class=`, `ClassType=`): its class is that one, exactly, in any letter case;
//! - `IsA=BasePart`: its class is that one or inherits from it;
//! - `Name=door`: its name contains `door`, in any letter case (a search, as Roblox's, not an exact name);
//! - any other key is a property, in any letter case: `Anchored=true`, `Transparency>0.5`, `Material=Plastic`.
//!   Numbers compare as numbers, booleans as `true` or `false`, enum items by name (`Enum.Material.Plastic` too), text
//!   by containing the value in any letter case, and anything else (vectors, colours, references) by the text the
//!   Properties panel shows, spaces aside (`Size=12,1,12`, `Parent=Workspace`). An instance without the property never
//!   matches.
//!
//! `<`, `>`, `<=` and `>=` compare numbers only. `!=` holds where `=` does not, on an instance with the key. A filter
//! with no value yet (`Anchored=`, as it is typed) only asks for its key: the instance has that property. A value that
//! does not fit the property (`Anchored=maybe`) matches nothing.

use std::cmp::Ordering;

use metallic_engine::metallic_core::{DataModel, Id, Value};

use crate::panels::properties::format_value;

/// The filters the Explorer's filter button offers (#577), each its label and its term.
pub const SUGGESTED: [(&str, &str); 6] = [
    ("Parts", "IsA=BasePart"),
    ("Models", "IsA=Model"),
    ("Scripts", "IsA=SourceContainer"),
    ("Lights", "IsA=Light"),
    ("Anchored", "Anchored=true"),
    ("Not anchored", "Anchored=false"),
];

/// How a filter compares the instance's value with its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `=` (or `==`).
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `>`.
    Gt,
    /// `<=`.
    Le,
    /// `>=`.
    Ge,
}

impl Op {
    /// The operators as typed, a longer one before the shorter it starts with, so `<=` is not read as `<`.
    const TYPED: [(&'static str, Op); 7] = [
        ("!=", Op::Ne),
        ("<=", Op::Le),
        (">=", Op::Ge),
        ("==", Op::Eq),
        ("=", Op::Eq),
        ("<", Op::Lt),
        (">", Op::Gt),
    ];

    /// Whether it holds for the instance's value standing `ordering` to the filter's.
    fn holds(self, ordering: Ordering) -> bool {
        match self {
            Op::Eq => ordering == Ordering::Equal,
            Op::Ne => ordering != Ordering::Equal,
            Op::Lt => ordering == Ordering::Less,
            Op::Gt => ordering == Ordering::Greater,
            Op::Le => ordering != Ordering::Greater,
            Op::Ge => ordering != Ordering::Less,
        }
    }

    /// Whether it holds when the values are `equal` or not; `None` for an ordering, which needs numbers.
    fn equality(self, equal: bool) -> Option<bool> {
        match self {
            Op::Eq => Some(equal),
            Op::Ne => Some(!equal),
            Op::Lt | Op::Gt | Op::Le | Op::Ge => None,
        }
    }
}

/// What a filter looks at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// The name of the class, exactly (`ClassName`, `Class`, `ClassType`).
    Class,
    /// The class or one it inherits from (`IsA`).
    IsA,
    /// The instance's name, which contains the value (`Name`).
    Name,
    /// A property, by its name (or an older one) in any letter case.
    Property(String),
}

impl Key {
    /// The key typed as `name`.
    fn named(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "classname" | "class" | "classtype" => Key::Class,
            "isa" => Key::IsA,
            "name" => Key::Name,
            _ => Key::Property(name.to_owned()),
        }
    }
}

/// A `key=value` term of a search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    /// What it looks at.
    pub key: Key,
    /// How it compares.
    pub op: Op,
    /// The value typed after the operator, without its quotes; empty while it is still being typed.
    pub value: String,
}

impl Filter {
    /// The filter `term` is, or `None` for a word: a key of letters, digits and `_`, then an operator.
    fn parse(term: &str) -> Option<Self> {
        let at = term.find(['=', '!', '<', '>'])?;
        let (key, rest) = term.split_at(at);
        if key.is_empty() || !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        let (op, value) = Op::TYPED
            .iter()
            .find_map(|&(typed, op)| rest.strip_prefix(typed).map(|value| (op, value)))?;
        Some(Self {
            key: Key::named(key),
            op,
            value: value.trim().to_owned(),
        })
    }

    /// Whether instance `id` passes it.
    pub fn matches(&self, dm: &DataModel, id: Id) -> bool {
        let want = self.value.as_str();
        match &self.key {
            Key::Class if want.is_empty() => true,
            Key::Class => {
                let class = dm.class_name(id).unwrap_or_default();
                self.op.equality(class.eq_ignore_ascii_case(want)).unwrap_or(false)
            }
            Key::IsA if want.is_empty() => true,
            Key::IsA => {
                let class = dm
                    .registry()
                    .classes()
                    .iter()
                    .find(|info| info.name().eq_ignore_ascii_case(want))
                    .map(|info| info.id());
                let is = class.is_some_and(|class| dm.is_a(id, class).unwrap_or(false));
                self.op.equality(is).unwrap_or(false)
            }
            Key::Name => {
                let name = dm.name(id).unwrap_or_default().to_lowercase();
                self.op.equality(name.contains(&want.to_lowercase())).unwrap_or(false)
            }
            Key::Property(name) => match property(dm, id, name) {
                None => false,
                Some(_) if want.is_empty() => true,
                Some(have) => compare(dm, &have, self.op, want).unwrap_or(false),
            },
        }
    }
}

/// The value of the property named `name` (or an older name of it), in any letter case, on `id`; `None` when it has
/// no such property.
fn property(dm: &DataModel, id: Id, name: &str) -> Option<Value> {
    let class = dm.class_of(id).ok()?;
    let registry = dm.registry();
    let found = registry.props(class).iter().find(|prop| {
        !prop.hidden
            && (prop.meta.name.eq_ignore_ascii_case(name)
                || prop.meta.aliases.iter().any(|alias| alias.eq_ignore_ascii_case(name)))
    })?;
    let prop = registry.prop_id(class, found.meta.name)?;
    dm.get(id, prop).ok()
}

/// Whether `have`, an instance's value, stands `op` to the typed `want`; `None` when `want` does not fit it.
fn compare(dm: &DataModel, have: &Value, op: Op, want: &str) -> Option<bool> {
    match have {
        Value::Int(n) => numbers(*n as f64, op, want),
        Value::Number(n) => numbers(*n, op, want),
        Value::Bool(on) => {
            let wanted = if want.eq_ignore_ascii_case("true") {
                true
            } else if want.eq_ignore_ascii_case("false") {
                false
            } else {
                return None;
            };
            op.equality(*on == wanted)
        }
        // `Enum.Material.Plastic` as well as `Plastic`: the item's name is the last part.
        Value::Enum(item) => op.equality(
            item.name()
                .eq_ignore_ascii_case(want.rsplit('.').next().unwrap_or(want)),
        ),
        Value::String(text) => op.equality(text.to_lowercase().contains(&want.to_lowercase())),
        other => op.equality(squashed(&format_value(dm, other)) == squashed(want)),
    }
}

/// Whether the number `have` stands `op` to the number typed in `want`; `None` when `want` is not one.
fn numbers(have: f64, op: Op, want: &str) -> Option<bool> {
    let want = want.parse::<f64>().ok().filter(|x| x.is_finite())?;
    // A value kept in single precision is a hair off what was typed (0.3 is 0.30000001).
    let close = (have - want).abs() <= 1e-6 * have.abs().max(want.abs()).max(1.0);
    let ordering = if close {
        Ordering::Equal
    } else {
        have.partial_cmp(&want)?
    };
    Some(op.holds(ordering))
}

/// `text` without its spaces, in lower case: how a vector or a colour typed compares with one shown.
fn squashed(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A search of the Explorer: its words and its filters, all of which an instance must pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// The words, in lower case, which the instance's name or class contains.
    pub words: Vec<String>,
    /// The `key=value` terms.
    pub filters: Vec<Filter>,
}

impl Query {
    /// The search typed as `text`.
    pub fn parse(text: &str) -> Self {
        let mut query = Self::default();
        for (term, quoted) in terms(text) {
            // A term in quotes from its start is a word, even with an operator in it.
            match (!quoted).then(|| Filter::parse(&term)).flatten() {
                Some(filter) => query.filters.push(filter),
                None => query.words.push(term.to_lowercase()),
            }
        }
        query
    }

    /// Whether it asks for nothing (a blank search): the Explorer shows its tree.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.filters.is_empty()
    }

    /// Whether it has a filter, so the note for no results says that nothing matches rather than that nothing is
    /// called that.
    pub fn has_filters(&self) -> bool {
        !self.filters.is_empty()
    }

    /// Whether instance `id` passes every word and filter.
    pub fn matches(&self, dm: &DataModel, id: Id) -> bool {
        if !self.words.is_empty() {
            let name = dm.name(id).unwrap_or_default();
            let class = dm.class_name(id).unwrap_or_default();
            let text = format!("{name} {class}").to_lowercase();
            if !self.words.iter().all(|word| text.contains(word.as_str())) {
                return false;
            }
        }
        self.filters.iter().all(|filter| filter.matches(dm, id))
    }
}

/// The terms of `text`, split at spaces outside double quotes, the quotes taken out; each with whether it started
/// with a quote. A quote left open runs to the end.
fn terms(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut term = String::new();
    let mut quoted = false;
    let mut inside = false;
    let mut started = false;
    for c in text.chars() {
        if c == '"' {
            if !started {
                quoted = true;
            }
            started = true;
            inside = !inside;
        } else if c.is_whitespace() && !inside {
            if !term.is_empty() {
                out.push((std::mem::take(&mut term), quoted));
            }
            quoted = false;
            started = false;
        } else {
            term.push(c);
            started = true;
        }
    }
    if !term.is_empty() {
        out.push((term, quoted));
    }
    out
}

/// `query` with `term` added at its end after a space (#577), as the filter button adds one; unchanged when it
/// already holds that term.
pub fn with_term(query: &str, term: &str) -> String {
    let query = query.trim_end();
    if query.split_whitespace().any(|have| have.eq_ignore_ascii_case(term)) {
        return query.to_owned();
    }
    if query.is_empty() {
        term.to_owned()
    } else {
        format!("{query} {term}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(key: Key, op: Op, value: &str) -> Filter {
        Filter {
            key,
            op,
            value: value.to_owned(),
        }
    }

    #[test]
    fn each_operator_is_read_the_longer_ones_first() {
        let property = |name: &str| Key::Property(name.to_owned());
        for (typed, op) in [
            ("Transparency=0.5", Op::Eq),
            ("Transparency==0.5", Op::Eq),
            ("Transparency!=0.5", Op::Ne),
            ("Transparency<0.5", Op::Lt),
            ("Transparency>0.5", Op::Gt),
            ("Transparency<=0.5", Op::Le),
            ("Transparency>=0.5", Op::Ge),
        ] {
            assert_eq!(
                Query::parse(typed).filters,
                [filter(property("Transparency"), op, "0.5")],
                "{typed}"
            );
        }
    }

    #[test]
    fn the_class_keys_have_aliases_in_any_letter_case() {
        for typed in [
            "ClassName=Part",
            "Class=Part",
            "ClassType=Part",
            "classname=Part",
            "CLASS=Part",
        ] {
            assert_eq!(
                Query::parse(typed).filters,
                [filter(Key::Class, Op::Eq, "Part")],
                "{typed}"
            );
        }
        assert_eq!(
            Query::parse("isa=BasePart").filters,
            [filter(Key::IsA, Op::Eq, "BasePart")]
        );
        assert_eq!(Query::parse("NAME=door").filters, [filter(Key::Name, Op::Eq, "door")]);
    }

    #[test]
    fn words_and_filters_mix_and_quotes_keep_spaces() {
        let query = Query::parse("  Spawn  IsA=BasePart  base ");
        assert_eq!(query.words, ["spawn", "base"]);
        assert_eq!(query.filters, [filter(Key::IsA, Op::Eq, "BasePart")]);
        assert!(query.has_filters());

        let query = Query::parse("Name=\"Spawn Location\" \"two words\"");
        assert_eq!(query.filters, [filter(Key::Name, Op::Eq, "Spawn Location")]);
        assert_eq!(query.words, ["two words"]);
        // A term quoted from its start is a word, operator and all; an open quote runs to the end.
        assert_eq!(Query::parse("\"a=b\"").words, ["a=b"]);
        assert_eq!(
            Query::parse("Name=\"open end").filters,
            [filter(Key::Name, Op::Eq, "open end")]
        );
    }

    #[test]
    fn terms_that_are_not_filters_are_words() {
        for typed in ["=Part", "<3", "!", "hello!", "a.b=c", "x!y", "!=Part"] {
            let query = Query::parse(typed);
            assert!(query.filters.is_empty(), "{typed}: {query:?}");
            assert_eq!(query.words, [typed.to_lowercase()], "{typed}");
        }
        assert!(Query::parse("").is_empty());
        assert!(Query::parse("   ").is_empty());
        assert!(Query::parse("\"\"").is_empty());
        assert!(!Query::parse("spawn").has_filters());
    }

    #[test]
    fn a_filter_without_a_value_has_an_empty_one() {
        assert_eq!(
            Query::parse("Anchored=").filters,
            [filter(Key::Property("Anchored".to_owned()), Op::Eq, "")]
        );
        assert_eq!(Query::parse("Class!=").filters, [filter(Key::Class, Op::Ne, "")]);
    }

    #[test]
    fn a_term_is_added_after_a_space_once() {
        assert_eq!(with_term("", "IsA=BasePart"), "IsA=BasePart");
        assert_eq!(with_term("door  ", "IsA=BasePart"), "door IsA=BasePart");
        assert_eq!(with_term("door isa=basepart", "IsA=BasePart"), "door isa=basepart");
    }

    /// The template's game (Workspace: Camera, Baseplate, SpawnLocation; Lighting; the storage services) with a model
    /// Door in the workspace holding a loose, half-clear plastic Panel with a light in it, a script in
    /// ServerScriptService and a StringValue in ReplicatedStorage.
    fn game() -> DataModel {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let scene = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d/Scenes/Main.scene"),
        )
        .unwrap();
        metallic_engine::metallic_project::load_scene_bytes(&mut dm, &scene).unwrap();
        let service = |dm: &DataModel, name: &str| dm.service(dm.registry().class_id(name).unwrap()).unwrap();
        let add = |dm: &mut DataModel, class: &str, parent: Id, name: &str| {
            let id = dm.create(dm.registry().class_id(class).unwrap()).unwrap();
            dm.set_name(id, name.into()).unwrap();
            dm.set_parent(id, Some(parent)).unwrap();
            id
        };
        let workspace = service(&dm, "Workspace");
        let door = add(&mut dm, "Model", workspace, "Door");
        let panel = add(&mut dm, "Part", door, "Panel");
        dm.set_by_name(panel, "Anchored", Value::Bool(false)).unwrap();
        dm.set_by_name(panel, "Transparency", Value::Number(0.5)).unwrap();
        let plastic = dm.registry().enum_item("Material", "Plastic").unwrap();
        dm.set_by_name(panel, "Material", Value::Enum(plastic)).unwrap();
        add(&mut dm, "PointLight", panel, "Lamp");
        let scripts = service(&dm, "ServerScriptService");
        add(&mut dm, "Script", scripts, "Opener");
        let storage = service(&dm, "ReplicatedStorage");
        let greeting = add(&mut dm, "StringValue", storage, "Greeting");
        dm.set_by_name(greeting, "Value", Value::String("Hello World".into()))
            .unwrap();
        dm
    }

    /// The names of the instances of `dm` that `typed` matches, in tree order.
    fn found(dm: &DataModel, typed: &str) -> Vec<String> {
        let query = Query::parse(typed);
        let mut out = Vec::new();
        let mut stack = vec![dm.root().unwrap()];
        while let Some(id) = stack.pop() {
            if query.matches(dm, id) {
                out.push(dm.name(id).unwrap().to_owned());
            }
            let children: Vec<Id> = dm.children(id).unwrap().collect();
            stack.extend(children.into_iter().rev());
        }
        out
    }

    #[test]
    fn class_name_is_exact_in_any_letter_case() {
        let dm = game();
        assert_eq!(found(&dm, "ClassName=Part"), ["Baseplate", "SpawnLocation", "Panel"]);
        assert_eq!(found(&dm, "class=part"), ["Baseplate", "SpawnLocation", "Panel"]);
        // Exact: a PointLight is not a Light, and no class is called "Par".
        assert!(found(&dm, "ClassName=Light").is_empty());
        assert!(found(&dm, "ClassName=Par").is_empty());
        let not_parts = found(&dm, "ClassName!=Part");
        assert!(not_parts.contains(&"Door".to_owned()) && !not_parts.contains(&"Panel".to_owned()));
        // An ordering asks for numbers.
        assert!(found(&dm, "ClassName>Part").is_empty());
    }

    #[test]
    fn is_a_takes_the_class_and_those_inheriting_from_it() {
        let dm = game();
        assert_eq!(found(&dm, "IsA=BasePart"), ["Baseplate", "SpawnLocation", "Panel"]);
        assert_eq!(found(&dm, "isa=light"), ["Lamp"]);
        assert_eq!(found(&dm, "IsA=Model"), ["Door"]);
        assert_eq!(found(&dm, "IsA=SourceContainer"), ["Opener"]);
        assert!(found(&dm, "IsA=Teleporter").is_empty(), "no such class");
        assert!(!found(&dm, "IsA!=BasePart").contains(&"Panel".to_owned()));
        assert!(found(&dm, "IsA!=Teleporter").contains(&"Panel".to_owned()));
    }

    #[test]
    fn name_contains_the_value_in_any_letter_case() {
        let dm = game();
        assert_eq!(found(&dm, "Name=door"), ["Door"]);
        assert_eq!(found(&dm, "Name=PLATE"), ["Baseplate"]);
        assert!(!found(&dm, "Name!=plate").contains(&"Baseplate".to_owned()));
    }

    #[test]
    fn booleans_compare_as_true_or_false() {
        let dm = game();
        assert_eq!(found(&dm, "Anchored=true"), ["Baseplate", "SpawnLocation"]);
        assert_eq!(found(&dm, "anchored=FALSE"), ["Panel"]);
        assert_eq!(found(&dm, "Anchored!=true"), ["Panel"]);
        assert!(found(&dm, "Anchored=maybe").is_empty(), "not a boolean");
        assert!(found(&dm, "Anchored!=maybe").is_empty());
        assert!(found(&dm, "Anchored<true").is_empty());
    }

    #[test]
    fn numbers_compare_as_numbers() {
        let dm = game();
        assert_eq!(found(&dm, "Transparency=0.5"), ["Panel"]);
        assert_eq!(found(&dm, "Transparency=.5"), ["Panel"]);
        assert_eq!(found(&dm, "Transparency>0.25"), ["Panel"]);
        assert_eq!(found(&dm, "Transparency>=0.5"), ["Panel"]);
        assert!(found(&dm, "Transparency>0.5").is_empty());
        assert_eq!(found(&dm, "Transparency<0.5"), ["Baseplate", "SpawnLocation"]);
        assert_eq!(found(&dm, "Transparency<=0.5"), ["Baseplate", "SpawnLocation", "Panel"]);
        assert_eq!(found(&dm, "Transparency!=0"), ["Panel"]);
        assert!(found(&dm, "Transparency>clear").is_empty(), "not a number");
        assert!(found(&dm, "Transparency>inf").is_empty());
    }

    #[test]
    fn enums_compare_by_item_name() {
        let dm = game();
        assert_eq!(found(&dm, "Material=Plastic"), ["Panel"]);
        assert_eq!(found(&dm, "material=smoothplastic"), ["SpawnLocation"]);
        assert_eq!(found(&dm, "Material=Enum.Material.Concrete"), ["Baseplate"]);
        assert_eq!(found(&dm, "Material!=Plastic"), ["Baseplate", "SpawnLocation"]);
    }

    #[test]
    fn text_contains_the_value_and_other_values_compare_as_shown() {
        let dm = game();
        assert_eq!(found(&dm, "Value=hello"), ["Greeting"]);
        assert!(found(&dm, "Value=goodbye").is_empty());
        // As the Properties panel shows them, spaces aside.
        assert_eq!(found(&dm, "Size=12,1,12"), ["SpawnLocation"]);
        assert_eq!(found(&dm, "Size=\"512, 16, 512\""), ["Baseplate"]);
        assert_eq!(found(&dm, "Parent=Door"), ["Panel"]);
    }

    #[test]
    fn an_instance_without_the_property_never_matches() {
        let dm = game();
        assert!(!found(&dm, "Anchored!=true").contains(&"Door".to_owned()));
        assert!(found(&dm, "Teleport=true").is_empty());
        assert!(found(&dm, "Teleport!=true").is_empty());
    }

    #[test]
    fn a_filter_without_a_value_asks_only_for_its_key() {
        let dm = game();
        assert_eq!(found(&dm, "Anchored="), ["Baseplate", "SpawnLocation", "Panel"]);
        assert_eq!(found(&dm, "Range>"), ["Lamp"]);
        assert_eq!(found(&dm, "IsA="), found(&dm, ""));
        assert_eq!(found(&dm, "ClassName!="), found(&dm, ""));
        assert_eq!(found(&dm, "Name="), found(&dm, ""));
    }

    #[test]
    fn every_word_and_filter_must_hold() {
        let dm = game();
        assert_eq!(found(&dm, "spawn IsA=BasePart"), ["SpawnLocation"]);
        assert_eq!(found(&dm, "part Anchored=true"), ["Baseplate", "SpawnLocation"]);
        assert_eq!(found(&dm, "IsA=BasePart Anchored=false Transparency>0"), ["Panel"]);
        assert!(found(&dm, "IsA=BasePart IsA=Light").is_empty());
        assert!(found(&dm, "teleporter Anchored=true").is_empty());
    }

    #[test]
    fn the_suggested_filters_are_filters_on_classes_that_exist() {
        let registry = metallic_engine::metallic_classes::shared_registry().unwrap();
        for (label, term) in SUGGESTED {
            let query = Query::parse(term);
            assert_eq!(query.filters.len(), 1, "{label}");
            assert!(query.words.is_empty(), "{label}");
            if let Key::IsA = query.filters[0].key {
                assert!(registry.class_id(&query.filters[0].value).is_some(), "{label}");
            }
        }
        let dm = game();
        assert_eq!(found(&dm, SUGGESTED[2].1), ["Opener"]);
    }
}
