//! The frame-graph-lite: a fixed, ordered list of passes with explicit attachments.
//!
//! A frame is at most these passes, always in this order: depth prepass, opaque, sky, transparent, post, upscale,
//! UI. The sky
//! comes after the opaque scene, drawn at the far plane with a depth test, so it only shades pixels nothing else
//! covered (ADR 0008 §7). Which passes run is a [`PassSet`]; disabled passes cost nothing.
//!
//! [`FramePlan`] works out every attachment's load and store operations from the passes that run, once, when the
//! pass set or MSAA changes (never per frame):
//!
//! - the first pass that touches an attachment clears it, later ones load it;
//! - a pass stores an attachment only when a later pass reads it; otherwise the result is discarded, which on
//!   tile-based (mobile) GPUs saves writing the tile back to memory;
//! - with MSAA the multisampled colour is resolved into the output inside its last pass and then discarded;
//! - an attachment only one pass touches can be transient (memoryless on tile-based GPUs).
//!
//! **Render scale.** With [`PassId::Upscale`] in the set (the renderer plans it when the scene is drawn at a render
//! scale, [`crate::resolution`]), the scene passes and post draw into the *scene colour* ([`ColorAttachment::Scene`]),
//! a texture at the render scale, and the upscale pass draws it over the whole output (cleared first: nothing before
//! it wrote there). UI then draws on the output at its full resolution. With MSAA the multisampled colour resolves
//! into the scene colour instead of the output.
//!
//! The scene passes (prepass, opaque, sky, transparent) draw into the scene colour and the depth buffer; post and
//! UI draw into the output. Until the post chain exists the scene colour *is* the output (or its MSAA source), so
//! post draws over it in place; the post-chain issue gives the scene its own HDR target, and only this plan changes.
//! Each pass is one wgpu render pass for now. Merging consecutive scene passes into one render pass (so depth can
//! stay in tile memory on mobile) is a later optimisation behind the same plan.

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
    /// The post chain (tone mapping, bloom, grading).
    Post,
    /// Draws the scene colour, rendered at the render scale, over the whole output (a bilinear upscale; see
    /// [`crate::resolution`]). Only planned when the scene is drawn at a render scale.
    Upscale,
    /// Screen-space UI.
    Ui,
}

impl PassId {
    /// Every pass, in frame order.
    pub const ALL: [PassId; 7] = [
        PassId::DepthPrepass,
        PassId::Opaque,
        PassId::Sky,
        PassId::Transparent,
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
            PassId::Post => "post",
            PassId::Upscale => "upscale",
            PassId::Ui => "ui",
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }

    /// Whether the pass draws into the scene colour; `false` for the depth-only prepass and for the output passes.
    const fn draws_scene_color(self) -> bool {
        matches!(self, PassId::Opaque | PassId::Sky | PassId::Transparent)
    }

    /// Whether the pass draws into the output (after the scene). With a render scale post draws into the scene
    /// colour instead ([`FramePlan::new`]).
    const fn draws_output(self) -> bool {
        matches!(self, PassId::Post | PassId::Upscale | PassId::Ui)
    }

