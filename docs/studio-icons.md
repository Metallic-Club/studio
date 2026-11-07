# Studio icons

<!-- Generated from apps/studio/src/theme/icons/mod.rs by `cargo test -p metallic_studio write_the_icon_docs -- --ignored`. Do not edit by hand. -->

Every icon in Metallic Studio has a stable id. By default it is drawn from [Lucide](https://lucide.dev/icons/)'s icon font (ISC licence), in its group's colour.

**Custom icons:** set a folder in Settings → Appearance → Icon folder. A file in it named after an icon's id, `<icon-id>.svg` or `<icon-id>.png` (the SVG wins when both are there), replaces that icon. Icons without a file keep the built-in one, and a file that cannot be read keeps it too, with one warning in Output. Studio reads the folder in the background when it is set and when you press **Reload**. A monochrome image (one colour on transparency, such as Lucide's own SVGs) is drawn in its group's colour; a coloured one is drawn as it is.

## Groups

| Group | Colour |
|---|---|
| Explorer: services | blue |
| Explorer and Assets: world, parts, models and scenes | mint |
| Explorer: lights | orange |
| Explorer and Assets: scripts | violet |
| Explorer: cameras | rose |
| Explorer and Assets: other | neutral |
| Panels: scene | blue |
| Panels and Assets: content | mint |
| Panels: log | violet |
| Running the game | mint |
| Studio | neutral |
| Interface | its place's (neutral by default) |

## Icons

