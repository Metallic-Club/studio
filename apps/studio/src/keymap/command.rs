//! The command registry (ADR 0009, Keybinds): every action Studio can bind, with a stable id, its words, where it
//! applies, its default bindings and its repeat policy. Menus, tooltips and the keymap all read it.

use std::collections::HashMap;

use super::binding::{Binding, KeyKind, Trigger};

/// Where a command applies. Contexts nest: Global, then the projects page or the Editor, then the panel that has
/// focus. A command applies in its contexts and every context inside them, and the most specific binding wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Context {
    /// Anywhere in Studio.
    Global,
    /// The projects page, with no dialog over it.
    Projects,
    /// With a project open.
    Editor,
    /// The 3D viewport has focus (or, for navigation, the pointer is over it).
    Viewport,
    /// The Explorer panel has focus.
    Explorer,
    /// The Properties panel has focus.
    Properties,
    /// The Assets panel has focus.
    Assets,
    /// The Output panel has focus.
    Output,
}

impl Context {
    /// Every context, outermost first.
    pub const ALL: [Context; 8] = [
        Context::Global,
        Context::Projects,
        Context::Editor,
        Context::Viewport,
        Context::Explorer,
        Context::Properties,
        Context::Assets,
        Context::Output,
    ];

    /// The context this one is inside.
    pub fn parent(self) -> Option<Context> {
        match self {
            Context::Global => None,
            Context::Projects | Context::Editor => Some(Context::Global),
            Context::Viewport | Context::Explorer | Context::Properties | Context::Assets | Context::Output => {
                Some(Context::Editor)
            }
        }
    }

    /// How deep it is: Global is 0, Projects and Editor 1, a panel 2. Deeper is more specific.
    pub fn depth(self) -> u8 {
        self.parent().map_or(0, |parent| parent.depth() + 1)
    }

    /// Whether `other` is this context or inside it.
    pub fn contains(self, other: Context) -> bool {
        let mut at = Some(other);
        while let Some(context) = at {
            if context == self {
                return true;
            }
            at = context.parent();
        }
        false
    }

    /// Whether one of the two contains the other: both can be active at once, so their bindings can clash.
    pub fn overlaps(self, other: Context) -> bool {
        self.contains(other) || other.contains(self)
    }

    /// The name people see.
    pub fn label(self) -> &'static str {
        match self {
            Context::Global => "Global",
            Context::Projects => "Projects",
            Context::Editor => "Editor",
            Context::Viewport => "Viewport",
            Context::Explorer => "Explorer",
            Context::Properties => "Properties",
            Context::Assets => "Assets",
            Context::Output => "Output",
        }
    }
}

/// What a command does with key auto-repeat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Repeat {
    /// Fires once per press: holding Primary+D duplicates once.
    #[default]
    Ignore,
    /// Fires again on every auto-repeat: navigation and nudges.
    Allow,
}

/// A command: an action that can be bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    /// Its stable id, as stored in `settings.toml` (`file.save`). Never renamed once shipped.
    pub id: &'static str,
    /// Its name.
    pub title: &'static str,
    /// The group the Keybinds page lists it under.
    pub category: &'static str,
    /// What it does, in a line.
    pub description: &'static str,
    /// Where it applies.
    pub contexts: &'static [Context],
    /// Its default bindings, as binding text. "Primary" makes them right on every platform.
    pub defaults: &'static [&'static str],
    /// What it does with key auto-repeat.
    pub repeat: Repeat,
    /// Whether it still fires while a text field has focus (the field gets every other key).
    pub while_typing: bool,
    /// Whether its mouse and wheel bindings start when the pointer is over its panel without focus: viewport
    /// navigation only.
    pub on_hover: bool,
}

impl Command {
    /// A command with no bindings, applying everywhere, ignoring repeat; set the rest with struct update syntax.
    pub const fn new(id: &'static str, title: &'static str, category: &'static str, description: &'static str) -> Self {
        Self {
            id,
            title,
            category,
            description,
            contexts: &[Context::Global],
            defaults: &[],
            repeat: Repeat::Ignore,
            while_typing: false,
            on_hover: false,
        }
    }
}

