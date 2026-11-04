//! The frame-graph-lite: an ordered list of passes with explicit attachments.
//!
//! A frame is at most these passes, in this order: depth prepass, opaque, sky, transparent (the *scene* passes), the
//! post effects (MLAA's edge weights, bloom; in the order of a [`PostChain`]), the finish pass, upscale, UI. The sky
//! comes after the opaque scene, drawn at the far plane with a depth test, so it only shades pixels nothing else
//! covered (ADR 0008 §7). Which passes run is a [`PassSet`]; disabled passes cost nothing: no pass is begun, and the
//! renderer makes no texture and binds nothing for them.
//!
//! [`FramePlan`] works out every attachment's load and store operations from the passes that run, once, when the
//! pass set or MSAA changes (never per frame):
//!
//! - the first pass that touches an attachment clears it, later ones load it;
//! - a pass stores an attachment only when a later pass reads it; otherwise the result is discarded, which on
//!   tile-based (mobile) GPUs saves writing the tile back to memory;
//! - with MSAA the multisampled colour is resolved into its single-sampled target inside its last pass and then
//!   discarded;
//! - an attachment only one pass touches can be transient (memoryless on tile-based GPUs).
//!
//! **The post chain (#85).** With [`PassId::Post`] in the set (the default), the scene passes draw into the *scene
//! colour* ([`ColorAttachment::Scene`]), an HDR texture ([`crate::post`]), and the finish pass reads it and writes the
//! output: exposure, bloom composite, tone curve and grade in one full-screen pass, and with a render scale the
//! bilinear upscale too, so [`PassId::Upscale`] is never planned beside it. The effects between them
//! ([`PassId::Mlaa`], [`PassId::Bloom`]) only read the scene colour and write their own textures, so the plan gives
//! them no attachment; they run only with the finish pass, which consumes what they make. UI then draws on the output
//! at its full resolution. With MSAA the multisampled colour resolves into the scene colour.
//!
//! **Without the finish pass** the scene draws straight into the output, as before the post chain, or with
//! [`PassId::Upscale`] (a render scale, [`crate::resolution`]) into the scene colour, which the upscale pass draws over
//! the output.
//!
//! **Merged scene passes.** With [`PassId::Scene`] in the set the scene passes are recorded as one render pass
//! instead of one each, so their colour and depth stay in tile memory on tile-based GPUs: depth (and the multisampled
//! colour, resolved at the end of it) is cleared and discarded within that one pass and can be transient. Desktop
//! GPUs gain little from it and lose the per-pass GPU timings, so the presets merge them only where tiles matter.

use std::fmt;

/// One pass of the frame, in frame order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum PassId {
    /// Depth only, before opaque shading, for scenes with heavy overdraw (a per-preset option).
    DepthPrepass,
    /// Opaque geometry.
    Opaque,
    /// The sky, at the far plane, after all opaque geometry.
    Sky,
    /// Transparent geometry, sorted back to front, over the sky.
    Transparent,
    /// The scene passes above recorded as one render pass. In a [`PassSet`] it asks for them to be merged; the plan
    /// then runs this one pass in their place (see the module documentation).
    Scene,
    /// MLAA's edges and blend weights ([`crate::post`]): reads the scene colour, writes the weights the finish pass
    /// blends with. Runs only with [`PassId::Post`].
    Mlaa,
    /// Bloom's chain of downsampling and upsampling passes at half the scene's resolution and below, timed as one.
    /// Runs only with [`PassId::Post`].
    Bloom,
    /// The finish pass: exposure, bloom composite, tone curve and grade, from the scene colour into the output (with
    /// a render scale, upscaled on the way).
    Post,
    /// Draws the scene colour, rendered at the render scale, over the whole output (a bilinear upscale; see
    /// [`crate::resolution`]). Only planned when the scene is drawn at a render scale without the finish pass, which
    /// otherwise upscales itself.
    Upscale,
    /// Screen-space UI.
    Ui,
}

impl PassId {
    /// Every pass, in frame order.
    pub const ALL: [PassId; 10] = [
        PassId::DepthPrepass,
        PassId::Opaque,
        PassId::Sky,
        PassId::Transparent,
        PassId::Scene,
        PassId::Mlaa,
        PassId::Bloom,
        PassId::Post,
        PassId::Upscale,
        PassId::Ui,
    ];

    /// How many passes a frame can have.
    pub const COUNT: usize = Self::ALL.len();

