//! The render side of components, registered by name, and what each class needs to be drawn.
//!
//! The scene never names a class. A class is renderable when it has a component registered here (`PrimitiveMesh`
//! for `Part`; mesh parts add theirs later), and the properties the scene reads (`CFrame`, `Size`, `Color`, ...)
//! are looked up by name once per class when the scene is made. Property ids are stable across subclasses, so the
//! per-change cost is a table index, never a name lookup.
//!
//! Local lights work the same way: a class is a light when it has a light component registered here
//! (`PointLightEmitter`, `SpotLightEmitter`), and its `Light` properties are resolved once per class.

use metallic_core::glam::Vec3;
use metallic_core::{ClassInfo, DataModel, Id, PropId, PropType, Registry};

use super::instance::MeshKind;
use super::lights::LightKind;

/// What a mesh component's render side is given to choose an instance's mesh.
#[derive(Clone, Copy, Debug)]
pub struct MeshInput<'a> {
    /// The data model the instance lives in.
    pub dm: &'a DataModel,
    /// The instance.
    pub id: Id,
    /// The properties resolved for its class.
    pub props: &'a PartProps,
    /// Its `Size`, already read.
    pub size: Vec3,
}

/// Picks the mesh and the scale applied to the unit mesh for one instance.
pub type MeshFn = fn(&MeshInput<'_>) -> (MeshKind, Vec3);

/// The render side of one component: how an instance with it is drawn.
#[derive(Clone, Copy, Debug)]
pub struct MeshComponent {
    /// The component's name, as classes list it.
    pub name: &'static str,
    /// Chooses the mesh kind and its scale.
    pub mesh: MeshFn,
}

/// The render side of one light component: which kind of local light an instance with it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightComponent {
    /// The component's name, as classes list it.
    pub name: &'static str,
    /// The light it gives.
    pub kind: LightKind,
}

/// The components the renderer knows how to draw. A class with none of them is not rendered.
#[derive(Clone, Debug, Default)]
pub struct RenderComponents {
    components: Vec<MeshComponent>,
    lights: Vec<LightComponent>,
}

impl RenderComponents {
    /// No components: nothing is renderable.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The built-in components: `PrimitiveMesh` (a part drawn as the built-in mesh for its `Shape`), and the local
    /// lights `PointLightEmitter` and `SpotLightEmitter` ([`crate::lights`]).
    pub fn builtin() -> Self {
        let mut components = Self::empty();
        components.register(MeshComponent {
            name: "PrimitiveMesh",
            mesh: primitive_mesh,
        });
        components.register_light(LightComponent {
            name: "PointLightEmitter",
            kind: LightKind::Point,
        });
        components.register_light(LightComponent {
            name: "SpotLightEmitter",
            kind: LightKind::Spot,
        });
        components
    }

    /// Adds (or replaces) the render side of the light component `component.name`.
    pub fn register_light(&mut self, component: LightComponent) {
        match self.lights.iter_mut().find(|c| c.name == component.name) {
            Some(existing) => *existing = component,
            None => self.lights.push(component),
        }
    }

    /// Every registered light component, in registration order.
    pub fn lights(&self) -> impl Iterator<Item = &LightComponent> {
        self.lights.iter()
    }

    /// The first registered light component `class` has.
    fn light_for_class(&self, class: &ClassInfo) -> Option<LightComponent> {
        self.lights.iter().copied().find(|c| class.has_component(c.name))
    }

    /// Adds (or replaces) the render side of the component `component.name`. Earlier registrations win when a
    /// class has several registered components.
    pub fn register(&mut self, component: MeshComponent) {
        match self.components.iter_mut().find(|c| c.name == component.name) {
            Some(existing) => *existing = component,
            None => self.components.push(component),
        }
    }

    /// Every registered component, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &MeshComponent> {
        self.components.iter()
    }

    /// The first registered component `class` has.
    fn for_class(&self, class: &ClassInfo) -> Option<MeshComponent> {
        self.components.iter().copied().find(|c| class.has_component(c.name))
    }
}

/// The ids of the properties the scene reads, for one class. Every one but `CFrame` is optional: a class without
/// it draws with the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartProps {
    /// `CFrame` (world placement).
    pub cframe: PropId,
    /// `Size` (default 1 × 1 × 1).
    pub size: Option<PropId>,
    /// `Color` (default white).
    pub color: Option<PropId>,
    /// `Transparency` (default 0).
    pub transparency: Option<PropId>,
    /// `Material` (default row 0).
    pub material: Option<PropId>,
    /// `Shape` (read by `PrimitiveMesh`; default `Block`).
    pub shape: Option<PropId>,
    /// `CastShadow` (default true).
    pub cast_shadow: Option<PropId>,
    /// `Anchored`, the static/dynamic region hint (default false).
    pub anchored: Option<PropId>,
}

