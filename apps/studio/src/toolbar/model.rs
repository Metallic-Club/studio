//! What the toolbar holds, as saved in `toolbar.json` (#232): its tabs, each with its elements and, when the tab
//! loads a layout of its own, that layout; and which tab is in front.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file the toolbar is saved in, beside the settings.
pub const TOOLBAR_FILE: &str = "toolbar.json";
/// The file's format version: 2 put the transform tools first in the first tab (#374).
pub const TOOLBAR_VERSION: u32 = 2;

/// One element of a tab's toolbar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Element {
    /// A tool's button, by the tool's id ([`super::Tool::id`]). A tool no longer registered (a plugin removed) shows
    /// greyed out until it is back or removed.
    Tool {
        /// The tool's id.
        id: String,
    },
    /// Empty space.
    Spacer,
    /// A thin line between groups.
    Separator,
}

impl Element {
    /// A tool's button.
    pub fn tool(id: &str) -> Self {
        Element::Tool { id: id.to_owned() }
    }
}

/// Select, Move, Scale and Rotate (#374), the increments (#375), then a separator: what the Home tab starts with, at
/// the far left.
fn transform_tools() -> Vec<Element> {
    use super::tools::ids;
    vec![
        Element::tool(ids::SELECT),
        Element::tool(ids::MOVE),
        Element::tool(ids::SCALE),
        Element::tool(ids::ROTATE),
        Element::tool(ids::SNAP),
        Element::Separator,
    ]
}

/// Where the tabs sit in their row (#329).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabAlign {
    /// From the left, the default.
    #[default]
    Left,
    /// In the middle.
    Center,
    /// Against the right.
    Right,
}

impl TabAlign {
    /// Every place, in the order the Tabs window lists them.
    pub const ALL: [TabAlign; 3] = [TabAlign::Left, TabAlign::Center, TabAlign::Right];

    /// Its name in the Tabs window.
    pub fn label(self) -> &'static str {
        match self {
            TabAlign::Left => "Left",
            TabAlign::Center => "Center",
            TabAlign::Right => "Right",
        }
    }
}

/// Serde's default for [`TabConfig::shown`]: a tab from before #329 is shown.
fn shown() -> bool {
    true
}

/// One tab.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabConfig {
    /// Its id, which stays with it when tabs are moved: the layout shown belongs to a tab by its id.
    #[serde(default)]
    pub id: u32,
    /// Its name, on the tab.
    pub name: String,
    /// Whether it is in the row: a hidden tab keeps its tools and layout until it is shown again (#329).
    #[serde(default = "shown")]
    pub shown: bool,
    /// Its toolbar.
    #[serde(default)]
    pub elements: Vec<Element>,
    /// Whether choosing the tab loads a dock layout of its own (and leaving it brings the main one back).
    #[serde(default)]
    pub own_layout: bool,
    /// That layout, as `layout.json` writes one; `None` until the tab is first shown, which starts it from the main.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
}

impl TabConfig {
    /// A tab called `name` holding `elements`.
    pub fn new(name: &str, elements: Vec<Element>) -> Self {
        Self {
            id: 0,
            name: name.to_owned(),
            shown: true,
            elements,
            own_layout: false,
            layout: None,
        }
    }
}

/// The whole toolbar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolbarConfig {
    /// The format version.
    pub version: u32,
    /// The tabs, left to right.
    pub tabs: Vec<TabConfig>,
    /// The tab in front, by index.
    #[serde(default)]
    pub active: usize,
    /// Where the tabs sit in their row.
    #[serde(default)]
    pub align: TabAlign,
    /// The tool row's height in points, as dragged (#414): [`super::TOOL_ROW`] by default, within
    /// [`super::TOOL_ROW_RANGE`].
    #[serde(default = "default_tool_row")]
    pub tool_row: u16,
}

/// Serde's default for [`ToolbarConfig::tool_row`].
fn default_tool_row() -> u16 {
    super::TOOL_ROW as u16
}