    /// The pass's name in timings and GPU debuggers.
    pub const fn name(self) -> &'static str {
        match self {
            PassId::DepthPrepass => "depth_prepass",
            PassId::Opaque => "opaque",
            PassId::Sky => "sky",
            PassId::Transparent => "transparent",
            PassId::Scene => "scene",
            PassId::Mlaa => "mlaa",
            PassId::Bloom => "bloom",
            PassId::Post => "post",
            PassId::Upscale => "upscale",
            PassId::Ui => "ui",
        }
    }

    const fn bit(self) -> u16 {
        1 << self as u8
    }

    /// Whether the pass is one of the scene passes, which draw geometry and the sky against the depth buffer.
    pub const fn is_scene(self) -> bool {
        matches!(
            self,
            PassId::DepthPrepass | PassId::Opaque | PassId::Sky | PassId::Transparent | PassId::Scene
        )
    }

    /// Whether the pass draws into the scene colour; `false` for the depth-only prepass and for the output passes.
    const fn draws_scene_color(self) -> bool {
        matches!(self, PassId::Opaque | PassId::Sky | PassId::Transparent | PassId::Scene)
    }

    /// Whether the pass is a post effect between the scene and the finish pass, which reads the scene colour and
    /// writes textures of its own.
    pub const fn is_effect(self) -> bool {
        matches!(self, PassId::Mlaa | PassId::Bloom)
    }

    /// How the pass uses depth, if at all, given whether a depth prepass runs.
    const fn depth_use(self, prepass: bool) -> Option<DepthUse> {
        match self {
            PassId::DepthPrepass | PassId::Scene => Some(DepthUse::Write),
            // After a prepass the depth is final; opaque shading only tests it (`Equal`/`GreaterEqual`).
            PassId::Opaque if prepass => Some(DepthUse::Test),
            PassId::Opaque => Some(DepthUse::Write),
            PassId::Sky | PassId::Transparent => Some(DepthUse::Test),
            PassId::Mlaa | PassId::Bloom | PassId::Post | PassId::Upscale | PassId::Ui => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DepthUse {
    Write,
    Test,
}

/// A set of passes. The default is opaque, sky, transparent, bloom, the finish pass and UI.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PassSet(u16);

impl PassSet {
    /// No passes.
    pub const EMPTY: PassSet = PassSet(0);

    /// Every pass.
    pub const ALL: PassSet = PassSet((1 << PassId::COUNT) - 1);

    /// The scene passes (prepass, opaque, sky, transparent) and nothing else.
    pub const SCENE: PassSet = PassSet::EMPTY
        .with(PassId::DepthPrepass)
        .with(PassId::Opaque)
        .with(PassId::Sky)
        .with(PassId::Transparent);

    /// This set with `pass` added.
    #[must_use]
    pub const fn with(self, pass: PassId) -> Self {
        Self(self.0 | pass.bit())
    }

    /// This set without `pass`.
    #[must_use]
    pub const fn without(self, pass: PassId) -> Self {
        Self(self.0 & !pass.bit())
    }

    /// This set with `pass` added when `on`, without it otherwise.
    #[must_use]
    pub const fn with_if(self, pass: PassId, on: bool) -> Self {
        if on { self.with(pass) } else { self.without(pass) }
    }

    /// Whether `pass` is in the set.
    pub const fn contains(self, pass: PassId) -> bool {
        self.0 & pass.bit() != 0
    }

    /// Whether the set has no passes.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The passes in the set, in frame order.
    pub fn iter(self) -> impl Iterator<Item = PassId> {
        PassId::ALL.into_iter().filter(move |&pass| self.contains(pass))
    }

    /// The passes in both sets.
    #[must_use]
    pub const fn intersect(self, other: PassSet) -> Self {
        Self(self.0 & other.0)
    }
}

impl Default for PassSet {
    fn default() -> Self {
        PassSet::EMPTY
            .with(PassId::Opaque)
            .with(PassId::Sky)
            .with(PassId::Transparent)
            .with(PassId::Bloom)
            .with(PassId::Post)
            .with(PassId::Ui)
    }
}

impl fmt::Debug for PassSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter().map(PassId::name)).finish()
    }
}

/// The order the post effects run in, between the scene and the finish pass: an open chain, where effects are added,
/// removed and reordered. Which of them run is still the [`PassSet`]'s choice; an effect in the set but not in the
/// chain runs after the chain's, in frame order.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PostChain {
    effects: [PassId; PostChain::CAPACITY],
    len: u8,
}