/// The ids of Studio's commands.
pub mod ids {
    /// Opens or closes the Settings window.
    pub const SETTINGS_OPEN: &str = "settings.open";
    /// Saves the open project.
    pub const FILE_SAVE: &str = "file.save";
    /// Closes the open project.
    pub const PROJECT_CLOSE: &str = "project.close";
    /// Puts the editor's panels back where they start.
    pub const VIEW_RESET_LAYOUT: &str = "view.reset_layout";
    /// Starts a game session in the Game tab (Play in Studio).
    pub const TEST_PLAY: &str = "test.play";
    /// Pauses or resumes the game session.
    pub const TEST_PAUSE: &str = "test.pause";
    /// Runs one frame of the paused game.
    pub const TEST_STEP: &str = "test.step";
    /// Ends the game session.
    pub const TEST_STOP: &str = "test.stop";
    /// Opens the project selected on the projects page.
    pub const PROJECTS_OPEN: &str = "projects.open";
    /// Starts a new project from a template.
    pub const PROJECTS_NEW: &str = "projects.new";
    /// Adds an existing project file to the list.
    pub const PROJECTS_ADD: &str = "projects.add";
    /// Flies the editor camera forward while held.
    pub const FLY_FORWARD: &str = "viewport.fly_forward";
    /// Flies the editor camera back while held.
    pub const FLY_BACK: &str = "viewport.fly_back";
    /// Flies the editor camera left while held.
    pub const FLY_LEFT: &str = "viewport.fly_left";
    /// Flies the editor camera right while held.
    pub const FLY_RIGHT: &str = "viewport.fly_right";
    /// Flies the editor camera up while held.
    pub const FLY_UP: &str = "viewport.fly_up";
    /// Flies the editor camera down while held.
    pub const FLY_DOWN: &str = "viewport.fly_down";
    /// Turns the editor camera with the pointer while held.
    pub const LOOK: &str = "viewport.look";
    /// Orbits the editor camera around its focus while held.
    pub const ORBIT: &str = "viewport.orbit";
    /// Moves the editor camera in and out.
    pub const ZOOM: &str = "viewport.zoom";
    /// Changes how fast the editor camera flies.
    pub const SPEED: &str = "viewport.speed";
}

const NAVIGATION: &str = "Viewport navigation";

/// A fly command: a physical key, held, repeating, in the viewport.
const fn fly(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    key: &'static [&'static str],
) -> Command {
    Command {
        contexts: &[Context::Viewport],
        defaults: key,
        repeat: Repeat::Allow,
        ..Command::new(id, title, NAVIGATION, description)
    }
}

/// A pointer navigation command: starts over the viewport, focused or not.
const fn pointer(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    bind: &'static [&'static str],
) -> Command {
    Command {
        contexts: &[Context::Viewport],
        defaults: bind,
        on_hover: true,
        ..Command::new(id, title, NAVIGATION, description)
    }
}

