//! The tools a toolbar can hold (#232): each a button with an id, a label, an icon and what it does.
//!
//! Studio registers its own at start ([`ToolRegistry::studio`]). **The plugin API registers its tools the same way**
//! ([`ToolRegistry::register`] with a [`ToolAction::Plugin`]): they then show in the toolbar's Add menu, under
//! Plugins, and a click hands the plugin its command. Until the plugin API exists, nothing registers one, and a
//! plugin's button is shown disabled.

use crate::dock::Panel;
use crate::theme::icons::Icon;

/// Studio's own tools' ids.
pub mod ids {
    /// Save the project.
    pub const SAVE: &str = "studio.save";
    /// Close the project.
    pub const CLOSE: &str = "studio.close";
    /// Play.
    pub const PLAY: &str = "studio.play";
    /// Pause or resume.
    pub const PAUSE: &str = "studio.pause";
    /// Step one frame.
    pub const STEP: &str = "studio.step";
    /// Stop.
    pub const STOP: &str = "studio.stop";
    /// Reset the dock's layout.
    pub const RESET_LAYOUT: &str = "studio.reset_layout";
    /// Open or close Settings.
    pub const SETTINGS: &str = "studio.settings";
    /// The prefix of a panel's toggle: `studio.panel.<panel id>`.
    pub const PANEL: &str = "studio.panel.";
}

/// Something Studio itself does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StudioCommand {
    /// Save the project.
    Save,
    /// Close the project.
    Close,
    /// Play.
    Play,
    /// Pause or resume.
    Pause,
    /// Step one frame.
    Step,
    /// Stop.
    Stop,
    /// Reset the dock's layout.
    ResetLayout,
    /// Open or close Settings.
    Settings,
    /// Open or close a panel, by its id.
    TogglePanel(&'static str),
}

/// What a tool does when its button is clicked.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ToolAction {
    /// One of Studio's own commands.
    Studio(StudioCommand),
    /// A plugin's command: the plugin API hands `command` to the plugin named `plugin`.
    Plugin {
        /// The plugin's name.
        plugin: String,
        /// Its command, as the plugin registered it.
        command: String,
    },
}

/// A tool: a button the toolbar can hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    /// Its id, which the saved toolbar refers to it by: `studio.<name>` for Studio's,
    /// `plugin.<plugin>.<name>` for a plugin's. Never renamed once shipped.
    pub id: String,
    /// The label under its icon.
    pub label: String,
    /// What it does, in a line (its tooltip).
    pub description: String,
    /// Its icon.
    pub icon: Icon,
    /// What a click does.
    pub action: ToolAction,
}

impl Tool {
    /// One of Studio's own tools.
    fn studio(id: &str, label: &str, description: &str, icon: Icon, command: StudioCommand) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            description: description.to_owned(),
            icon,
            action: ToolAction::Studio(command),
        }
    }

    /// A plugin's tool: `name` in plugin `plugin`, with id `plugin.<plugin>.<name>`. A plugin without an icon of its
    /// own gets the puzzle piece.
    pub fn plugin(plugin: &str, name: &str, label: &str, description: &str, icon: Option<Icon>) -> Self {
        Self {
            id: format!("plugin.{plugin}.{name}"),
            label: label.to_owned(),
            description: description.to_owned(),
            icon: icon.unwrap_or(Icon::ToolbarPlugin),
            action: ToolAction::Plugin {
                plugin: plugin.to_owned(),
                command: name.to_owned(),
            },
        }
    }

    /// Whether it is a plugin's.
    pub fn is_plugin(&self) -> bool {
        matches!(self.action, ToolAction::Plugin { .. })
    }
}

/// Why a tool could not be registered.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ToolError {
    /// Another tool has its id.
    #[error("a tool with the id `{0}` is already registered")]
    Taken(String),
    /// Its id is empty or has characters other than ASCII letters, digits, `.`, `_` and `-`.
    #[error("`{0}` is not a tool id: use ASCII letters, digits, `.`, `_` and `-`")]
    BadId(String),
}

/// Every tool the toolbar's Add menu offers.
#[derive(Clone, Debug, Default)]
pub struct ToolRegistry {
    tools: Vec<Tool>,
}

