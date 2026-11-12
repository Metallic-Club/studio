//! The Search panel (#555): text found across every script of the game, as VS Code's search has it. Ctrl+Shift+F opens
//! it on the left, Ctrl+Shift+H with its replace field.
//!
//! - **The query** has the find bar's toggles: match case, whole word, regular expression ([`crate::code::find`]).
//! - **The results** list each script file with its matching lines, the match lit; a file's row folds its lines away.
//!   Clicking a line opens its script with the match selected.
//! - **Replacing** shows each match struck out with its replacement after it; a line, a file or every file can be
//!   replaced. What is replaced is typed text, kept in the project as typing is, until Save.

use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Rect, Role, ScrollArea, Sense, Stroke, Ui, WidgetInfo,
    vec2,
};

use super::{empty_note, ids};
use crate::code::find::{FileMatches, LineMatch};
use crate::code::find_bar;
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::theme::current_colors;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{space, text};

/// The Search panel.
#[derive(Clone, Copy, Debug)]
pub struct Search;

/// The height of a result's row.
const ROW: f32 = 22.0;

/// What a click in the results asked for.
enum Ask {
    /// Fold or unfold a file's lines.
    Fold(String),
    /// Open the script at a match.
    Open(crate::code::ScriptId, String, Range<usize>),
    /// Replace one match of a file.
    ReplaceOne(String, Range<usize>),
    /// Replace every match of a file.
    ReplaceFile(String),
}

impl Panel for Search {
    fn id(&self) -> &'static str {
        ids::SEARCH
    }

    fn title(&self) -> &'static str {
        "Search"
    }

    fn icon(&self) -> Icon {
        Icon::UiSearch
    }

    fn zone(&self) -> Zone {
        // Left of the script editor, as VS Code's search is.
        Zone::Left
    }

    fn context(&self) -> Context {
        Context::Editor
    }

    fn open_by_default(&self) -> bool {
        false
    }

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        let colors = current_colors(ui.ctx());
        let p = colors.palette;
        let editor = &mut *cx.editor;
        let id = Id::unique("metallic.search");
        let search = &mut editor.scripts.search;
        ui.add_space(space::S2);
        // The query, its toggles, and the replacement.
        let width = ui.available_width();
        let field_width = (width - 2.0 * space::S2 - 4.0 * 24.0).max(60.0);
        let error = search.finder().err();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(space::S1);
            let (icon, words) = if search.show_replace {
                (Icon::UiCollapse, "Hide replace")
            } else {
                (Icon::UiExpand, "Show replace")
            };
            if find_bar::icon_button(ui, id.with("toggle"), icon, words, true).clicked() {
                search.show_replace = !search.show_replace;
            }
            let query = id.with("query");
            find_bar::field(
                ui,
                query,
                &mut search.query.text,
                "Search scripts",
                "Search",
                field_width,
                error.is_some(),
            );
            if search.asked {
                search.asked = false;
                find_bar::focus_all(ui.ctx(), query, search.query.text.chars().count());
            }
            find_bar::toggle(ui, id.with("case"), "Aa", "Match case", &mut search.query.case);
            find_bar::toggle(ui, id.with("word"), "ab", "Match whole word", &mut search.query.word);
            find_bar::toggle(
                ui,
                id.with("regex"),
                ".*",
                "Use regular expression",
                &mut search.query.regex,
            );
        });
        let mut replace_all = false;
        if search.show_replace {
            ui.add_space(space::S1);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(space::S1 + 24.0);
                find_bar::field(
                    ui,
                    id.with("replace"),
                    &mut search.replace,
                    "Replace in scripts",
                    "Replace",
                    field_width,
                    false,
                );
                replace_all = find_bar::text_button(ui, id.with("all"), "All", "Replace all in scripts", true)
                    .on_hover_text("Replace every match in every script")
                    .clicked();
            });
        }
        ui.add_space(space::S2);
        let finder = match search.finder() {
            Ok(Some(finder)) => finder,
            Ok(None) => {
                empty_note(ui, "Type to search every script.");
                return;
            }
            Err(why) => {
                empty_note(ui, &why);
                return;
            }
        };
        let replace = search.show_replace.then(|| search.replace.clone());
        let folded = search.folded.clone();
        let files = editor.scripts.search(&mut editor.project, &editor.dm, &finder);
        let total: usize = files.iter().map(|file| file.matches.len()).sum();
        let summary = match (total, files.len()) {
            (0, _) => "No results".to_owned(),
            (1, _) => "1 result in 1 file".to_owned(),
            (total, 1) => format!("{total} results in 1 file"),
            (total, files) => format!("{total} results in {files} files"),
        };
        ui.horizontal(|ui| {
            ui.add_space(space::S3);
            ui.label(egui::RichText::new(summary).size(text::SMALL).color(p.muted));
        });
        ui.add_space(space::S1);
        let mut asks = Vec::new();
        ScrollArea::vertical()
            .id_salt(id.with("results"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for file in &files {
                    let open = !folded.contains(&file.member);
                    if let Some(ask) = file_row(ui, id, file, open, replace.is_some()) {
                        asks.push(ask);
                    }
                    if open {
                        for found in &file.matches {
                            if let Some(ask) = line_row(ui, id, file, found, replace.as_deref()) {
                                asks.push(ask);
                            }
                        }
                    }
                }
            });
        // What was asked, now that the results are drawn.
        if replace_all {
            for file in &files {
                replace_in(editor, &finder, &file.member, None);
            }
        }
        for ask in asks {
            match ask {
                Ask::Fold(member) => {
                    let folded = &mut editor.scripts.search.folded;
                    if !folded.remove(&member) {
                        folded.insert(member);
                    }
                }
                Ask::Open(script, member, range) => {
                    editor.open_script(script);
                    let text = editor.scripts.text(&mut editor.project, &member);
                    let start = text[..range.start.min(text.len())].chars().count();
                    let end = text[..range.end.min(text.len())].chars().count();
                    crate::code::view::select(ui.ctx(), super::script::field_id(script), start..end);
                }
                Ask::ReplaceOne(member, range) => replace_in(editor, &finder, &member, Some(range)),
                Ask::ReplaceFile(member) => replace_in(editor, &finder, &member, None),
            }
        }
    }
}