/// Every Studio command, in the order the Keybinds page lists them. The viewport's are registered for #217.
pub const STUDIO_COMMANDS: &[Command] = &[
    Command {
        defaults: &["Primary+Comma"],
        // Closing Settings from its own search box is expected; a comma typed with Primary held is not text.
        while_typing: true,
        ..Command::new(
            ids::SETTINGS_OPEN,
            "Settings",
            "General",
            "Opens or closes the Settings window.",
        )
    },
    Command {
        contexts: &[Context::Editor],
        defaults: &["Primary+W"],
        ..Command::new(
            ids::PROJECT_CLOSE,
            "Close project",
            "Project",
            "Closes the project and goes back to your projects.",
        )
    },
    Command {
        contexts: &[Context::Editor],
        defaults: &["Primary+S"],
        ..Command::new(ids::FILE_SAVE, "Save", "Project", "Saves the project to its file.")
    },
    Command {
        contexts: &[Context::Editor],
        ..Command::new(
            ids::VIEW_RESET_LAYOUT,
            "Reset layout",
            "View",
            "Puts the editor's panels back where they were when Studio was new.",
        )
    },
    Command {
        contexts: &[Context::Editor],
        defaults: &["F5"],
        ..Command::new(
            ids::TEST_PLAY,
            "Play",
            "Test",
            "Runs the game in the Game tab, from the scene as it is now.",
        )
    },
    Command {
        contexts: &[Context::Editor],
        defaults: &["F7"],
        ..Command::new(
            ids::TEST_PAUSE,
            "Pause or resume",
            "Test",
            "Freezes the running game's time, or lets it run again.",
        )
    },
    Command {
        contexts: &[Context::Editor],
        defaults: &["F6"],
        ..Command::new(ids::TEST_STEP, "Step", "Test", "Runs one frame of the paused game.")
    },
    Command {
        contexts: &[Context::Editor],
        defaults: &["Shift+F5"],
        ..Command::new(
            ids::TEST_STOP,
            "Stop",
            "Test",
            "Ends the game and puts the scene back as it was before Play.",
        )
    },
    Command {
        contexts: &[Context::Projects],
        defaults: &["Enter"],
        ..Command::new(
            ids::PROJECTS_OPEN,
            "Open project",
            "Projects",
            "Opens the project selected on the projects page.",
        )
    },
    Command {
        contexts: &[Context::Projects],
        defaults: &["Primary+N"],
        ..Command::new(
            ids::PROJECTS_NEW,
            "New project",
            "Projects",
            "Starts a new project from a template.",
        )
    },
    Command {
        contexts: &[Context::Projects],
        defaults: &["Primary+O"],
        ..Command::new(
            ids::PROJECTS_ADD,
            "Add project",
            "Projects",
            "Adds a project file you already have to the list, and opens it.",
        )
    },
    fly(
        ids::FLY_FORWARD,
        "Fly forward",
        "Moves the camera forward while held.",
        &["[W]"],
    ),
    fly(ids::FLY_BACK, "Fly back", "Moves the camera back while held.", &["[S]"]),
    fly(ids::FLY_LEFT, "Fly left", "Moves the camera left while held.", &["[A]"]),
    fly(
        ids::FLY_RIGHT,
        "Fly right",
        "Moves the camera right while held.",
        &["[D]"],
    ),
    fly(ids::FLY_UP, "Fly up", "Moves the camera up while held.", &["[E]"]),
    fly(ids::FLY_DOWN, "Fly down", "Moves the camera down while held.", &["[Q]"]),
    pointer(
        ids::LOOK,
        "Look around",
        "Turns the camera with the mouse while the button is held.",
        &["DragRight"],
    ),
    pointer(
        ids::ORBIT,
        "Orbit",
        "Circles the camera around what it looks at while the button is held.",
        &["Alt+DragLeft"],
    ),
    pointer(
        ids::ZOOM,
        "Zoom",
        "Moves the camera in and out with the wheel.",
        &["Wheel"],
    ),
    pointer(
        ids::SPEED,
        "Fly speed",
        "Makes the camera fly faster or slower with the wheel.",
        &["Shift+Wheel"],
    ),
];

/// The registered commands, with their parsed defaults and an index by id. Built once.
#[derive(Clone, Debug)]
pub struct Registry {
    commands: Vec<Command>,
    defaults: Vec<Vec<Binding>>,
    by_id: HashMap<&'static str, usize>,
}

impl Registry {
    /// A registry of `commands`. A default that does not parse is a bug: it is logged and left out. A second command
    /// with an id already registered is logged and left out.
    pub fn new(commands: &[Command]) -> Self {
        let mut registry = Self {
            commands: Vec::with_capacity(commands.len()),
            defaults: Vec::with_capacity(commands.len()),
            by_id: HashMap::with_capacity(commands.len()),
        };
        for command in commands {
            if registry.by_id.contains_key(command.id) {
                log::error!("keymap: the command `{}` is registered twice", command.id);
                continue;
            }
            let defaults = command
                .defaults
                .iter()
                .filter_map(|text| {
                    Binding::parse(text)
                        .map_err(|error| log::error!("keymap: `{}`'s default: {error}", command.id))
                        .ok()
                })
                .collect();
            registry.by_id.insert(command.id, registry.commands.len());
            registry.commands.push(*command);
            registry.defaults.push(defaults);
        }
        registry
    }