impl PartProps {
    /// Resolves the properties of `class`, checking each one's type. `None` when it has no `CFrame`.
    fn resolve(class: &ClassInfo) -> Option<Self> {
        let typed = |name: &str, ok: fn(&PropType) -> bool| {
            class
                .prop_id(name)
                .filter(|&id| class.prop(id).is_some_and(|p| ok(&p.meta.ty)))
        };
        Some(Self {
            cframe: typed("CFrame", |t| matches!(t, PropType::CFrame))?,
            size: typed("Size", |t| matches!(t, PropType::Vector3)),
            color: typed("Color", |t| matches!(t, PropType::Color3)),
            transparency: typed("Transparency", |t| matches!(t, PropType::Number)),
            material: typed("Material", |t| matches!(t, PropType::Enum(_))),
            shape: typed("Shape", |t| matches!(t, PropType::Enum(_))),
            cast_shadow: typed("CastShadow", |t| matches!(t, PropType::Bool)),
            anchored: typed("Anchored", |t| matches!(t, PropType::Bool)),
        })
    }
}

/// How the scene draws instances of one class.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClassBinding {
    pub(crate) props: PartProps,
    pub(crate) mesh: MeshFn,
}

/// The ids of the `Light` properties the scene reads, for one light class. Every one but `CFrame` is optional: a
/// class without it lights with `Light`'s default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LightProps {
    pub(crate) cframe: PropId,
    pub(crate) enabled: Option<PropId>,
    pub(crate) brightness: Option<PropId>,
    pub(crate) color: Option<PropId>,
    pub(crate) range: Option<PropId>,
    pub(crate) shadows: Option<PropId>,
    /// `SpotLight.Angle`.
    pub(crate) angle: Option<PropId>,
}

impl LightProps {
    fn resolve(class: &ClassInfo) -> Option<Self> {
        let typed = |name: &str, ok: fn(&PropType) -> bool| {
            class
                .prop_id(name)
                .filter(|&id| class.prop(id).is_some_and(|p| ok(&p.meta.ty)))
        };
        let number: fn(&PropType) -> bool = |t| matches!(t, PropType::Number);
        let boolean: fn(&PropType) -> bool = |t| matches!(t, PropType::Bool);
        Some(Self {
            cframe: typed("CFrame", |t| matches!(t, PropType::CFrame))?,
            enabled: typed("Enabled", boolean),
            brightness: typed("Brightness", number),
            color: typed("Color", |t| matches!(t, PropType::Color3)),
            range: typed("Range", number),
            shadows: typed("Shadows", boolean),
            angle: typed("Angle", number),
        })
    }
}

/// How the scene reads instances of one light class.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LightBinding {
    pub(crate) kind: LightKind,
    pub(crate) props: LightProps,
}

/// Per class id, how it is drawn (`None`: not renderable) or which light it is. Built once per registry.
#[derive(Debug, Default)]
pub(crate) struct ClassTable {
    classes: Vec<Option<ClassBinding>>,
    lights: Vec<Option<LightBinding>>,
    /// The `Material` enum's items, whose positions are the material table's rows.
    materials: &'static [&'static str],
}

impl ClassTable {
    pub(crate) fn new(registry: &Registry, components: &RenderComponents) -> Self {
        let classes = registry
            .classes()
            .iter()
            .map(|class| {
                let component = components.for_class(class)?;
                Some(ClassBinding {
                    props: PartProps::resolve(class)?,
                    mesh: component.mesh,
                })
            })
            .collect();
        let lights = registry
            .classes()
            .iter()
            .map(|class| {
                let component = components.light_for_class(class)?;
                Some(LightBinding {
                    kind: component.kind,
                    props: LightProps::resolve(class)?,
                })
            })
            .collect();
        Self {
            classes,
            lights,
            materials: registry.enum_desc("Material").map_or(&[], |e| e.items),
        }
    }

    #[inline]
    pub(crate) fn get(&self, class: metallic_core::ClassId) -> Option<&ClassBinding> {
        self.classes.get(class.index()).and_then(Option::as_ref)
    }

    /// How instances of `class` light the scene, when it is a light class.
    #[inline]
    pub(crate) fn light(&self, class: metallic_core::ClassId) -> Option<&LightBinding> {
        self.lights.get(class.index()).and_then(Option::as_ref)
    }

    /// The `Material` enum's items, in row order.
    pub(crate) fn materials(&self) -> &'static [&'static str] {
        self.materials
    }

    /// The material table row of the `Material` item named `name` (its position in the enum); 0 when unknown.
    #[inline]
    pub(crate) fn material_row(&self, name: &str) -> u16 {
        self.materials
            .iter()
            .position(|&item| item == name)
            .and_then(|row| u16::try_from(row).ok())
            .unwrap_or(0)
    }
}

/// `PrimitiveMesh`: the built-in mesh for the part's `Shape`, scaled as Roblox and the physics colliders do: a
/// Ball's diameter is the smallest side; a Cylinder runs along X with the smaller of Y and Z as its diameter; Block
/// and Wedge fill the whole size.
fn primitive_mesh(input: &MeshInput<'_>) -> (MeshKind, Vec3) {
    let shape = input
        .props
        .shape
        .and_then(|prop| input.dm.enum_item(input.id, prop).ok())
        .map_or(MeshKind::BLOCK, |item| match item.name() {
            "Ball" => MeshKind::BALL,
            "Cylinder" => MeshKind::CYLINDER,
            "Wedge" => MeshKind::WEDGE,
            _ => MeshKind::BLOCK,
        });
    let size = input.size;
    let scale = match shape {
        MeshKind::BALL => Vec3::splat(size.min_element()),
        MeshKind::CYLINDER => {
            let diameter = size.y.min(size.z);
            Vec3::new(size.x, diameter, diameter)
        }
        _ => size,
    };
    (shape, scale)
}