/// Replaces the matches of `finder` in `member` for the search's replacement: the one at `only`, or every one.
fn replace_in(
    editor: &mut crate::editor::EditorState,
    finder: &crate::code::find::Finder,
    member: &str,
    only: Option<Range<usize>>,
) {
    let with = editor.scripts.search.replace.clone();
    let text = editor.scripts.text(&mut editor.project, member).clone();
    let matches = finder.matches(&text);
    let replaced = match only {
        // Still a match: the text may have changed since the results were drawn.
        Some(range) if matches.contains(&range) => {
            let mut out = text.clone();
            out.replace_range(range.clone(), &finder.replacement(&text, range, &with));
            out
        }
        Some(_) => return,
        None if matches.is_empty() => return,
        None => finder.replace_all(&text, &matches, &with),
    };
    editor.scripts.replace_text(&mut editor.project, member, replaced);
}

/// A file's row of the results: its fold arrow, its script's name and file, and how many matches it has; with
/// `replace`, a button replacing them all under the pointer.
fn file_row(ui: &mut Ui, id: Id, file: &FileMatches, open: bool, replace: bool) -> Option<Ask> {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| WidgetInfo::labeled(Role::TreeItem, true, &file.member));
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter().rect_filled(rect, 0.0, p.surface_hover);
    }
    let mut x = rect.left() + space::S2;
    let arrow = if open { Icon::UiCollapse } else { Icon::UiExpand };
    icons::paint_at(
        ui.painter(),
        egui::pos2(x + 7.0, rect.center().y),
        Align2::CENTER_CENTER,
        arrow,
        text::SMALL,
        p.muted,
    );
    x += 16.0;
    icons::paint_at(
        ui.painter(),
        egui::pos2(x + 8.0, rect.center().y),
        Align2::CENTER_CENTER,
        Icon::ExplorerScript,
        text::BODY,
        Icon::ExplorerScript.color(&colors),
    );
    x += 20.0;
    let short = file.name.rsplit('.').next().unwrap_or(&file.name);
    let name = ui.painter().text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        short,
        FontId::proportional(text::BODY),
        p.text,
    );
    ui.painter().text(
        egui::pos2(name.right() + space::S2, rect.center().y),
        Align2::LEFT_CENTER,
        &file.member,
        FontId::proportional(text::SMALL),
        p.subtle,
    );
    // How many, in a badge at the right.
    let count = file.matches.len().to_string();
    let badge = Rect::from_center_size(
        egui::pos2(rect.right() - space::S3 - 10.0, rect.center().y),
        vec2(20.0f32.max(8.0 + 7.0 * count.len() as f32), 16.0),
    );
    ui.painter().rect_filled(badge, CornerRadius::same(8), p.surface);
    ui.painter().text(
        badge.center(),
        Align2::CENTER_CENTER,
        count,
        FontId::proportional(text::SMALL),
        p.muted,
    );
    if replace && hovered {
        let button = Rect::from_center_size(egui::pos2(badge.left() - 16.0, rect.center().y), vec2(22.0, 20.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(button)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let label = format!("Replace in {}", file.member);
        if find_bar::icon_button(
            &mut child,
            id.with(("replace-file", &file.member)),
            Icon::UiCheck,
            &label,
            true,
        )
        .on_hover_text("Replace every match in this file")
        .clicked()
        {
            return Some(Ask::ReplaceFile(file.member.clone()));
        }
    }
    response.clicked().then(|| Ask::Fold(file.member.clone()))
}

/// A match's row: its line around it, the match lit (struck out with `replace` after it, when replacing); a click opens
/// the script there, and with `replace` a button replaces it under the pointer.
fn line_row(ui: &mut Ui, id: Id, file: &FileMatches, found: &LineMatch, replace: Option<&str>) -> Option<Ask> {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let dark = colors.dark;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let label = format!(
        "{} line {} column {}: {}{}{}",
        file.member,
        found.line + 1,
        found.column + 1,
        found.before,
        found.found,
        found.after
    );
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter().rect_filled(rect, 0.0, p.surface_hover);
    }
    let font = FontId::monospace(text::SMALL);
    let plain = |color: Color32| TextFormat {
        font_id: font.clone(),
        color,
        ..TextFormat::default()
    };
    let warning = crate::code::view::severity_color(crate::code::analysis::Severity::Warning, dark);
    let mut job = LayoutJob::default();
    job.append(&found.before, 0.0, plain(p.muted));
    match replace {
        Some(with) => {
            let red = crate::code::view::severity_color(crate::code::analysis::Severity::Error, dark);
            job.append(
                &found.found,
                0.0,
                TextFormat {
                    background: red.gamma_multiply(0.25),
                    strikethrough: Stroke::new(1.0, p.muted),
                    ..plain(p.muted)
                },
            );
            job.append(
                with,
                0.0,
                TextFormat {
                    background: Color32::from_rgb(60, 160, 90).gamma_multiply(0.35),
                    ..plain(p.text)
                },
            );
        }
        None => job.append(
            &found.found,
            0.0,
            TextFormat {
                background: warning.gamma_multiply(0.3),
                ..plain(p.text)
            },
        ),
    }
    job.append(&found.after, 0.0, plain(p.muted));
    job.wrap.max_width = f32::INFINITY;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let left = rect.left() + space::S2 + 36.0;
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.galley(
        egui::pos2(left, rect.center().y - galley.size().y / 2.0),
        galley,
        p.text,
    );
    if replace.is_some() && hovered {
        let button = Rect::from_center_size(
            egui::pos2(rect.right() - space::S3 - 11.0, rect.center().y),
            vec2(22.0, 20.0),
        );
        ui.painter().rect_filled(button.expand(2.0), 0.0, p.surface_hover);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(button)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let label = format!(
            "Replace line {} column {} of {}",
            found.line + 1,
            found.column + 1,
            file.member
        );
        if find_bar::icon_button(
            &mut child,
            id.with(("replace-one", &file.member, found.range.start)),
            Icon::UiCheck,
            &label,
            true,
        )
        .on_hover_text("Replace")
        .clicked()
        {
            return Some(Ask::ReplaceOne(file.member.clone(), found.range.clone()));
        }
    }
    response
        .on_hover_text(format!("{} line {}", file.name, found.line + 1))
        .clicked()
        .then(|| Ask::Open(file.script, file.member.clone(), found.range.clone()))
}