impl Default for ToolbarConfig {
    /// Home, Model, UI (with a layout of its own), Scripts and Plugins. Home holds the transform tools at its left
    /// (#374) and Studio's tools, Model the transform tools (#355); the others start empty, for the user (and later
    /// plugins) to fill.
    fn default() -> Self {
        let home = transform_tools().into_iter().chain([
            Element::tool(super::tools::ids::SAVE),
            Element::Separator,
            Element::tool(super::tools::ids::PLAY),
            Element::tool(super::tools::ids::PAUSE),
            Element::tool(super::tools::ids::STEP),
            Element::tool(super::tools::ids::STOP),
            Element::Separator,
            Element::tool("studio.panel.explorer"),
            Element::tool("studio.panel.properties"),
            Element::tool("studio.panel.output"),
            Element::Spacer,
            Element::tool(super::tools::ids::SETTINGS),
        ]);
        let mut ui = TabConfig::new("UI", Vec::new());
        ui.own_layout = true;
        Self {
            version: TOOLBAR_VERSION,
            tabs: vec![
                TabConfig::new("Home", home.collect()),
                TabConfig::new(
                    "Model",
                    vec![
                        Element::tool(super::tools::ids::SELECT),
                        Element::tool(super::tools::ids::MOVE),
                        Element::tool(super::tools::ids::SCALE),
                        Element::tool(super::tools::ids::ROTATE),
                        Element::Separator,
                        Element::tool(super::tools::ids::LOCAL_SPACE),
                        Element::tool(super::tools::ids::SNAP),
                    ],
                ),
                ui,
                TabConfig::new("Scripts", Vec::new()),
                TabConfig::new("Plugins", Vec::new()),
            ],
            active: 0,
            align: TabAlign::default(),
            tool_row: default_tool_row(),
        }
        .repaired()
    }
}

