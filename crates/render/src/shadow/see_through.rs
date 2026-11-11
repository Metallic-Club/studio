//! See-through casters (#122, #491): the partly see-through parts a shadow view holds, grouped into draws, and the maps
//! beside a shadow map that they are drawn into.
//!
//! A shadow view's cull puts its see-through casters in its [`VisibleSet`]'s see-through list, in no particular order
//! and of every mesh kind. [`SeeThroughCasters::gather`] sorts them by mesh kind and level of detail, so they draw as
//! one instanced draw per kind and level, as the solid casters do. They are not in the solid casters' map: they are
//! drawn twice into [`SeeThroughMaps`], their depth (the nearest one's is kept) and the light they let through (each
//! multiplies what is there by its Transparency, so the order within a draw does not matter). A receiver behind the
//! nearest is lit by that light, read filtered: the shadow gets lighter smoothly as the part gets more see-through.

use crate::context::in_heap_turn;
use crate::cull::VisibleSet;
use crate::scene::{InstanceScene, MeshKind};
use crate::textures::{Slot, TextureMaker, TextureSpec};

/// One draw of see-through casters: `count` entries of one mesh kind and level, from `first` in the gathered order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SeeThroughRun {
    pub(crate) kind: MeshKind,
    pub(crate) level: u8,
    pub(crate) first: u32,
    pub(crate) count: u32,
}

/// A shadow view's see-through casters, sorted into runs. Reused from frame to frame: nothing allocates once it has
/// held as many as the scene has.
#[derive(Debug, Default)]
pub(crate) struct SeeThroughCasters {
    /// (mesh kind << 8 | level, entry), sorted.
    sorted: Vec<(u32, u32)>,
    runs: Vec<SeeThroughRun>,
}

impl SeeThroughCasters {
    /// Gathers `set`'s see-through entries, at most `room` of them, leaving out `skip` (a light's own part), and
    /// sorts them into runs.
    pub(crate) fn gather(&mut self, set: &VisibleSet, instances: &InstanceScene, room: usize, skip: Option<u32>) {
        self.sorted.clear();
        self.runs.clear();
        let entries = set.transparent().iter().zip(set.transparent_levels());
        for (&entry, &level) in entries {
            if self.sorted.len() >= room {
                break;
            }
            if Some(entry) == skip {
                continue;
            }
            if let Some(instance) = instances.entry_instance(entry) {
                self.sorted
                    .push(((u32::from(instance.mesh().0) << 8) | u32::from(level), entry));
            }
        }
        self.sorted.sort_unstable();
        for (index, &(key, _)) in (0u32..).zip(&self.sorted) {
            let kind = MeshKind((key >> 8) as u16);
            let level = (key & 0xff) as u8;
            match self.runs.last_mut() {
                Some(run) if run.kind == kind && run.level == level => run.count += 1,
                _ => self.runs.push(SeeThroughRun {
                    kind,
                    level,
                    first: index,
                    count: 1,
                }),
            }
        }
    }

    /// The gathered entries, run after run.
    pub(crate) fn entries(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.sorted.iter().map(|&(_, entry)| entry)
    }

    /// How many entries were gathered.
    pub(crate) fn len(&self) -> usize {
        self.sorted.len()
    }

    /// Whether none were.
    pub(crate) fn is_empty(&self) -> bool {
        self.sorted.is_empty()
    }

    /// The draws over [`SeeThroughCasters::entries`].
    pub(crate) fn runs(&self) -> &[SeeThroughRun] {
        &self.runs
    }
}

/// The format of the light a see-through caster lets through ([`SeeThroughMaps`]): one channel, 0 none to 1 all.
pub(crate) const SEE_THROUGH_LIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The see-through casters' maps beside a shadow map (#491): for each layer, the depth of the see-through caster
/// nearest the light, and the share of light the see-through casters there let through (their Transparency,
/// multiplied where several are drawn over one another). A receiver behind the nearest one is lit by that share; one
/// in front of it (the see-through part itself among them) is not dimmed. Both are filtered when read, so the shadow
/// fades smoothly.
///
/// Half the shadow map's resolution: a see-through shadow is soft anyway, and the maps cost a quarter of the memory.
/// They are made the first time they are wanted ([`SeeThroughMaps::want`]), off the frame like the shadow maps;
/// until they exist, see-through casters cast no shadow, and a shadow map with no see-through caster never reads
/// them. Placeholders are bound meanwhile.
#[derive(Debug)]
pub(crate) struct SeeThroughMaps {
    device: wgpu::Device,
    maker: TextureMaker,
    /// The size (resolution, layers) wanted, made or coming.
    wanted: Option<(u32, u32)>,
    /// Textures still being made: depth, light.
    incoming: Option<(Slot, Slot)>,
    made: Option<Made>,
    placeholder: (wgpu::TextureView, wgpu::TextureView),
    /// Changes whenever the views to sample are replaced.
    revision: u64,
}

/// The one-texel arrays bound in place of see-through maps not made yet (#491), the same views for the cascades and
/// the atlas: a frame binds the same two views twice, which costs nothing more than binding them once.
#[derive(Clone, Debug)]
pub(crate) struct SeeThroughPlaceholder {
    depth: wgpu::TextureView,
    light: wgpu::TextureView,
}

impl SeeThroughPlaceholder {
    /// The placeholders, with as many layers as an array needs on `device`.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layers = SeeThroughMaps::min_layers(device);
        let array = wgpu::TextureViewDimension::D2Array;
        Self {
            depth: view(&spec_depth(1, layers).make_now(device), array, None),
            light: view(&spec_light(1, layers).make_now(device), array, None),
        }
    }
}