impl ToolRegistry {
    /// Studio's own tools: the project, the game's controls, the layout, Settings, and a toggle per panel in
    /// `panels` (but Studio's own transient ones).
    pub fn studio(panels: &[&dyn Panel]) -> Self {
        let mut tools = vec![
            Tool::studio(
                ids::SAVE,
                "Save",
                "Saves the project.",
                Icon::ToolbarSave,
                StudioCommand::Save,
            ),
            Tool::studio(
                ids::CLOSE,
                "Close",
                "Closes the project and goes back to your projects.",
                Icon::ToolbarClose,
                StudioCommand::Close,
            ),
            Tool::studio(
                ids::PLAY,
                "Play",
                "Runs the game.",
                Icon::ToolbarPlay,
                StudioCommand::Play,
            ),
            Tool::studio(
                ids::PAUSE,
                "Pause",
                "Pauses or resumes the running game.",
                Icon::ToolbarPause,
                StudioCommand::Pause,
            ),
            Tool::studio(
                ids::STEP,
                "Step",
                "Runs one frame of the paused game.",
                Icon::ToolbarStep,
                StudioCommand::Step,
            ),
            Tool::studio(
                ids::STOP,
                "Stop",
                "Ends the game and puts the scene back.",
                Icon::ToolbarStop,
                StudioCommand::Stop,
            ),
            Tool::studio(
                ids::RESET_LAYOUT,
                "Reset layout",
                "Puts the panels back where they were when Studio was new.",
                Icon::ToolbarResetLayout,
                StudioCommand::ResetLayout,
            ),
            Tool::studio(
                ids::SETTINGS,
                "Settings",
                "Opens or closes the Settings window.",
                Icon::ToolbarSettings,
                StudioCommand::Settings,
            ),
        ];
        for panel in panels.iter().filter(|panel| !panel.transient()) {
            tools.push(Tool::studio(
                &format!("{}{}", ids::PANEL, panel.id()),
                panel.title(),
                &format!("Opens or closes the {} panel.", panel.title()),
                panel.icon(),
                StudioCommand::TogglePanel(panel.id()),
            ));
        }
        Self { tools }
    }

    /// Adds `tool`, after the others: from then on the Add menu offers it. The plugin API registers plugins' tools
    /// with this.
    pub fn register(&mut self, tool: Tool) -> Result<(), ToolError> {
        let good = !tool.id.is_empty()
            && tool
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !good {
            return Err(ToolError::BadId(tool.id));
        }
        if self.get(&tool.id).is_some() {
            return Err(ToolError::Taken(tool.id));
        }
        self.tools.push(tool);
        Ok(())
    }

    /// Takes a tool out (a plugin unloaded). Toolbars that hold it show it greyed out. Returns whether it was there.
    pub fn unregister(&mut self, id: &str) -> bool {
        let before = self.tools.len();
        self.tools.retain(|tool| tool.id != id);
        self.tools.len() != before
    }

    /// The tool with id `id`.
    pub fn get(&self, id: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.id == id)
    }

    /// Every tool, Studio's first.
    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::PANELS;

    #[test]
    fn studio_registers_its_tools_and_one_per_panel() {
        let registry = ToolRegistry::studio(PANELS);
        for id in [ids::SAVE, ids::PLAY, ids::STOP, ids::SETTINGS, "studio.panel.explorer"] {
            assert!(registry.get(id).is_some(), "{id}");
        }
        // The Game tab is Studio's to open: no toggle.
        assert!(registry.get("studio.panel.game").is_none());
        assert!(registry.tools().iter().all(|tool| !tool.is_plugin()));
        // Every tool the default toolbar names is registered.
        for tab in super::super::ToolbarConfig::default().tabs {
            for element in tab.elements {
                if let super::super::Element::Tool { id } = element {
                    assert!(registry.get(&id).is_some(), "{id}");
                }
            }
        }
    }

    #[test]
    fn a_plugin_registers_tools_and_ids_are_checked() {
        let mut registry = ToolRegistry::studio(PANELS);
        let tool = Tool::plugin("terrain", "paint", "Paint", "Paints terrain.", None);
        assert_eq!(tool.id, "plugin.terrain.paint");
        assert_eq!(tool.icon, Icon::ToolbarPlugin);
        registry.register(tool.clone()).unwrap();
        assert!(registry.get("plugin.terrain.paint").is_some_and(Tool::is_plugin));
        assert_eq!(
            registry.register(tool),
            Err(ToolError::Taken("plugin.terrain.paint".into()))
        );
        let mut bad = Tool::plugin("x", "y", "Y", "", None);
        bad.id = "has space".into();
        assert!(matches!(registry.register(bad), Err(ToolError::BadId(_))));
        assert!(registry.unregister("plugin.terrain.paint"));
        assert!(!registry.unregister("plugin.terrain.paint"));
    }
}
