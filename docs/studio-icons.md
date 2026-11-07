# Studio icons

<!-- Generated from apps/studio/src/theme/icons/mod.rs by `cargo test -p metallic_studio write_the_icon_docs -- --ignored`. Do not edit by hand. -->

Every icon in Metallic Studio has a stable id. By default it is a [Lucide](https://lucide.dev/icons/) SVG (ISC licence) in Studio's icons folder, drawn in its group's colour.

**The icons folder** is `icons` in Studio's own folder: `%APPDATA%\Metallic\Studio\icons` on Windows. Studio ships its icons beside its executable and copies any file missing there on start, so your edits are kept. `icons.json` in it says which SVG file each icon id uses, with a line on what it marks. To change an icon for good, put an SVG in the folder and point its entry's `file` at it. A file that is missing or cannot be read falls back to the default, with one warning in Output.

**Custom icons:** set a folder in Settings → Appearance → Icon folder. A file in it named after an icon's id, `<icon-id>.svg` or `<icon-id>.png` (the SVG wins when both are there), replaces that icon. Icons without a file keep the built-in one, and a file that cannot be read keeps it too, with one warning in Output. Studio reads the folder in the background when it is set and when you press **Reload**. A monochrome image (one colour on transparency, such as Lucide's own SVGs) is drawn in its group's colour; a coloured one is drawn as it is.

## Groups

| Group | Colour |
|---|---|
| Explorer: the workspace | green |
| Explorer: storage services | orange |
| Explorer: input and settings services | cyan |
| Explorer and Assets: server scripts | green |
| Explorer: local scripts | blue |
| Explorer: module scripts | purple |
| Explorer: bindable events and functions | pink |
| Explorer: lighting and lights | yellow |
| Explorer and Assets: parts, models, folders, cameras | white |
| Explorer: values | grey |
| Explorer and Assets: other | grey |
| Panels: scene | blue |
| Panels and Assets: content | pink |
| Panels: log | purple |
| Running the game | green |
| Studio | grey |
| Interface | its place's (neutral by default) |

## Icons