    /// Studio's commands ([`STUDIO_COMMANDS`]).
    pub fn studio() -> Self {
        Self::new(STUDIO_COMMANDS)
    }

    /// Every command, in registration order.
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// The command at `index`.
    pub fn get(&self, index: usize) -> Option<&Command> {
        self.commands.get(index)
    }

    /// The index of the command `id`.
    pub fn index(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    /// The command `id`.
    pub fn command(&self, id: &str) -> Option<&Command> {
        self.index(id).map(|index| &self.commands[index])
    }

    /// The default bindings of the command at `index`.
    pub fn defaults(&self, index: usize) -> &[Binding] {
        self.defaults.get(index).map_or(&[], Vec::as_slice)
    }

    /// Whether the command at `index` binds keys by place ([`KeyKind::Physical`]), so capturing a key for it records
    /// the place: true when its defaults do.
    pub fn prefers_physical(&self, index: usize) -> bool {
        self.defaults(index).iter().any(|binding| {
            matches!(
                binding.first.trigger,
                Trigger::Key {
                    kind: KeyKind::Physical,
                    ..
                }
            )
        })
    }

    /// Whether the command at `index` is bound to drags by default, so capturing a mouse button for it records a drag.
    pub fn prefers_drag(&self, index: usize) -> bool {
        self.defaults(index)
            .iter()
            .any(|binding| matches!(binding.first.trigger, Trigger::Drag(_)))
    }
}

/// Whether any context of `a` overlaps any context of `b`.
pub fn contexts_overlap(a: &Command, b: &Command) -> bool {
    a.contexts.iter().any(|ca| b.contexts.iter().any(|cb| ca.overlaps(*cb)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexts_nest() {
        assert_eq!(Context::Global.depth(), 0);
        assert_eq!(Context::Editor.depth(), 1);
        assert_eq!(Context::Viewport.depth(), 2);
        assert!(Context::Global.contains(Context::Viewport));
        assert!(Context::Editor.contains(Context::Output));
        assert!(!Context::Viewport.contains(Context::Editor));
        assert!(Context::Viewport.overlaps(Context::Global));
        assert!(
            !Context::Viewport.overlaps(Context::Explorer),
            "sibling panels never clash"
        );
        assert_eq!(Context::Projects.depth(), 1);
        assert!(Context::Projects.overlaps(Context::Global));
        assert!(
            !Context::Projects.overlaps(Context::Viewport),
            "the projects page and the editor are never shown together"
        );
    }

    #[test]
    fn studios_commands_are_well_formed() {
        let registry = Registry::studio();
        assert_eq!(registry.commands().len(), STUDIO_COMMANDS.len(), "no id twice");
        for (index, command) in registry.commands().iter().enumerate() {
            assert_eq!(
                registry.defaults(index).len(),
                command.defaults.len(),
                "{}'s defaults parse",
                command.id
            );
            assert!(command.id.contains('.'), "{}", command.id);
            assert!(command.description.ends_with('.'), "{}", command.id);
            assert!(!command.contexts.is_empty(), "{}", command.id);
        }
        assert!(registry.prefers_physical(registry.index(ids::FLY_FORWARD).unwrap()));
        assert!(!registry.prefers_physical(registry.index(ids::SETTINGS_OPEN).unwrap()));
        assert!(registry.prefers_drag(registry.index(ids::LOOK).unwrap()));
        assert_eq!(registry.command(ids::ZOOM).map(|c| c.on_hover), Some(true));
    }

    #[test]
    fn a_duplicate_id_is_left_out() {
        let one = Command::new("a.b", "A", "X", "A.");
        let registry = Registry::new(&[one, Command { title: "B", ..one }]);
        assert_eq!(registry.commands().len(), 1);
        assert_eq!(registry.command("a.b").unwrap().title, "A");
    }
}
