# Studio icons

<!-- Generated from apps/studio/src/theme/icons/mod.rs by `cargo test -p metallic_studio write_the_icon_docs -- --ignored`. Do not edit by hand. -->

Every icon in Metallic Studio has a stable id. By default it is a [Lucide](https://lucide.dev/icons/) SVG (ISC licence) in Studio's icons folder, drawn in its group's colour.

**The icons folder** is `icons` in Studio's own folder: `%APPDATA%\Metallic\Studio\icons` on Windows. Studio ships its icons beside its executable and copies any file missing there on start, so your edits are kept. `icons.json` sits at the top; the SVGs are in a folder per part of Studio that draws them (`explorer`, `assets`, `panels`, `toolbar`, `interface`), and Lucide's licence is in `licenses`. `icons.json` says which SVG file each icon id uses and which colour group it is drawn in, with a line on what it marks, and under `colors` each group's colour on the dark and the light mode (`#rrggbb`). To change an icon for good, put an SVG in the folder and point its entry's `file` at it; to change a colour, edit its group's colours, or point an icon's `color` at another group. A file that is missing or cannot be read falls back to the default, with one warning in Output. Studio reads `icons.json` when it starts.

**Custom icons:** set a folder in Settings → Appearance → Icon folder. A file in it named after an icon's id, `<icon-id>.svg` or `<icon-id>.png` (the SVG wins when both are there), replaces that icon. Icons without a file keep the built-in one, and a file that cannot be read keeps it too, with one warning in Output. Studio reads the folder in the background when it is set and when you press **Reload**. A monochrome image (one colour on transparency, such as Lucide's own SVGs) is drawn in its group's colour; a coloured one is drawn as it is.

## Groups

| Group | Id in `icons.json` | Colour | Dark | Light |
|---|---|---|---|---|
| Explorer: the workspace | `workspace` | green | `#6bc98c` | `#2e7d4f` |
| Explorer: replicated storage | `storage` | orange | `#e0995a` | `#a65619` |
| Explorer: input and settings services | `settings_service` | cyan | `#5cbcc8` | `#1d6f7a` |
| Explorer and Assets: server scripts and server storage | `server_script` | green | `#6bc98c` | `#2e7d4f` |
| Explorer: local scripts | `local_script` | blue | `#6aa3e8` | `#2f5fa8` |
| Explorer: module scripts | `module_script` | purple | `#a88be0` | `#6545b0` |
| Explorer: bindable events and functions | `event` | pink | `#d987b0` | `#a33d6e` |
| Explorer: lighting and lights | `light` | yellow | `#dcc25a` | `#806512` |
| Explorer and Assets: parts, models, folders, cameras | `object` | white | `#dcdcdc` | `#3f3f3f` |
| Explorer: values | `value` | grey | `#adadad` | `#646464` |
| Explorer and Assets: other | `other` | grey | `#adadad` | `#646464` |
| Panels: scene | `scene` | blue | `#6aa3e8` | `#2f5fa8` |
| Panels and Assets: content | `content` | pink | `#d987b0` | `#a33d6e` |
| Panels: log | `log` | purple | `#a88be0` | `#6545b0` |
| Running the game | `run` | green | `#6bc98c` | `#2e7d4f` |
| Studio | `studio` | grey | `#adadad` | `#646464` |
| Interface | `interface` | its place's (neutral by default) |  |  |

## Icons