| Icon id | Group | Lucide icon | Custom files |
|---|---|---|---|
| `explorer.game` | Explorer: services | [`gamepad-2`](https://lucide.dev/icons/gamepad-2) | `explorer.game.svg`, `explorer.game.png` |
| `explorer.workspace` | Explorer and Assets: world, parts, models and scenes | [`globe`](https://lucide.dev/icons/globe) | `explorer.workspace.svg`, `explorer.workspace.png` |
| `explorer.lighting` | Explorer: lights | [`sun`](https://lucide.dev/icons/sun) | `explorer.lighting.svg`, `explorer.lighting.png` |
| `explorer.replicated_storage` | Explorer: services | [`package`](https://lucide.dev/icons/package) | `explorer.replicated_storage.svg`, `explorer.replicated_storage.png` |
| `explorer.script_service` | Explorer: services | [`braces`](https://lucide.dev/icons/braces) | `explorer.script_service.svg`, `explorer.script_service.png` |
| `explorer.server_storage` | Explorer: services | [`hard-drive`](https://lucide.dev/icons/hard-drive) | `explorer.server_storage.svg`, `explorer.server_storage.png` |
| `explorer.settings_service` | Explorer: services | [`sliders-horizontal`](https://lucide.dev/icons/sliders-horizontal) | `explorer.settings_service.svg`, `explorer.settings_service.png` |
| `explorer.camera` | Explorer: cameras | [`video`](https://lucide.dev/icons/video) | `explorer.camera.svg`, `explorer.camera.png` |
| `explorer.part` | Explorer and Assets: world, parts, models and scenes | [`box`](https://lucide.dev/icons/box) | `explorer.part.svg`, `explorer.part.png` |
| `explorer.mesh_part` | Explorer and Assets: world, parts, models and scenes | [`cuboid`](https://lucide.dev/icons/cuboid) | `explorer.mesh_part.svg`, `explorer.mesh_part.png` |
| `explorer.model` | Explorer and Assets: world, parts, models and scenes | [`shapes`](https://lucide.dev/icons/shapes) | `explorer.model.svg`, `explorer.model.png` |
| `explorer.directional_light` | Explorer: lights | [`sun`](https://lucide.dev/icons/sun) | `explorer.directional_light.svg`, `explorer.directional_light.png` |
| `explorer.spot_light` | Explorer: lights | [`flashlight`](https://lucide.dev/icons/flashlight) | `explorer.spot_light.svg`, `explorer.spot_light.png` |
| `explorer.light` | Explorer: lights | [`lightbulb`](https://lucide.dev/icons/lightbulb) | `explorer.light.svg`, `explorer.light.png` |
| `explorer.script` | Explorer and Assets: scripts | [`file-code`](https://lucide.dev/icons/file-code) | `explorer.script.svg`, `explorer.script.png` |
| `explorer.module_script` | Explorer and Assets: scripts | [`braces`](https://lucide.dev/icons/braces) | `explorer.module_script.svg`, `explorer.module_script.png` |
| `explorer.event` | Explorer and Assets: scripts | [`radio`](https://lucide.dev/icons/radio) | `explorer.event.svg`, `explorer.event.png` |
| `explorer.folder` | Explorer and Assets: other | [`folder`](https://lucide.dev/icons/folder) | `explorer.folder.svg`, `explorer.folder.png` |
| `explorer.value` | Explorer and Assets: other | [`diamond`](https://lucide.dev/icons/diamond) | `explorer.value.svg`, `explorer.value.png` |
| `explorer.input` | Explorer and Assets: other | [`mouse-pointer`](https://lucide.dev/icons/mouse-pointer) | `explorer.input.svg`, `explorer.input.png` |
| `explorer.instance` | Explorer and Assets: other | [`circle`](https://lucide.dev/icons/circle) | `explorer.instance.svg`, `explorer.instance.png` |
| `panel.viewport` | Panels: scene | [`monitor`](https://lucide.dev/icons/monitor) | `panel.viewport.svg`, `panel.viewport.png` |
| `panel.explorer` | Panels: scene | [`list-tree`](https://lucide.dev/icons/list-tree) | `panel.explorer.svg`, `panel.explorer.png` |
| `panel.properties` | Panels: scene | [`list`](https://lucide.dev/icons/list) | `panel.properties.svg`, `panel.properties.png` |
| `panel.assets` | Panels and Assets: content | [`files`](https://lucide.dev/icons/files) | `panel.assets.svg`, `panel.assets.png` |
| `panel.output` | Panels: log | [`square-terminal`](https://lucide.dev/icons/square-terminal) | `panel.output.svg`, `panel.output.png` |
| `panel.game` | Running the game | [`gamepad-2`](https://lucide.dev/icons/gamepad-2) | `panel.game.svg`, `panel.game.png` |
| `asset.mesh` | Explorer and Assets: world, parts, models and scenes | [`cuboid`](https://lucide.dev/icons/cuboid) | `asset.mesh.svg`, `asset.mesh.png` |
| `asset.image` | Panels and Assets: content | [`image`](https://lucide.dev/icons/image) | `asset.image.svg`, `asset.image.png` |
| `asset.audio` | Panels and Assets: content | [`music`](https://lucide.dev/icons/music) | `asset.audio.svg`, `asset.audio.png` |
| `asset.scene` | Explorer and Assets: world, parts, models and scenes | [`list-tree`](https://lucide.dev/icons/list-tree) | `asset.scene.svg`, `asset.scene.png` |
| `asset.script` | Explorer and Assets: scripts | [`file-code`](https://lucide.dev/icons/file-code) | `asset.script.svg`, `asset.script.png` |
| `asset.file` | Explorer and Assets: other | [`file`](https://lucide.dev/icons/file) | `asset.file.svg`, `asset.file.png` |
| `toolbar.play` | Running the game | [`play`](https://lucide.dev/icons/play) | `toolbar.play.svg`, `toolbar.play.png` |
| `toolbar.pause` | Running the game | [`pause`](https://lucide.dev/icons/pause) | `toolbar.pause.svg`, `toolbar.pause.png` |
| `toolbar.step` | Running the game | [`skip-forward`](https://lucide.dev/icons/skip-forward) | `toolbar.step.svg`, `toolbar.step.png` |
| `toolbar.stop` | Running the game | [`square`](https://lucide.dev/icons/square) | `toolbar.stop.svg`, `toolbar.stop.png` |
| `toolbar.settings` | Studio | [`settings`](https://lucide.dev/icons/settings) | `toolbar.settings.svg`, `toolbar.settings.png` |
| `ui.search` | Interface | [`search`](https://lucide.dev/icons/search) | `ui.search.svg`, `ui.search.png` |
| `ui.close` | Interface | [`x`](https://lucide.dev/icons/x) | `ui.close.svg`, `ui.close.png` |
| `ui.warning` | Interface | [`triangle-alert`](https://lucide.dev/icons/triangle-alert) | `ui.warning.svg`, `ui.warning.png` |
| `ui.error` | Interface | [`circle-x`](https://lucide.dev/icons/circle-x) | `ui.error.svg`, `ui.error.png` |
| `ui.info` | Interface | [`info`](https://lucide.dev/icons/info) | `ui.info.svg`, `ui.info.png` |
| `ui.check` | Interface | [`check`](https://lucide.dev/icons/check) | `ui.check.svg`, `ui.check.png` |
| `ui.expand` | Interface | [`chevron-right`](https://lucide.dev/icons/chevron-right) | `ui.expand.svg`, `ui.expand.png` |
| `ui.collapse` | Interface | [`chevron-down`](https://lucide.dev/icons/chevron-down) | `ui.collapse.svg`, `ui.collapse.png` |
| `ui.palette` | Interface | [`palette`](https://lucide.dev/icons/palette) | `ui.palette.svg`, `ui.palette.png` |
| `ui.add` | Interface | [`plus`](https://lucide.dev/icons/plus) | `ui.add.svg`, `ui.add.png` |