impl ToolbarConfig {
    /// The toolbar saved at `path`, repaired; the default when there is none, or when it cannot be read (logged).
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<ToolbarConfig>(&text) {
                Ok(config) => config.repaired(),
                Err(error) => {
                    log::warn!("toolbar: {}: {error}; using the default toolbar", path.display());
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!(
                    "toolbar: could not read {}: {error}; using the default toolbar",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// The toolbar as `toolbar.json` holds it.
    pub fn to_json(&self) -> String {
        // Strings, numbers and lists only: it cannot fail.
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }

    /// At least one tab, each with an id of its own, and the active one in range; a file from before version 2
    /// gets the transform tools at the far left of its first tab, unless that tab already holds one of them.
    pub fn repaired(mut self) -> Self {
        if self.tabs.is_empty() {
            self.tabs.push(TabConfig::new("Home", Vec::new()));
        }
        if self.version < 2 {
            let tools = transform_tools();
            let first = &mut self.tabs[0].elements;
            let holds = |element: &Element| matches!(element, Element::Tool { .. }) && first.contains(element);
            if !tools.iter().any(holds) {
                first.splice(0..0, tools);
            }
        }
        self.version = TOOLBAR_VERSION;
        let mut seen = std::collections::HashSet::new();
        for index in 0..self.tabs.len() {
            if self.tabs[index].id == 0 || !seen.insert(self.tabs[index].id) {
                let id = self.next_id();
                self.tabs[index].id = id;
                seen.insert(id);
            }
        }
        self.active = self.active.min(self.tabs.len() - 1);
        self.tool_row = self.tool_row.clamp(super::TOOL_ROW_RANGE.0, super::TOOL_ROW_RANGE.1);
        self.show_a_tab();
        self
    }

    /// Keeps the row from being empty and a shown tab in front: shows the first tab when every one is hidden, and
    /// brings the nearest shown tab forward when the front one is hidden (#329).
    pub fn show_a_tab(&mut self) {
        if !self.tabs.iter().any(|tab| tab.shown)
            && let Some(first) = self.tabs.first_mut()
        {
            first.shown = true;
        }
        if self.tabs.get(self.active).is_some_and(|tab| !tab.shown) {
            let before = self.tabs[..self.active].iter().rposition(|tab| tab.shown);
            let after = self.tabs[self.active..]
                .iter()
                .position(|tab| tab.shown)
                .map(|at| at + self.active);
            self.active = after.or(before).unwrap_or(0);
        }
    }

    /// An id no tab has.
    fn next_id(&self) -> u32 {
        self.tabs.iter().map(|tab| tab.id).max().unwrap_or(0) + 1
    }

    /// The tab in front.
    pub fn active_tab(&self) -> &TabConfig {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }

    /// The tab with id `id`.
    pub fn tab(&self, id: u32) -> Option<&TabConfig> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    /// The tab with id `id`, to change.
    pub fn tab_mut(&mut self, id: u32) -> Option<&mut TabConfig> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// Adds an empty tab called `name` at the end. Returns its index.
    pub fn add_tab(&mut self, name: &str) -> usize {
        let mut tab = TabConfig::new(name, Vec::new());
        tab.id = self.next_id();
        self.tabs.push(tab);
        self.tabs.len() - 1
    }

    /// Deletes the tab at `index`, unless it is the last one. Returns whether it did.
    pub fn remove_tab(&mut self, index: usize) -> bool {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return false;
        }
        self.tabs.remove(index);
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        true
    }

    /// Moves the tab at `from` to `to`, keeping the same tab in front.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        let len = self.tabs.len();
        if from >= len || from == to {
            return;
        }
        let to = to.min(len - 1);
        let active = self.tabs[self.active].id;
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = self.tabs.iter().position(|tab| tab.id == active).unwrap_or(0);
    }

    /// Puts `element` at `at` in tab `tab`'s toolbar (at the end when `at` is past it).
    pub fn insert_element(&mut self, tab: usize, at: usize, element: Element) {
        if let Some(tab) = self.tabs.get_mut(tab) {
            let at = at.min(tab.elements.len());
            tab.elements.insert(at, element);
        }
    }

    /// Takes the element at `index` out of tab `tab`'s toolbar.
    pub fn remove_element(&mut self, tab: usize, index: usize) {
        if let Some(tab) = self.tabs.get_mut(tab)
            && index < tab.elements.len()
        {
            tab.elements.remove(index);
        }
    }

    /// Moves the element at `from` to `to` in tab `tab`'s toolbar.
    pub fn move_element(&mut self, tab: usize, from: usize, to: usize) {
        if let Some(tab) = self.tabs.get_mut(tab) {
            let len = tab.elements.len();
            if from >= len || from == to {
                return;
            }
            let element = tab.elements.remove(from);
            tab.elements.insert(to.min(len - 1), element);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_has_the_owner_s_tabs() {
        let config = ToolbarConfig::default();
        let names: Vec<&str> = config.tabs.iter().map(|tab| tab.name.as_str()).collect();
        assert_eq!(names, ["Home", "Model", "UI", "Scripts", "Plugins"]);
        // UI loads a layout of its own; the others change the toolbar only.
        let own: Vec<bool> = config.tabs.iter().map(|tab| tab.own_layout).collect();
        assert_eq!(own, [false, false, true, false, false]);
        assert_eq!(config.active, 0);
        // Every tab has an id of its own.
        let ids: std::collections::HashSet<u32> = config.tabs.iter().map(|tab| tab.id).collect();
        assert_eq!(ids.len(), 5);
        // Home starts with the transform tools (#374) and the increments (#375).
        assert_eq!(config.tabs[0].elements[..6], transform_tools()[..]);
    }

    #[test]
    fn tabs_and_elements_move_and_go_and_the_front_tab_follows() {
        // UI in front.
        let mut config = ToolbarConfig {
            active: 2,
            ..ToolbarConfig::default()
        };
        let ui = config.tabs[2].id;
        config.move_tab(2, 0);
        assert_eq!(config.tabs[0].name, "UI");
        assert_eq!(config.active_tab().id, ui, "the same tab in front");
        let added = config.add_tab("Terrain");
        assert_eq!(config.tabs[added].name, "Terrain");
        assert!(config.tabs[..added].iter().all(|tab| tab.id != config.tabs[added].id));
        assert!(config.remove_tab(1));
        assert_eq!(config.active_tab().id, ui);
        // The last tab cannot go.
        let mut one = ToolbarConfig {
            version: TOOLBAR_VERSION,
            tabs: vec![TabConfig::new("Only", Vec::new())],
            active: 0,
            align: TabAlign::Left,
            tool_row: 54,
        }
        .repaired();
        assert!(!one.remove_tab(0));

        config.insert_element(0, 0, Element::Spacer);
        config.insert_element(0, 99, Element::tool("studio.save"));
        assert_eq!(config.tabs[0].elements, [Element::Spacer, Element::tool("studio.save")]);
        config.move_element(0, 1, 0);
        assert_eq!(config.tabs[0].elements, [Element::tool("studio.save"), Element::Spacer]);
        config.remove_element(0, 1);
        assert_eq!(config.tabs[0].elements, [Element::tool("studio.save")]);
        one.insert_element(5, 0, Element::Separator); // no such tab: nothing
    }

    #[test]
    fn hidden_tabs_never_leave_the_row_empty_or_a_hidden_tab_in_front() {
        let mut config = ToolbarConfig {
            active: 1,
            ..ToolbarConfig::default()
        };
        // The front tab hidden: the next shown one comes forward.
        config.tabs[1].shown = false;
        config.show_a_tab();
        assert_eq!(config.active, 2);
        // Hidden at the end: the one before.
        config.active = 4;
        config.tabs[4].shown = false;
        config.show_a_tab();
        assert_eq!(config.active, 3);
        // Every tab hidden: the first is shown again, and in front.
        for tab in &mut config.tabs {
            tab.shown = false;
        }
        config.show_a_tab();
        assert!(config.tabs[0].shown);
        assert_eq!(config.active, 0);
        // Saved and read back with the place of the tabs.
        config.align = TabAlign::Right;
        let text = config.to_json();
        assert!(text.contains("\"align\": \"right\""), "{text}");
        assert!(text.contains("\"shown\": false"), "{text}");
        assert_eq!(serde_json::from_str::<ToolbarConfig>(&text).unwrap(), config);
    }

    #[test]
    fn it_round_trips_and_a_bad_file_gives_the_default() {
        let dir = std::env::temp_dir().join(format!("metallic-toolbar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(TOOLBAR_FILE);
        assert_eq!(ToolbarConfig::load(&path), ToolbarConfig::default(), "no file");
        let mut config = ToolbarConfig {
            active: 3,
            ..ToolbarConfig::default()
        };
        config.tabs[1].elements.push(Element::Separator);
        config.tabs[2].layout = Some("{ \"version\": 1 }".to_owned());
        std::fs::write(&path, config.to_json()).unwrap();
        assert_eq!(ToolbarConfig::load(&path), config);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(ToolbarConfig::load(&path), ToolbarConfig::default());
        // Out of range and without ids: repaired.
        std::fs::write(
            &path,
            r#"{ "version": 1, "tabs": [ { "name": "A" }, { "name": "B" } ], "active": 9 }"#,
        )
        .unwrap();
        let repaired = ToolbarConfig::load(&path);
        assert_eq!(repaired.active, 1);
        assert_ne!(repaired.tabs[0].id, repaired.tabs[1].id);
        // A file from before #329: every tab shown, the tabs on the left.
        assert!(repaired.tabs.iter().all(|tab| tab.shown));
        assert_eq!(repaired.align, TabAlign::Left);
        // From before version 2: the transform tools at the first tab's far left, once (#374).
        assert_eq!(repaired.tabs[0].elements, transform_tools());
        assert!(repaired.tabs[1].elements.is_empty());
        assert_eq!(repaired.version, TOOLBAR_VERSION);
        assert_eq!(repaired.clone().repaired(), repaired, "not again");
        let held = format!(
            r#"{{ "version": 1, "tabs": [ {{ "name": "A", "elements": [ {{ "kind": "tool", "id": "{}" }} ] }} ] }}"#,
            super::super::tools::ids::MOVE
        );
        std::fs::write(&path, held).unwrap();
        assert_eq!(
            ToolbarConfig::load(&path).tabs[0].elements.len(),
            1,
            "a tab that holds one already is left as it is"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