/// The maps in use.
#[derive(Debug)]
struct Made {
    depth: wgpu::TextureView,
    light: wgpu::TextureView,
    /// Each layer's views to draw into: depth, light.
    layers: Vec<(wgpu::TextureView, wgpu::TextureView)>,
}

impl SeeThroughMaps {
    /// None yet, `placeholder` bound meanwhile. Both are read as arrays: a layer per cascade, or the atlas's one.
    pub(crate) fn new(device: &wgpu::Device, maker: &TextureMaker, placeholder: &SeeThroughPlaceholder) -> Self {
        let placeholder = (placeholder.depth.clone(), placeholder.light.clone());
        Self {
            device: device.clone(),
            maker: maker.clone(),
            wanted: None,
            incoming: None,
            made: None,
            placeholder,
            revision: 0,
        }
    }

    /// The fewest layers an array has: wgpu's OpenGL backend cannot read a one-layer texture as an array (#155).
    fn min_layers(device: &wgpu::Device) -> u32 {
        if device.adapter_info().backend == wgpu::Backend::Gl {
            2
        } else {
            1
        }
    }

    /// Asks for maps for a shadow map of `resolution`² texels and `layers` layers (made at half the resolution), when
    /// those in use or coming are another size.
    pub(crate) fn want(&mut self, resolution: u32, layers: u32) {
        let size = ((resolution / 2).max(16), layers.max(Self::min_layers(&self.device)));
        if self.wanted == Some(size) {
            return;
        }
        self.wanted = Some(size);
        if let Some((depth, light)) = self.incoming.take() {
            depth.cancel(&self.maker);
            light.cancel(&self.maker);
        }
        self.incoming = Some((
            Slot::new(self.maker.make(spec_depth(size.0, size.1))),
            Slot::new(self.maker.make(spec_light(size.0, size.1))),
        ));
    }

    /// Puts maps that arrived in use. Returns whether the views to sample were replaced (bind groups that hold them
    /// must be made again, and whatever the old maps held drawn again).
    pub(crate) fn poll(&mut self) -> bool {
        let Some((depth, light)) = self.incoming.as_mut() else {
            return false;
        };
        if !(depth.poll(&self.maker) & light.poll(&self.maker)) {
            return false;
        }
        let Some((depth, light)) = self.incoming.take() else {
            return false;
        };
        let (Some(depth), Some(light)) = (depth.into_texture(), light.into_texture()) else {
            return false;
        };
        let layers = (0..depth.depth_or_array_layers())
            .map(|layer| {
                let one = wgpu::TextureViewDimension::D2;
                (view(&depth, one, Some(layer)), view(&light, one, Some(layer)))
            })
            .collect();
        let array = wgpu::TextureViewDimension::D2Array;
        self.made = Some(Made {
            depth: view(&depth, array, None),
            light: view(&light, array, None),
            layers,
        });
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// Whether the maps exist.
    pub(crate) fn ready(&self) -> bool {
        self.made.is_some()
    }

    /// The nearest see-through casters' depth, to sample (a placeholder until the maps exist).
    pub(crate) fn depth_view(&self) -> &wgpu::TextureView {
        self.made.as_ref().map_or(&self.placeholder.0, |made| &made.depth)
    }

    /// The light the see-through casters let through, to sample (a placeholder until the maps exist).
    pub(crate) fn light_view(&self) -> &wgpu::TextureView {
        self.made.as_ref().map_or(&self.placeholder.1, |made| &made.light)
    }

    /// Layer `layer`'s views to draw into, once the maps exist: depth, light.
    pub(crate) fn targets(&self, layer: usize) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
        let (depth, light) = self.made.as_ref()?.layers.get(layer)?;
        Some((depth, light))
    }

    /// Changes whenever [`SeeThroughMaps::depth_view`] and [`SeeThroughMaps::light_view`] do.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Their memory, in bytes (none until they exist).
    pub(crate) fn gpu_bytes(&self) -> u64 {
        self.wanted
            .filter(|_| self.made.is_some())
            .map_or(0, |(resolution, layers)| {
                spec_depth(resolution, layers).bytes() + spec_light(resolution, layers).bytes()
            })
    }
}

fn spec_depth(resolution: u32, layers: u32) -> TextureSpec {
    TextureSpec::new(
        "metallic see-through shadow depth",
        (resolution, resolution, layers),
        super::SHADOW_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    )
}

fn spec_light(resolution: u32, layers: u32) -> TextureSpec {
    TextureSpec::new(
        "metallic see-through shadow light",
        (resolution, resolution, layers),
        SEE_THROUGH_LIGHT_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    )
}

/// A view of `texture` as `dimension`, of one `layer` or all.
fn view(texture: &wgpu::Texture, dimension: wgpu::TextureViewDimension, layer: Option<u32>) -> wgpu::TextureView {
    in_heap_turn(|| {
        texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("metallic see-through shadow"),
            dimension: Some(dimension),
            base_array_layer: layer.unwrap_or(0),
            array_layer_count: layer.map(|_| 1),
            ..wgpu::TextureViewDescriptor::default()
        })
    })
}

/// Clears a tile of the light map to "all light through": a full-viewport triangle writing 1 (the atlas's tiles
/// share one map, so a tile is cleared by drawing, as its depth is).
pub(crate) fn clear_light_pipeline(device: &wgpu::Device, cache: Option<&wgpu::PipelineCache>) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("metallic see-through clear"),
        source: wgpu::ShaderSource::Wgsl(
            "@vertex fn vs_clear(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
                return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
            }
            @fragment fn fs_clear() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }"
                .into(),
        ),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("metallic see-through clear"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_clear"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_clear"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(SEE_THROUGH_LIGHT_FORMAT.into())],
        }),
        multiview_mask: None,
        cache,
    })
}