impl PostChain {
    /// The most effects a chain holds.
    pub const CAPACITY: usize = 4;

    /// No effects.
    pub const EMPTY: PostChain = PostChain {
        effects: [PassId::Post; PostChain::CAPACITY],
        len: 0,
    };

    /// The default order: MLAA's weights, then bloom.
    pub const DEFAULT: PostChain = PostChain::EMPTY.push(PassId::Mlaa).push(PassId::Bloom);

    /// The chain with `effect` added at its end (moved there if it was in it). Passes that are not effects
    /// ([`PassId::is_effect`]) and effects beyond [`PostChain::CAPACITY`] are ignored.
    #[must_use]
    pub const fn push(self, effect: PassId) -> Self {
        let mut chain = self.remove(effect);
        if effect.is_effect() && (chain.len as usize) < Self::CAPACITY {
            chain.effects[chain.len as usize] = effect;
            chain.len += 1;
        }
        chain
    }

    /// The chain without `effect`.
    #[must_use]
    pub const fn remove(self, effect: PassId) -> Self {
        let mut chain = PostChain::EMPTY;
        let mut i = 0;
        while i < self.len as usize {
            if self.effects[i] as u8 != effect as u8 {
                chain.effects[chain.len as usize] = self.effects[i];
                chain.len += 1;
            }
            i += 1;
        }
        chain
    }

    /// The effects, in the order they run.
    pub fn effects(&self) -> &[PassId] {
        &self.effects[..usize::from(self.len)]
    }
}

impl Default for PostChain {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Debug for PostChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.effects().iter().map(|p| p.name())).finish()
    }
}

/// Load and store for one attachment in one pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachmentOps {
    /// Clear on load (the first pass to touch the attachment); otherwise load the previous contents.
    pub clear: bool,
    /// Keep the result for a later pass; otherwise discard it.
    pub store: bool,
}

/// Which texture a pass's colour attachment is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorAttachment {
    /// The frame's output (the swapchain image or the offscreen target).
    Output(AttachmentOps),
    /// The scene colour: the HDR texture the finish pass reads, or without it the texture at the render scale the
    /// upscale pass reads.
    Scene(AttachmentOps),
    /// The multisampled scene colour. `resolve` resolves it into the scene colour (without one, the output) at the
    /// end of the pass.
    Msaa {
        /// Load and store of the multisampled texture.
        ops: AttachmentOps,
        /// Resolve at the end of this pass (its last use).
        resolve: bool,
    },
}

/// How a pass uses the depth buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthOps {
    /// The pass writes depth.
    Write(AttachmentOps),
    /// The pass only tests against depth (bound read-only, contents kept).
    ReadOnly,
}

/// One pass of a [`FramePlan`] with its attachments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlannedPass {
    /// Which pass.
    pub id: PassId,
    /// The colour attachment; `None` for a depth-only pass and for the post effects, which draw into textures of
    /// their own.
    pub color: Option<ColorAttachment>,
    /// The depth attachment; `None` when the pass does not use depth.
    pub depth: Option<DepthOps>,
    /// The passes this one runs, in order: itself, or for [`PassId::Scene`] the scene passes it merges.
    pub runs: PassSet,
}

/// The passes of a frame with every attachment's load and store operations resolved. See the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramePlan {
    passes: [PlannedPass; PassId::COUNT],
    len: usize,
    msaa_samples: u32,
    depth_uses: u8,
    msaa_uses: u8,
    scaled: bool,
    post: bool,
    to_scene_color: bool,
}

impl FramePlan {
    /// Plans the frame for `passes`, with the scene drawn at `msaa_samples` per pixel (1 for no MSAA) and the post
    /// effects in the default order ([`PostChain::DEFAULT`]). See [`FramePlan::with_chain`].
    pub fn new(passes: PassSet, msaa_samples: u32) -> Self {
        Self::with_chain(passes, PostChain::DEFAULT, msaa_samples)
    }