| Icon id | What it marks | Group | Default file | Custom files |
|---|---|---|---|---|
| `explorer.game` | The game (the data model). | Explorer and Assets: parts, models, folders, cameras | [`explorer/gamepad-2.svg`](https://lucide.dev/icons/gamepad-2) | `explorer.game.svg`, `explorer.game.png` |
| `explorer.workspace` | The workspace. | Explorer: the workspace | [`explorer/earth.svg`](https://lucide.dev/icons/earth) | `explorer.workspace.svg`, `explorer.workspace.png` |
| `explorer.lighting` | The lighting service. | Explorer: lighting and lights | [`explorer/sun.svg`](https://lucide.dev/icons/sun) | `explorer.lighting.svg`, `explorer.lighting.png` |
| `explorer.replicated_storage` | Replicated storage. | Explorer: replicated storage | [`explorer/package.svg`](https://lucide.dev/icons/package) | `explorer.replicated_storage.svg`, `explorer.replicated_storage.png` |
| `explorer.script_service` | The server script service. | Explorer and Assets: server scripts and server storage | [`explorer/scroll.svg`](https://lucide.dev/icons/scroll) | `explorer.script_service.svg`, `explorer.script_service.png` |
| `explorer.server_storage` | Server storage. | Explorer and Assets: server scripts and server storage | [`explorer/hard-drive.svg`](https://lucide.dev/icons/hard-drive) | `explorer.server_storage.svg`, `explorer.server_storage.png` |
| `explorer.settings_service` | A settings or input service (user settings, input, run service). | Explorer: input and settings services | [`explorer/sliders-horizontal.svg`](https://lucide.dev/icons/sliders-horizontal) | `explorer.settings_service.svg`, `explorer.settings_service.png` |
| `explorer.camera` | A camera. | Explorer and Assets: parts, models, folders, cameras | [`explorer/camera.svg`](https://lucide.dev/icons/camera) | `explorer.camera.svg`, `explorer.camera.png` |
| `explorer.part` | A part. | Explorer and Assets: parts, models, folders, cameras | [`explorer/box.svg`](https://lucide.dev/icons/box) | `explorer.part.svg`, `explorer.part.png` |
| `explorer.mesh_part` | A mesh part. | Explorer and Assets: parts, models, folders, cameras | [`explorer/cuboid.svg`](https://lucide.dev/icons/cuboid) | `explorer.mesh_part.svg`, `explorer.mesh_part.png` |
| `explorer.model` | A model. | Explorer and Assets: parts, models, folders, cameras | [`explorer/shapes.svg`](https://lucide.dev/icons/shapes) | `explorer.model.svg`, `explorer.model.png` |
| `explorer.directional_light` | A directional light. | Explorer: lighting and lights | [`explorer/sun.svg`](https://lucide.dev/icons/sun) | `explorer.directional_light.svg`, `explorer.directional_light.png` |
| `explorer.spot_light` | A spotlight. | Explorer: lighting and lights | [`explorer/spotlight.svg`](https://lucide.dev/icons/spotlight) | `explorer.spot_light.svg`, `explorer.spot_light.png` |
| `explorer.light` | Any other light. | Explorer: lighting and lights | [`explorer/lightbulb.svg`](https://lucide.dev/icons/lightbulb) | `explorer.light.svg`, `explorer.light.png` |
| `explorer.script` | A script. | Explorer and Assets: server scripts and server storage | [`explorer/scroll.svg`](https://lucide.dev/icons/scroll) | `explorer.script.svg`, `explorer.script.png` |
| `explorer.local_script` | A local script, which runs on the player's machine: a script's scroll (#289), in its own colour. | Explorer: local scripts | [`explorer/scroll.svg`](https://lucide.dev/icons/scroll) | `explorer.local_script.svg`, `explorer.local_script.png` |
| `explorer.module_script` | A module script. | Explorer: module scripts | [`explorer/scroll-text.svg`](https://lucide.dev/icons/scroll-text) | `explorer.module_script.svg`, `explorer.module_script.png` |
| `explorer.event` | A bindable event. | Explorer: bindable events and functions | [`explorer/radio.svg`](https://lucide.dev/icons/radio) | `explorer.event.svg`, `explorer.event.png` |
| `explorer.function` | A bindable function. | Explorer: bindable events and functions | [`explorer/square-function.svg`](https://lucide.dev/icons/square-function) | `explorer.function.svg`, `explorer.function.png` |
| `explorer.folder` | A folder. | Explorer and Assets: parts, models, folders, cameras | [`explorer/folder.svg`](https://lucide.dev/icons/folder) | `explorer.folder.svg`, `explorer.folder.png` |
| `explorer.value` | A value object. | Explorer: values | [`explorer/diamond.svg`](https://lucide.dev/icons/diamond) | `explorer.value.svg`, `explorer.value.png` |
| `explorer.bool_value` | A true-or-false value. | Explorer: values | [`explorer/toggle-left.svg`](https://lucide.dev/icons/toggle-left) | `explorer.bool_value.svg`, `explorer.bool_value.png` |
| `explorer.int_value` | A whole-number value. | Explorer: values | [`explorer/hash.svg`](https://lucide.dev/icons/hash) | `explorer.int_value.svg`, `explorer.int_value.png` |
| `explorer.number_value` | A number value. | Explorer: values | [`explorer/binary.svg`](https://lucide.dev/icons/binary) | `explorer.number_value.svg`, `explorer.number_value.png` |
| `explorer.string_value` | A text value. | Explorer: values | [`explorer/type.svg`](https://lucide.dev/icons/type) | `explorer.string_value.svg`, `explorer.string_value.png` |
| `explorer.object_value` | A value that refers to another object. | Explorer: values | [`explorer/link.svg`](https://lucide.dev/icons/link) | `explorer.object_value.svg`, `explorer.object_value.png` |
| `explorer.input` | An input object. | Explorer and Assets: other | [`explorer/mouse-pointer.svg`](https://lucide.dev/icons/mouse-pointer) | `explorer.input.svg`, `explorer.input.png` |
| `explorer.instance` | Any other instance. | Explorer and Assets: other | [`explorer/circle.svg`](https://lucide.dev/icons/circle) | `explorer.instance.svg`, `explorer.instance.png` |
| `panel.viewport` | The Viewport panel. | Panels: scene | [`panels/monitor.svg`](https://lucide.dev/icons/monitor) | `panel.viewport.svg`, `panel.viewport.png` |
| `panel.explorer` | The Explorer panel. | Panels: scene | [`panels/list-tree.svg`](https://lucide.dev/icons/list-tree) | `panel.explorer.svg`, `panel.explorer.png` |
| `panel.properties` | The Properties panel. | Panels: scene | [`panels/list.svg`](https://lucide.dev/icons/list) | `panel.properties.svg`, `panel.properties.png` |
| `panel.assets` | The Assets panel. | Panels and Assets: content | [`panels/files.svg`](https://lucide.dev/icons/files) | `panel.assets.svg`, `panel.assets.png` |
| `panel.output` | The Output panel. | Panels: log | [`panels/square-terminal.svg`](https://lucide.dev/icons/square-terminal) | `panel.output.svg`, `panel.output.png` |
| `panel.game` | The Game tab. | Running the game | [`panels/gamepad-2.svg`](https://lucide.dev/icons/gamepad-2) | `panel.game.svg`, `panel.game.png` |
| `asset.mesh` | A mesh in the Assets panel. | Explorer and Assets: parts, models, folders, cameras | [`assets/cuboid.svg`](https://lucide.dev/icons/cuboid) | `asset.mesh.svg`, `asset.mesh.png` |
| `asset.image` | An image in the Assets panel. | Panels and Assets: content | [`assets/image.svg`](https://lucide.dev/icons/image) | `asset.image.svg`, `asset.image.png` |
| `asset.audio` | A sound in the Assets panel. | Panels and Assets: content | [`assets/music.svg`](https://lucide.dev/icons/music) | `asset.audio.svg`, `asset.audio.png` |
| `asset.scene` | A scene in the Assets panel. | Explorer and Assets: parts, models, folders, cameras | [`assets/list-tree.svg`](https://lucide.dev/icons/list-tree) | `asset.scene.svg`, `asset.scene.png` |
| `asset.script` | A script in the Assets panel. | Explorer and Assets: server scripts and server storage | [`assets/scroll.svg`](https://lucide.dev/icons/scroll) | `asset.script.svg`, `asset.script.png` |
| `asset.file` | Any other file in the Assets panel. | Explorer and Assets: other | [`assets/file.svg`](https://lucide.dev/icons/file) | `asset.file.svg`, `asset.file.png` |
| `toolbar.play` | Play, and Resume while paused. | Running the game | [`toolbar/play.svg`](https://lucide.dev/icons/play) | `toolbar.play.svg`, `toolbar.play.png` |
| `toolbar.pause` | Pause. | Running the game | [`toolbar/pause.svg`](https://lucide.dev/icons/pause) | `toolbar.pause.svg`, `toolbar.pause.png` |
| `toolbar.step` | Step one frame. | Running the game | [`toolbar/skip-forward.svg`](https://lucide.dev/icons/skip-forward) | `toolbar.step.svg`, `toolbar.step.png` |
| `toolbar.stop` | Stop. | Running the game | [`toolbar/square.svg`](https://lucide.dev/icons/square) | `toolbar.stop.svg`, `toolbar.stop.png` |
| `toolbar.settings` | The gear that opens Settings. | Studio | [`toolbar/settings.svg`](https://lucide.dev/icons/settings) | `toolbar.settings.svg`, `toolbar.settings.png` |
| `toolbar.save` | The toolbar's Save tool. | Studio | [`toolbar/save.svg`](https://lucide.dev/icons/save) | `toolbar.save.svg`, `toolbar.save.png` |
| `toolbar.close` | The toolbar's Close project tool. | Studio | [`toolbar/folder-x.svg`](https://lucide.dev/icons/folder-x) | `toolbar.close.svg`, `toolbar.close.png` |
| `toolbar.undo` | The toolbar's Undo tool. | Studio | [`toolbar/undo-2.svg`](https://lucide.dev/icons/undo-2) | `toolbar.undo.svg`, `toolbar.undo.png` |
| `toolbar.redo` | The toolbar's Redo tool. | Studio | [`toolbar/redo-2.svg`](https://lucide.dev/icons/redo-2) | `toolbar.redo.svg`, `toolbar.redo.png` |
| `toolbar.reset_layout` | The toolbar's Reset layout tool. | Panels: scene | [`toolbar/rotate-ccw.svg`](https://lucide.dev/icons/rotate-ccw) | `toolbar.reset_layout.svg`, `toolbar.reset_layout.png` |
| `toolbar.plugin` | A plugin's tool that brings no icon of its own. | Studio | [`toolbar/puzzle.svg`](https://lucide.dev/icons/puzzle) | `toolbar.plugin.svg`, `toolbar.plugin.png` |
| `toolbar.spacer` | A spacer, in the toolbar's Add window. | Interface | [`toolbar/space.svg`](https://lucide.dev/icons/space) | `toolbar.spacer.svg`, `toolbar.spacer.png` |
| `toolbar.separator` | A separator, in the toolbar's Add window. | Interface | [`toolbar/separator-vertical.svg`](https://lucide.dev/icons/separator-vertical) | `toolbar.separator.svg`, `toolbar.separator.png` |
| `ui.search` | A search box. | Interface | [`interface/search.svg`](https://lucide.dev/icons/search) | `ui.search.svg`, `ui.search.png` |
| `ui.close` | Close, dismiss. | Interface | [`interface/x.svg`](https://lucide.dev/icons/x) | `ui.close.svg`, `ui.close.png` |
| `ui.warning` | A warning. | Interface | [`interface/triangle-alert.svg`](https://lucide.dev/icons/triangle-alert) | `ui.warning.svg`, `ui.warning.png` |
| `ui.error` | An error. | Interface | [`interface/circle-x.svg`](https://lucide.dev/icons/circle-x) | `ui.error.svg`, `ui.error.png` |
| `ui.info` | Information. | Interface | [`interface/info.svg`](https://lucide.dev/icons/info) | `ui.info.svg`, `ui.info.png` |
| `ui.check` | A check mark. | Interface | [`interface/check.svg`](https://lucide.dev/icons/check) | `ui.check.svg`, `ui.check.png` |
| `ui.expand` | A collapsed tree item or section. | Interface | [`interface/chevron-right.svg`](https://lucide.dev/icons/chevron-right) | `ui.expand.svg`, `ui.expand.png` |
| `ui.collapse` | An expanded tree item or section. | Interface | [`interface/chevron-down.svg`](https://lucide.dev/icons/chevron-down) | `ui.collapse.svg`, `ui.collapse.png` |
| `ui.palette` | A colour of your own. | Interface | [`interface/palette.svg`](https://lucide.dev/icons/palette) | `ui.palette.svg`, `ui.palette.png` |
| `ui.add` | Add another. | Interface | [`interface/plus.svg`](https://lucide.dev/icons/plus) | `ui.add.svg`, `ui.add.png` |
| `ui.move_up` | Move up in a list. | Interface | [`interface/chevron-up.svg`](https://lucide.dev/icons/chevron-up) | `ui.move_up.svg`, `ui.move_up.png` |
| `ui.move_down` | Move down in a list. | Interface | [`interface/chevron-down.svg`](https://lucide.dev/icons/chevron-down) | `ui.move_down.svg`, `ui.move_down.png` |
| `ui.delete` | Delete. | Interface | [`interface/trash-2.svg`](https://lucide.dev/icons/trash-2) | `ui.delete.svg`, `ui.delete.png` |