    /// How the pass uses depth, if at all, given whether a depth prepass runs.
    const fn depth_use(self, prepass: bool) -> Option<DepthUse> {
        match self {
            PassId::DepthPrepass => Some(DepthUse::Write),
            // After a prepass the depth is final; opaque shading only tests it (`Equal`/`GreaterEqual`).
            PassId::Opaque if prepass => Some(DepthUse::Test),
            PassId::Opaque => Some(DepthUse::Write),
            PassId::Sky | PassId::Transparent => Some(DepthUse::Test),
            PassId::Post | PassId::Upscale | PassId::Ui => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DepthUse {
    Write,
    Test,
}

/// A set of passes. The default is opaque, sky, transparent and UI.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PassSet(u8);

impl PassSet {
    /// No passes.
    pub const EMPTY: PassSet = PassSet(0);

    /// Every pass.
    pub const ALL: PassSet = PassSet((1 << PassId::COUNT) - 1);

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
}

impl Default for PassSet {
    fn default() -> Self {
        PassSet::EMPTY
            .with(PassId::Opaque)
            .with(PassId::Sky)
            .with(PassId::Transparent)
            .with(PassId::Ui)
    }
}

impl fmt::Debug for PassSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter().map(PassId::name)).finish()
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
    /// The scene colour at the render scale, which the upscale pass reads ([`PassId::Upscale`]).
    Scene(AttachmentOps),
    /// The multisampled scene colour. `resolve` resolves it into the output (with a render scale, into the scene
    /// colour) at the end of the pass.
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
    /// The colour attachment; `None` for a depth-only pass.
    pub color: Option<ColorAttachment>,
    /// The depth attachment; `None` when the pass does not use depth.
    pub depth: Option<DepthOps>,
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
}

impl FramePlan {
    /// Plans the frame for `passes`, with the scene drawn at `msaa_samples` per pixel (1 for no MSAA). With
    /// [`PassId::Upscale`] in `passes` the scene is drawn into the scene colour and upscaled; it is left out when no
    /// pass draws the scene.
    pub fn new(passes: PassSet, msaa_samples: u32) -> Self {
        let msaa = msaa_samples > 1;
        let prepass = passes.contains(PassId::DepthPrepass);
        let scaled = passes.contains(PassId::Upscale) && passes.iter().any(PassId::draws_scene_color);
        let passes = if scaled {
            passes
        } else {
            passes.without(PassId::Upscale)
        };
        let placeholder = PlannedPass {
            id: PassId::Opaque,
            color: None,
            depth: None,
        };
        let mut plan = Self {
            passes: [placeholder; PassId::COUNT],
            len: 0,
            msaa_samples: msaa_samples.max(1),
            depth_uses: 0,
            msaa_uses: 0,
            scaled,
        };

        // Which pass indices (bits) use each attachment. Resolving into the output (or the scene colour) counts as
        // using it, and so does the upscale pass reading the scene colour.
        let (mut depth_users, mut msaa_users, mut output_users, mut scene_users) = (0u8, 0u8, 0u8, 0u8);
        for (i, pass) in passes.iter().enumerate() {
            let bit = 1 << i;
            if pass.depth_use(prepass).is_some() {
                depth_users |= bit;
            }
            if pass.draws_scene_color() {
                if msaa {
                    msaa_users |= bit;
                } else if scaled {
                    scene_users |= bit;
                } else {
                    output_users |= bit;
                }
            }
            if scaled && matches!(pass, PassId::Post | PassId::Upscale) {
                scene_users |= bit;
            }
            if pass.draws_output() && !(scaled && pass == PassId::Post) {
                output_users |= bit;
            }
        }
        // The last multisampled pass resolves into the output, or the scene colour.
        let resolver = last_index(msaa_users);
        if let Some(r) = resolver {
            if scaled {
                scene_users |= 1 << r;
            } else {
                output_users |= 1 << r;
            }
        }

        for (i, pass) in passes.iter().enumerate() {
            let ops = |users: u8| AttachmentOps {
                clear: first_index(users) == Some(i),
                store: last_index(users) != Some(i),
            };
            let color = if pass.draws_scene_color() && msaa {
                Some(ColorAttachment::Msaa {
                    ops: ops(msaa_users),
                    resolve: resolver == Some(i),
                })
            } else if scaled && (pass.draws_scene_color() || pass == PassId::Post) {
                // Every writer stores: the upscale pass, the scene colour's last user, reads it.
                Some(ColorAttachment::Scene(ops(scene_users)))
            } else if pass.draws_scene_color() || pass.draws_output() {
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
            plan.passes[i] = PlannedPass { id: pass, color, depth };
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

    /// Whether the scene is drawn into the scene colour at a render scale and upscaled into the output
    /// ([`PassId::Upscale`]).
    pub fn scaled(&self) -> bool {
        self.scaled
    }
}

fn first_index(bits: u8) -> Option<usize> {
    (bits != 0).then(|| bits.trailing_zeros() as usize)
}

fn last_index(bits: u8) -> Option<usize> {
    (bits != 0).then(|| 7 - bits.leading_zeros() as usize)
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

    fn pass(plan: &FramePlan, id: PassId) -> PlannedPass {
        *plan.passes().iter().find(|p| p.id == id).expect("pass is planned")
    }

    #[test]
    fn passes_run_in_frame_order() {
        let plan = FramePlan::new(PassSet::ALL, 1);
        let order: Vec<_> = plan.passes().iter().map(|p| p.id).collect();
        assert_eq!(order, PassId::ALL);
        assert_eq!(
            FramePlan::new(PassSet::default(), 1)
                .passes()
                .iter()
                .map(|p| p.id.name())
                .collect::<Vec<_>>(),
            ["opaque", "sky", "transparent", "ui"]
        );
    }

    #[test]
    fn default_frame_clears_once_and_discards_depth_after_its_last_use() {
        let plan = FramePlan::new(PassSet::default(), 1);
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
        let plan = FramePlan::new(PassSet::default().with(PassId::DepthPrepass), 1);
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
        let plan = FramePlan::new(PassSet::default(), 4);
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
    fn output_only_frame_clears_in_the_first_output_pass() {
        let plan = FramePlan::new(PassSet::EMPTY.with(PassId::Post).with(PassId::Ui), 4);
        assert_eq!(
            pass(&plan, PassId::Post).color,
            Some(ColorAttachment::Output(CLEAR_STORE))
        );
        assert_eq!(pass(&plan, PassId::Ui).color, Some(ColorAttachment::Output(LOAD_STORE)));
        assert!(!plan.uses_depth() && !plan.uses_msaa_color());
    }

    #[test]
    fn a_scaled_frame_draws_the_scene_into_the_scene_colour_and_upscales_it() {
        let plan = FramePlan::new(PassSet::default().with(PassId::Upscale), 1);
        assert!(plan.scaled());
        let order: Vec<_> = plan.passes().iter().map(|p| p.id.name()).collect();
        assert_eq!(order, ["opaque", "sky", "transparent", "upscale", "ui"]);
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
    fn a_scaled_frame_with_msaa_resolves_into_the_scene_colour() {
        let plan = FramePlan::new(PassSet::default().with(PassId::Post).with(PassId::Upscale), 4);
        assert_eq!(
            pass(&plan, PassId::Transparent).color,
            Some(ColorAttachment::Msaa {
                ops: LOAD_DISCARD,
                resolve: true
            })
        );
        // Post draws over the resolved scene colour, at the render scale.
        assert_eq!(
            pass(&plan, PassId::Post).color,
            Some(ColorAttachment::Scene(LOAD_STORE))
        );
        assert_eq!(
            pass(&plan, PassId::Upscale).color,
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
    }
}