    /// Plans the frame for `passes`, with the scene drawn at `msaa_samples` per pixel and the post effects in
    /// `chain`'s order. The finish pass, the effects and the upscale are left out when no pass draws the scene; the
    /// effects, also without the finish pass, and the upscale with it (the finish pass upscales). With
    /// [`PassId::Upscale`] in `passes` the scene is drawn at a render scale ([`FramePlan::scaled`]).
    pub fn with_chain(passes: PassSet, chain: PostChain, msaa_samples: u32) -> Self {
        let msaa = msaa_samples > 1;
        let has_scene = passes.iter().any(PassId::draws_scene_color);
        let post = passes.contains(PassId::Post) && has_scene;
        let scaled = passes.contains(PassId::Upscale) && has_scene;
        let to_scene_color = post || scaled;
        let scene_passes = passes.intersect(PassSet::SCENE);
        let merge = passes.contains(PassId::Scene) && !scene_passes.is_empty();
        let prepass = passes.contains(PassId::DepthPrepass);

        // The planned passes, in order.
        let mut order = [PassId::Post; PassId::COUNT];
        let mut runs = [PassSet::EMPTY; PassId::COUNT];
        let mut len = 0;
        let mut add = |id: PassId, set: PassSet| {
            order[len] = id;
            runs[len] = set;
            len += 1;
        };
        if merge {
            add(PassId::Scene, scene_passes);
        } else {
            for pass in scene_passes.iter() {
                add(pass, PassSet::EMPTY.with(pass));
            }
        }
        if post {
            let effects = chain.effects().iter().copied().chain(
                PassId::ALL
                    .into_iter()
                    .filter(|p| p.is_effect() && !chain.effects().contains(p)),
            );
            for effect in effects.filter(|&e| passes.contains(e)) {
                add(effect, PassSet::EMPTY.with(effect));
            }
            add(PassId::Post, PassSet::EMPTY.with(PassId::Post));
        } else if scaled {
            add(PassId::Upscale, PassSet::EMPTY.with(PassId::Upscale));
        }
        if passes.contains(PassId::Ui) {
            add(PassId::Ui, PassSet::EMPTY.with(PassId::Ui));
        }

        let placeholder = PlannedPass {
            id: PassId::Opaque,
            color: None,
            depth: None,
            runs: PassSet::EMPTY,
        };
        let mut plan = Self {
            passes: [placeholder; PassId::COUNT],
            len: 0,
            msaa_samples: msaa_samples.max(1),
            depth_uses: 0,
            msaa_uses: 0,
            scaled,
            post,
            to_scene_color,
        };

        // Which pass indices (bits) use each attachment. Resolving into the scene colour (or the output) counts as
        // using it, and so does a later pass reading the scene colour (the effects, finish and upscale passes).
        let (mut depth_users, mut msaa_users, mut output_users, mut scene_users) = (0u16, 0u16, 0u16, 0u16);
        for (i, &pass) in order[..len].iter().enumerate() {
            let bit = 1 << i;
            if pass.depth_use(prepass).is_some() {
                depth_users |= bit;
            }
            if pass.draws_scene_color() {
                if msaa {
                    msaa_users |= bit;
                } else if to_scene_color {
                    scene_users |= bit;
                } else {
                    output_users |= bit;
                }
            }
            if to_scene_color && (pass.is_effect() || matches!(pass, PassId::Post | PassId::Upscale)) {
                scene_users |= bit;
            }
            if matches!(pass, PassId::Post | PassId::Upscale | PassId::Ui) {
                output_users |= bit;
            }
        }
        // The last multisampled pass resolves into the scene colour, or the output.
        let resolver = last_index(msaa_users);
        if let Some(r) = resolver {
            if to_scene_color {
                scene_users |= 1 << r;
            } else {
                output_users |= 1 << r;
            }
        }

        for (i, &pass) in order[..len].iter().enumerate() {
            let ops = |users: u16| AttachmentOps {
                clear: first_index(users) == Some(i),
                store: last_index(users) != Some(i),
            };
            let color = if pass.draws_scene_color() && msaa {
                Some(ColorAttachment::Msaa {
                    ops: ops(msaa_users),
                    resolve: resolver == Some(i),
                })
            } else if pass.draws_scene_color() && to_scene_color {
                // Every writer stores: a later pass reads the scene colour.
                Some(ColorAttachment::Scene(ops(scene_users)))
            } else if pass.draws_scene_color() || matches!(pass, PassId::Post | PassId::Upscale | PassId::Ui) {
                // The output is what the frame produces: always stored.
                Some(ColorAttachment::Output(AttachmentOps {
                    clear: first_index(output_users) == Some(i),
                    store: true,
                }))
            } else {
                None
            };
            let depth = pass.depth_use(prepass).map(|depth_use| {
                let ops = ops(depth_users);
                // The first pass to touch depth writes it (with a clear), even one that would only test it.
                if depth_use == DepthUse::Write || ops.clear {
                    DepthOps::Write(ops)
                } else {
                    DepthOps::ReadOnly
                }
            });
            plan.passes[i] = PlannedPass {
                id: pass,
                color,
                depth,
                runs: runs[i],
            };
            plan.len = i + 1;
        }
        plan.depth_uses = depth_users.count_ones() as u8;
        plan.msaa_uses = msaa_users.count_ones() as u8;
        plan
    }

