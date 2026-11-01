//! The Instance inspector: the raw ids and every property (hidden ones too) of the selection,
//! as the engine stores them. Read-only; edits go through Properties.

use eframe::egui::{self, RichText};
use metallic_core::{InstanceId, Value, World, classes};

use crate::theme::{self, fonts};

/// Most selected instances listed (the rest are counted).
pub const MAX_SHOWN: usize = 8;
/// Longest value text shown, in characters.
pub const MAX_VALUE: usize = 160;

/// One property row.
#[derive(Clone, Debug, PartialEq)]
pub struct PropRow {
    /// Property name.
    pub name: &'static str,
    /// Its type, as the class declares it.
    pub ty: String,
    /// The stored value, as text (cut at [`MAX_VALUE`] characters).
    pub value: String,
    /// Engine bookkeeping that Luau and Properties do not show.
    pub hidden: bool,
}

/// What the inspector shows for one instance.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceReport {
    /// The id.
    pub id: InstanceId,
    /// Class name.
    pub class: &'static str,
    /// Full name ("Workspace.Coin").
    pub path: String,
    /// The parent's id.
    pub parent: Option<InstanceId>,
    /// Number of children.
    pub children: usize,
    /// Every property of the class chain, in declaration order.
    pub props: Vec<PropRow>,
}

/// "0x0000000100000007": the id's bits, which save files and logs use.
pub fn id_text(id: InstanceId) -> String {
    format!("{:#018x}", id.to_bits())
}

fn value_text(v: &Value) -> String {
    let text = match v {
        Value::Nil => "nil".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("{s:?}"),
        Value::Vector3(p) => format!("{}, {}, {}", p.x, p.y, p.z),
        Value::Color3(c) => format!("{}, {}, {}", c.r, c.g, c.b),
        Value::Ref(Some(id)) => id_text(*id),
        Value::Ref(None) => "nil".to_owned(),
        other => format!("{other:?}"),
    };
    if text.chars().count() > MAX_VALUE {
        let mut cut: String = text.chars().take(MAX_VALUE).collect();
        cut.push('…');
        cut
    } else {
        text
    }
}

/// The report for `id`, or `None` when it is not alive.
pub fn report(world: &World, id: InstanceId) -> Option<InstanceReport> {
    let class = world.class_name(id)?;
    let props = classes::all_properties(class)
        .into_iter()
        .map(|p| PropRow {
            name: p.name,
            ty: format!("{:?}", p.ty),
            value: world
                .get_ref(id, p.name)
                .map_or_else(|e| format!("<{e}>"), value_text),
            hidden: p.hidden,
        })
        .collect();
    Some(InstanceReport {
        id,
        class,
        path: world.full_name(id),
        parent: world.parent(id),
        children: world.children(id).len(),
        props,
    })
}

/// Draws the Instance inspector for `selection`. Returns false when it was closed.
pub fn show(ctx: &egui::Context, world: &World, selection: &[InstanceId]) -> bool {
    let mut open = true;
    egui::Window::new(super::overlay::window_title("Instance inspector"))
        .id(egui::Id::unique("dev-inspector"))
        .open(&mut open)
        .default_size([460.0, 420.0])
        .max_width(560.0)
        .default_pos([300.0, 190.0])
        .order(egui::Order::Foreground)
        .frame(super::overlay::window_frame())
        .show(ctx, |ui| {
            if selection.is_empty() {
                ui.label(
                    RichText::new("Select something to see its raw properties.")
                        .color(theme::text_3()),
                );
                return;
            }
            let mono = egui::TextStyle::Monospace.resolve(ui.style());
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for &id in selection.iter().take(MAX_SHOWN) {
                        let Some(r) = report(world, id) else { continue };
                        egui::CollapsingHeader::new(
                            RichText::new(format!("{} · {}", r.path, r.class))
                                .font(fonts::strong()),
                        )
                        .id_salt(("dev-inspect", id.to_bits()))
                        .default_open(selection.len() == 1)
                        .show(ui, |ui| instance_grid(ui, world, &r, &mono));
                    }
                    if selection.len() > MAX_SHOWN {
                        ui.label(
                            RichText::new(format!("and {} more", selection.len() - MAX_SHOWN))
                                .color(theme::text_3()),
                        );
                    }
                });
        });
    open
}

fn instance_grid(ui: &mut egui::Ui, world: &World, r: &InstanceReport, mono: &egui::FontId) {
    let key = |ui: &mut egui::Ui, text: &str| {
        ui.label(
            RichText::new(text)
                .font(fonts::small())
                .color(theme::text_2()),
        );
    };
    egui::Grid::new(("dev-inspect-grid", r.id.to_bits()))
        .num_columns(3)
        .spacing([12.0, 2.0])
        .striped(true)
        .show(ui, |ui| {
            key(ui, "Id");
            ui.label(RichText::new(id_text(r.id)).font(mono.clone()));
            ui.label("");
            ui.end_row();
            key(ui, "Parent");
            let parent = r.parent.map_or("none".to_owned(), |p| {
                format!("{} {}", id_text(p), world.full_name(p))
            });
            ui.label(RichText::new(parent).font(mono.clone()));
            ui.label("");
            ui.end_row();
            key(ui, "Children");
            ui.label(RichText::new(r.children.to_string()).font(mono.clone()));
            ui.label("");
            ui.end_row();
            for p in &r.props {
                let name = if p.hidden {
                    format!("{} (hidden)", p.name)
                } else {
                    p.name.to_owned()
                };
                key(ui, &name);
                ui.add(egui::Label::new(RichText::new(&p.value).font(mono.clone())).truncate())
                    .on_hover_text(&p.value);
                ui.label(
                    RichText::new(&p.ty)
                        .font(fonts::small())
                        .color(theme::text_3()),
                );
                ui.end_row();
            }
        });
}