| Icon id | What it marks | Group | Default file | Custom files |
|---|---|---|---|---|
| `explorer.game` | The game (the data model). | Explorer and Assets: parts, models, folders, cameras | [`gamepad-2.svg`](https://lucide.dev/icons/gamepad-2) | `explorer.game.svg`, `explorer.game.png` |
| `explorer.workspace` | The workspace. | Explorer: the workspace | [`earth.svg`](https://lucide.dev/icons/earth) | `explorer.workspace.svg`, `explorer.workspace.png` |
| `explorer.lighting` | The lighting service. | Explorer: lighting and lights | [`sun.svg`](https://lucide.dev/icons/sun) | `explorer.lighting.svg`, `explorer.lighting.png` |
| `explorer.replicated_storage` | Replicated storage. | Explorer: storage services | [`package.svg`](https://lucide.dev/icons/package) | `explorer.replicated_storage.svg`, `explorer.replicated_storage.png` |
| `explorer.script_service` | The server script service. | Explorer and Assets: server scripts | [`scroll.svg`](https://lucide.dev/icons/scroll) | `explorer.script_service.svg`, `explorer.script_service.png` |
| `explorer.server_storage` | Server storage. | Explorer: storage services | [`hard-drive.svg`](https://lucide.dev/icons/hard-drive) | `explorer.server_storage.svg`, `explorer.server_storage.png` |
| `explorer.settings_service` | A settings or input service (user settings, input, run service). | Explorer: input and settings services | [`sliders-horizontal.svg`](https://lucide.dev/icons/sliders-horizontal) | `explorer.settings_service.svg`, `explorer.settings_service.png` |
| `explorer.camera` | A camera. | Explorer and Assets: parts, models, folders, cameras | [`camera.svg`](https://lucide.dev/icons/camera) | `explorer.camera.svg`, `explorer.camera.png` |
| `explorer.part` | A part. | Explorer and Assets: parts, models, folders, cameras | [`box.svg`](https://lucide.dev/icons/box) | `explorer.part.svg`, `explorer.part.png` |
| `explorer.mesh_part` | A mesh part. | Explorer and Assets: parts, models, folders, cameras | [`cuboid.svg`](https://lucide.dev/icons/cuboid) | `explorer.mesh_part.svg`, `explorer.mesh_part.png` |
| `explorer.model` | A model. | Explorer and Assets: parts, models, folders, cameras | [`shapes.svg`](https://lucide.dev/icons/shapes) | `explorer.model.svg`, `explorer.model.png` |
| `explorer.directional_light` | A directional light. | Explorer: lighting and lights | [`sun.svg`](https://lucide.dev/icons/sun) | `explorer.directional_light.svg`, `explorer.directional_light.png` |
| `explorer.spot_light` | A spotlight. | Explorer: lighting and lights | [`spotlight.svg`](https://lucide.dev/icons/spotlight) | `explorer.spot_light.svg`, `explorer.spot_light.png` |
| `explorer.light` | Any other light. | Explorer: lighting and lights | [`lightbulb.svg`](https://lucide.dev/icons/lightbulb) | `explorer.light.svg`, `explorer.light.png` |
| `explorer.script` | A script. | Explorer and Assets: server scripts | [`scroll.svg`](https://lucide.dev/icons/scroll) | `explorer.script.svg`, `explorer.script.png` |
| `explorer.local_script` | A local script, which runs on the player's machine: a script's scroll (#289), in its own colour. | Explorer: local scripts | [`scroll.svg`](https://lucide.dev/icons/scroll) | `explorer.local_script.svg`, `explorer.local_script.png` |
| `explorer.module_script` | A module script. | Explorer: module scripts | [`scroll-text.svg`](https://lucide.dev/icons/scroll-text) | `explorer.module_script.svg`, `explorer.module_script.png` |
| `explorer.event` | A bindable event. | Explorer: bindable events and functions | [`radio.svg`](https://lucide.dev/icons/radio) | `explorer.event.svg`, `explorer.event.png` |
| `explorer.function` | A bindable function. | Explorer: bindable events and functions | [`square-function.svg`](https://lucide.dev/icons/square-function) | `explorer.function.svg`, `explorer.function.png` |
| `explorer.folder` | A folder. | Explorer and Assets: parts, models, folders, cameras | [`folder.svg`](https://lucide.dev/icons/folder) | `explorer.folder.svg`, `explorer.folder.png` |
| `explorer.value` | A value object. | Explorer: values | [`diamond.svg`](https://lucide.dev/icons/diamond) | `explorer.value.svg`, `explorer.value.png` |
| `explorer.bool_value` | A true-or-false value. | Explorer: values | [`toggle-left.svg`](https://lucide.dev/icons/toggle-left) | `explorer.bool_value.svg`, `explorer.bool_value.png` |
| `explorer.int_value` | A whole-number value. | Explorer: values | [`hash.svg`](https://lucide.dev/icons/hash) | `explorer.int_value.svg`, `explorer.int_value.png` |
| `explorer.number_value` | A number value. | Explorer: values | [`binary.svg`](https://lucide.dev/icons/binary) | `explorer.number_value.svg`, `explorer.number_value.png` |
| `explorer.string_value` | A text value. | Explorer: values | [`type.svg`](https://lucide.dev/icons/type) | `explorer.string_value.svg`, `explorer.string_value.png` |
| `explorer.object_value` | A value that refers to another object. | Explorer: values | [`link.svg`](https://lucide.dev/icons/link) | `explorer.object_value.svg`, `explorer.object_value.png` |
| `explorer.input` | An input object. | Explorer and Assets: other | [`mouse-pointer.svg`](https://lucide.dev/icons/mouse-pointer) | `explorer.input.svg`, `explorer.input.png` |
| `explorer.instance` | Any other instance. | Explorer and Assets: other | [`circle.svg`](https://lucide.dev/icons/circle) | `explorer.instance.svg`, `explorer.instance.png` |
| `panel.viewport` | The Viewport panel. | Panels: scene | [`monitor.svg`](https://lucide.dev/icons/monitor) | `panel.viewport.svg`, `panel.viewport.png` |
| `panel.explorer` | The Explorer panel. | Panels: scene | [`list-tree.svg`](https://lucide.dev/icons/list-tree) | `panel.explorer.svg`, `panel.explorer.png` |
| `panel.properties` | The Properties panel. | Panels: scene | [`list.svg`](https://lucide.dev/icons/list) | `panel.properties.svg`, `panel.properties.png` |
| `panel.assets` | The Assets panel. | Panels and Assets: content | [`files.svg`](https://lucide.dev/icons/files) | `panel.assets.svg`, `panel.assets.png` |
| `panel.output` | The Output panel. | Panels: log | [`square-terminal.svg`](https://lucide.dev/icons/square-terminal) | `panel.output.svg`, `panel.output.png` |
| `panel.game` | The Game tab. | Running the game | [`gamepad-2.svg`](https://lucide.dev/icons/gamepad-2) | `panel.game.svg`, `panel.game.png` |
| `asset.mesh` | A mesh in the Assets panel. | Explorer and Assets: parts, models, folders, cameras | [`cuboid.svg`](https://lucide.dev/icons/cuboid) | `asset.mesh.svg`, `asset.mesh.png` |
| `asset.image` | An image in the Assets panel. | Panels and Assets: content | [`image.svg`](https://lucide.dev/icons/image) | `asset.image.svg`, `asset.image.png` |
| `asset.audio` | A sound in the Assets panel. | Panels and Assets: content | [`music.svg`](https://lucide.dev/icons/music) | `asset.audio.svg`, `asset.audio.png` |
| `asset.scene` | A scene in the Assets panel. | Explorer and Assets: parts, models, folders, cameras | [`list-tree.svg`](https://lucide.dev/icons/list-tree) | `asset.scene.svg`, `asset.scene.png` |
| `asset.script` | A script in the Assets panel. | Explorer and Assets: server scripts | [`scroll.svg`](https://lucide.dev/icons/scroll) | `asset.script.svg`, `asset.script.png` |
| `asset.file` | Any other file in the Assets panel. | Explorer and Assets: other | [`file.svg`](https://lucide.dev/icons/file) | `asset.file.svg`, `asset.file.png` |
| `toolbar.play` | Play, and Resume while paused. | Running the game | [`play.svg`](https://lucide.dev/icons/play) | `toolbar.play.svg`, `toolbar.play.png` |
| `toolbar.pause` | Pause. | Running the game | [`pause.svg`](https://lucide.dev/icons/pause) | `toolbar.pause.svg`, `toolbar.pause.png` |
| `toolbar.step` | Step one frame. | Running the game | [`skip-forward.svg`](https://lucide.dev/icons/skip-forward) | `toolbar.step.svg`, `toolbar.step.png` |
| `toolbar.stop` | Stop. | Running the game | [`square.svg`](https://lucide.dev/icons/square) | `toolbar.stop.svg`, `toolbar.stop.png` |
| `toolbar.settings` | The gear that opens Settings. | Studio | [`settings.svg`](https://lucide.dev/icons/settings) | `toolbar.settings.svg`, `toolbar.settings.png` |
| `ui.search` | A search box. | Interface | [`search.svg`](https://lucide.dev/icons/search) | `ui.search.svg`, `ui.search.png` |
| `ui.close` | Close, dismiss. | Interface | [`x.svg`](https://lucide.dev/icons/x) | `ui.close.svg`, `ui.close.png` |
| `ui.warning` | A warning. | Interface | [`triangle-alert.svg`](https://lucide.dev/icons/triangle-alert) | `ui.warning.svg`, `ui.warning.png` |
| `ui.error` | An error. | Interface | [`circle-x.svg`](https://lucide.dev/icons/circle-x) | `ui.error.svg`, `ui.error.png` |
| `ui.info` | Information. | Interface | [`info.svg`](https://lucide.dev/icons/info) | `ui.info.svg`, `ui.info.png` |
| `ui.check` | A check mark. | Interface | [`check.svg`](https://lucide.dev/icons/check) | `ui.check.svg`, `ui.check.png` |
| `ui.expand` | A collapsed tree item or section. | Interface | [`chevron-right.svg`](https://lucide.dev/icons/chevron-right) | `ui.expand.svg`, `ui.expand.png` |
| `ui.collapse` | An expanded tree item or section. | Interface | [`chevron-down.svg`](https://lucide.dev/icons/chevron-down) | `ui.collapse.svg`, `ui.collapse.png` |
| `ui.palette` | A colour of your own. | Interface | [`palette.svg`](https://lucide.dev/icons/palette) | `ui.palette.svg`, `ui.palette.png` |
| `ui.add` | Add another. | Interface | [`plus.svg`](https://lucide.dev/icons/plus) | `ui.add.svg`, `ui.add.png` |