    /// The passes that run, in order.
    pub fn passes(&self) -> &[PlannedPass] {
        &self.passes[..self.len]
    }

    /// Whether `pass` runs as a pass of its own (not merged into [`PassId::Scene`]).
    pub fn contains(&self, pass: PassId) -> bool {
        self.passes().iter().any(|p| p.id == pass)
    }

    /// Samples per pixel of the scene colour and depth.
    pub fn msaa_samples(&self) -> u32 {
        self.msaa_samples
    }

    /// Whether any pass uses the depth buffer.
    pub fn uses_depth(&self) -> bool {
        self.depth_uses > 0
    }

    /// Whether any pass draws into a multisampled scene colour.
    pub fn uses_msaa_color(&self) -> bool {
        self.msaa_uses > 0
    }

    /// Whether the depth buffer lives within a single pass (cleared and discarded there), so it can be transient.
    pub fn depth_transient(&self) -> bool {
        self.depth_uses == 1
    }

    /// Whether the multisampled scene colour lives within a single pass, so it can be transient.
    pub fn msaa_color_transient(&self) -> bool {
        self.msaa_uses == 1
    }

    /// Whether the scene is drawn at a render scale ([`PassId::Upscale`] asked for): upscaled by the finish pass, or
    /// without it by the upscale pass.
    pub fn scaled(&self) -> bool {
        self.scaled
    }

    /// Whether the finish pass runs, so the scene is drawn into the HDR scene colour and finished from it.
    pub fn post(&self) -> bool {
        self.post
    }

    /// Whether the scene is drawn into the scene colour, which a later pass reads (the finish pass, or the upscale).
    pub fn uses_scene_color(&self) -> bool {
        self.to_scene_color
    }

    /// How many of the passes read or write a whole target at the scene's or the output's resolution: every planned
    /// pass but bloom, whose chain works at half the scene's resolution and below. Merged scene passes count once.
    pub fn full_screen_passes(&self) -> usize {
        self.passes().iter().filter(|p| p.id != PassId::Bloom).count()
    }
}

fn first_index(bits: u16) -> Option<usize> {
    (bits != 0).then(|| bits.trailing_zeros() as usize)
}

fn last_index(bits: u16) -> Option<usize> {
    (bits != 0).then(|| 15 - bits.leading_zeros() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAR_STORE: AttachmentOps = AttachmentOps {
        clear: true,
        store: true,
    };
    const LOAD_STORE: AttachmentOps = AttachmentOps {
        clear: false,
        store: true,
    };
    const CLEAR_DISCARD: AttachmentOps = AttachmentOps {
        clear: true,
        store: false,
    };
    const LOAD_DISCARD: AttachmentOps = AttachmentOps {
        clear: false,
        store: false,
    };

    /// The passes before the post chain: opaque, sky, transparent and UI, drawn straight into the output.
    fn direct() -> PassSet {
        PassSet::default().without(PassId::Post).without(PassId::Bloom)
    }

    fn pass(plan: &FramePlan, id: PassId) -> PlannedPass {
        *plan.passes().iter().find(|p| p.id == id).expect("pass is planned")
    }

    fn names(plan: &FramePlan) -> Vec<&'static str> {
        plan.passes().iter().map(|p| p.id.name()).collect()
    }

    #[test]
    fn passes_run_in_frame_order() {
        let every = PassSet::ALL.without(PassId::Scene);
        let plan = FramePlan::new(every, 1);
        // The finish pass upscales, so the upscale pass is never planned beside it.
        assert_eq!(
            names(&plan),
            [
                "depth_prepass",
                "opaque",
                "sky",
                "transparent",
                "mlaa",
                "bloom",
                "post",
                "ui"
            ]
        );
        assert_eq!(
            names(&FramePlan::new(PassSet::default(), 1)),
            ["opaque", "sky", "transparent", "bloom", "post", "ui"]
        );
        assert_eq!(
            names(&FramePlan::new(direct(), 1)),
            ["opaque", "sky", "transparent", "ui"]
        );
    }

    #[test]
    fn default_frame_draws_the_scene_into_the_scene_colour_and_finishes_it() {
        let plan = FramePlan::new(PassSet::default(), 1);
        assert!(plan.post() && plan.uses_scene_color() && !plan.scaled());
        let opaque = pass(&plan, PassId::Opaque);
        assert_eq!(opaque.color, Some(ColorAttachment::Scene(CLEAR_STORE)));
        assert_eq!(opaque.depth, Some(DepthOps::Write(CLEAR_STORE)));
        assert_eq!(pass(&plan, PassId::Sky).color, Some(ColorAttachment::Scene(LOAD_STORE)));
        // The finish pass reads what the last scene pass wrote.
        let transparent = pass(&plan, PassId::Transparent);
        assert_eq!(transparent.color, Some(ColorAttachment::Scene(LOAD_STORE)));
        assert_eq!(transparent.depth, Some(DepthOps::ReadOnly));
        // Bloom draws into textures of its own.
        assert_eq!(pass(&plan, PassId::Bloom).color, None);
        assert_eq!(pass(&plan, PassId::Bloom).depth, None);
        // Nothing wrote the output before the finish pass, which covers all of it.
        let post = pass(&plan, PassId::Post);
        assert_eq!(post.color, Some(ColorAttachment::Output(CLEAR_STORE)));
        assert_eq!(post.depth, None);
        assert_eq!(pass(&plan, PassId::Ui).color, Some(ColorAttachment::Output(LOAD_STORE)));
        // Opaque, sky, transparent, the finish pass and UI: bloom works below the scene's resolution.
        assert_eq!(plan.full_screen_passes(), 5);
    }

    #[test]
    fn direct_frame_clears_once_and_discards_depth_after_its_last_use() {
        let plan = FramePlan::new(direct(), 1);
        assert!(!plan.post() && !plan.uses_scene_color());
        let opaque = pass(&plan, PassId::Opaque);
        assert_eq!(opaque.color, Some(ColorAttachment::Output(CLEAR_STORE)));
        // Opaque writes depth that sky and transparent test, so it stores it.
        assert_eq!(opaque.depth, Some(DepthOps::Write(CLEAR_STORE)));
        let sky = pass(&plan, PassId::Sky);
        assert_eq!(sky.color, Some(ColorAttachment::Output(LOAD_STORE)));
        assert_eq!(sky.depth, Some(DepthOps::ReadOnly));
        assert_eq!(pass(&plan, PassId::Transparent).depth, Some(DepthOps::ReadOnly));
        let ui = pass(&plan, PassId::Ui);
        assert_eq!(ui.color, Some(ColorAttachment::Output(LOAD_STORE)));
        assert_eq!(ui.depth, None);
        assert!(plan.uses_depth() && !plan.depth_transient());
        assert!(!plan.uses_msaa_color());
    }

    #[test]
    fn depth_written_last_is_discarded() {
        let plan = FramePlan::new(PassSet::EMPTY.with(PassId::Opaque).with(PassId::Ui), 1);
        assert_eq!(pass(&plan, PassId::Opaque).depth, Some(DepthOps::Write(CLEAR_DISCARD)));
        assert!(plan.depth_transient());
    }

    #[test]
    fn prepass_writes_depth_and_opaque_only_tests_it() {
        let plan = FramePlan::new(direct().with(PassId::DepthPrepass), 1);
        let prepass = pass(&plan, PassId::DepthPrepass);
        assert_eq!(prepass.color, None);
        assert_eq!(prepass.depth, Some(DepthOps::Write(CLEAR_STORE)));
        let opaque = pass(&plan, PassId::Opaque);
        assert_eq!(opaque.depth, Some(DepthOps::ReadOnly));
        // The prepass has no colour, so opaque is still the first to touch it.
        assert_eq!(opaque.color, Some(ColorAttachment::Output(CLEAR_STORE)));
    }

    #[test]
    fn the_first_depth_user_clears_even_if_it_only_tests() {
        let plan = FramePlan::new(PassSet::EMPTY.with(PassId::Sky).with(PassId::Transparent), 1);
        assert_eq!(pass(&plan, PassId::Sky).depth, Some(DepthOps::Write(CLEAR_STORE)));
        assert_eq!(pass(&plan, PassId::Transparent).depth, Some(DepthOps::ReadOnly));
    }

    #[test]
    fn msaa_resolves_in_the_last_scene_pass_and_is_discarded() {
        let plan = FramePlan::new(direct(), 4);
        assert_eq!(plan.msaa_samples(), 4);
        assert_eq!(
            pass(&plan, PassId::Opaque).color,
            Some(ColorAttachment::Msaa {
                ops: CLEAR_STORE,
                resolve: false
            })
        );
        assert_eq!(
            pass(&plan, PassId::Sky).color,
            Some(ColorAttachment::Msaa {
                ops: LOAD_STORE,
                resolve: false
            })
        );
        assert_eq!(
            pass(&plan, PassId::Transparent).color,
            Some(ColorAttachment::Msaa {
                ops: LOAD_DISCARD,
                resolve: true
            })
        );
        // The resolve wrote the output, so UI loads it.
        assert_eq!(pass(&plan, PassId::Ui).color, Some(ColorAttachment::Output(LOAD_STORE)));
        assert!(plan.uses_msaa_color() && !plan.msaa_color_transient());
    }

    #[test]
    fn msaa_resolves_into_the_scene_colour_the_finish_pass_reads() {
        let plan = FramePlan::new(PassSet::default(), 4);
        assert_eq!(
            pass(&plan, PassId::Transparent).color,
            Some(ColorAttachment::Msaa {
                ops: LOAD_DISCARD,
                resolve: true
            })
        );
        assert_eq!(
            pass(&plan, PassId::Post).color,
            Some(ColorAttachment::Output(CLEAR_STORE))
        );
    }

    #[test]
    fn single_msaa_pass_is_transient() {
        let plan = FramePlan::new(PassSet::EMPTY.with(PassId::Opaque), 4);
        assert_eq!(
            pass(&plan, PassId::Opaque).color,
            Some(ColorAttachment::Msaa {
                ops: CLEAR_DISCARD,
                resolve: true
            })
        );
        assert!(plan.msaa_color_transient() && plan.depth_transient());
    }

    #[test]
    fn merged_scene_passes_keep_depth_and_msaa_in_one_pass() {
        let plan = FramePlan::new(PassSet::default().with(PassId::Scene), 4);
        assert_eq!(names(&plan), ["scene", "bloom", "post", "ui"]);
        let scene = pass(&plan, PassId::Scene);
        assert_eq!(
            scene.runs,
            PassSet::EMPTY
                .with(PassId::Opaque)
                .with(PassId::Sky)
                .with(PassId::Transparent)
        );
        // Cleared, resolved into the scene colour and discarded, all inside the one pass: both can be transient.
        assert_eq!(
            scene.color,
            Some(ColorAttachment::Msaa {
                ops: CLEAR_DISCARD,
                resolve: true
            })
        );
        assert_eq!(scene.depth, Some(DepthOps::Write(CLEAR_DISCARD)));
        assert!(plan.depth_transient() && plan.msaa_color_transient());
        assert_eq!(plan.full_screen_passes(), 3);
        assert!(!plan.contains(PassId::Opaque) && plan.contains(PassId::Scene));
        // Without MSAA the merged pass writes the scene colour itself.
        let plan = FramePlan::new(PassSet::default().with(PassId::Scene), 1);
        assert_eq!(
            pass(&plan, PassId::Scene).color,
            Some(ColorAttachment::Scene(CLEAR_STORE))
        );
        // With a prepass it merges that too.
        let plan = FramePlan::new(PassSet::default().with(PassId::Scene).with(PassId::DepthPrepass), 1);
        assert!(pass(&plan, PassId::Scene).runs.contains(PassId::DepthPrepass));
    }

    #[test]
    fn effects_follow_the_chain_and_need_the_finish_pass() {
        let set = PassSet::default().with(PassId::Mlaa);
        let reordered = PostChain::DEFAULT.push(PassId::Mlaa);
        assert_eq!(reordered.effects(), [PassId::Bloom, PassId::Mlaa]);
        assert_eq!(
            names(&FramePlan::with_chain(set, reordered, 1)),
            ["opaque", "sky", "transparent", "bloom", "mlaa", "post", "ui"]
        );
        // An effect left out of the chain still runs, after the chain's.
        let only_bloom = PostChain::EMPTY.push(PassId::Bloom);
        assert_eq!(
            names(&FramePlan::with_chain(set, only_bloom, 1)),
            ["opaque", "sky", "transparent", "bloom", "mlaa", "post", "ui"]
        );
        // Not effects: ignored.
        assert_eq!(PostChain::EMPTY.push(PassId::Ui).effects(), []);
        // Without the finish pass nothing consumes them.
        assert_eq!(
            names(&FramePlan::new(set.without(PassId::Post), 1)),
            ["opaque", "sky", "transparent", "ui"]
        );
    }

    #[test]
    fn a_post_frame_without_a_scene_has_nothing_to_finish() {
        let plan = FramePlan::new(
            PassSet::EMPTY.with(PassId::Post).with(PassId::Bloom).with(PassId::Ui),
            4,
        );
        assert_eq!(names(&plan), ["ui"]);
        assert_eq!(
            pass(&plan, PassId::Ui).color,
            Some(ColorAttachment::Output(CLEAR_STORE))
        );
        assert!(!plan.uses_depth() && !plan.uses_msaa_color() && !plan.post());
    }

    #[test]
    fn a_scaled_frame_draws_the_scene_into_the_scene_colour_and_upscales_it() {
        let plan = FramePlan::new(direct().with(PassId::Upscale), 1);
        assert!(plan.scaled());
        assert_eq!(names(&plan), ["opaque", "sky", "transparent", "upscale", "ui"]);
        assert_eq!(
            pass(&plan, PassId::Opaque).color,
            Some(ColorAttachment::Scene(CLEAR_STORE))
        );
        assert_eq!(pass(&plan, PassId::Sky).color, Some(ColorAttachment::Scene(LOAD_STORE)));
        // The last scene pass still stores: the upscale pass reads it.
        assert_eq!(
            pass(&plan, PassId::Transparent).color,
            Some(ColorAttachment::Scene(LOAD_STORE))
        );
        assert_eq!(pass(&plan, PassId::Transparent).depth, Some(DepthOps::ReadOnly));
        // Nothing wrote the output before the upscale, which covers all of it.
        let upscale = pass(&plan, PassId::Upscale);
        assert_eq!(upscale.color, Some(ColorAttachment::Output(CLEAR_STORE)));
        assert_eq!(upscale.depth, None);
        assert_eq!(pass(&plan, PassId::Ui).color, Some(ColorAttachment::Output(LOAD_STORE)));
    }

    #[test]
    fn a_scaled_frame_with_the_finish_pass_upscales_there() {
        let plan = FramePlan::new(PassSet::default().with(PassId::Upscale), 4);
        assert!(plan.scaled() && plan.post());
        assert_eq!(names(&plan), ["opaque", "sky", "transparent", "bloom", "post", "ui"]);
        assert_eq!(
            pass(&plan, PassId::Transparent).color,
            Some(ColorAttachment::Msaa {
                ops: LOAD_DISCARD,
                resolve: true
            })
        );
        assert_eq!(
            pass(&plan, PassId::Post).color,
            Some(ColorAttachment::Output(CLEAR_STORE))
        );
    }

    #[test]
    fn upscale_without_a_scene_is_left_out() {
        let plan = FramePlan::new(PassSet::EMPTY.with(PassId::Upscale).with(PassId::Ui), 1);
        assert!(!plan.scaled());
        assert_eq!(plan.passes().len(), 1);
        assert_eq!(
            pass(&plan, PassId::Ui).color,
            Some(ColorAttachment::Output(CLEAR_STORE))
        );
    }

    #[test]
    fn empty_set_plans_nothing() {
        let plan = FramePlan::new(PassSet::EMPTY, 1);
        assert!(plan.passes().is_empty());
        assert!(PassSet::EMPTY.is_empty());
    }

    #[test]
    fn pass_set_edits() {
        let set = PassSet::EMPTY.with(PassId::Ui).with(PassId::Opaque);
        assert_eq!(set.iter().collect::<Vec<_>>(), [PassId::Opaque, PassId::Ui]);
        assert!(!set.without(PassId::Ui).contains(PassId::Ui));
        assert_eq!(format!("{set:?}"), r#"{"opaque", "ui"}"#);
        assert_eq!(PassSet::ALL.iter().count(), PassId::COUNT);
        assert!(set.with_if(PassId::Mlaa, true).contains(PassId::Mlaa));
        assert!(!set.with_if(PassId::Ui, false).contains(PassId::Ui));
    }
}
