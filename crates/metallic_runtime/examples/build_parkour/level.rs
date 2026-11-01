//! The Parkour level: floating platforms running along -Z from the spawn island to the finish,
//! about 770 studs long and rising from y = 0 to y = 66. Four sections, each with its own colours,
//! split by the three checkpoints and getting harder bit by bit:
//!
//! 1. Sandstone: the spawn island, five easy 4-stud jumps, four 3-stud steps, a zigzag of four
//!    pads, a plank bridge and three more hops, up to Checkpoint 1 (y = 14).
//! 2. Slate blue: the lava walk (floor tiles and lava strips; each strip leaves a safe lane that
//!    changes side), a moving platform over a 23-stud gap, two 3-stud-wide beams over a lava
//!    river and a second, mirrored lava walk, up to Checkpoint 2.
//! 3. Moss green: the spinning bar arena (jump the bar as it sweeps), ledges and a 2-stud beam,
//!    stepping stones over a lava pool, then a tower climb: fourteen pads spiralling up a column
//!    (a wide base catches anyone who slips), up to Checkpoint 3 (y = 55).
//! 4. Ice: four long jumps (5 to 7 studs; running helps), a second moving platform and a slalom
//!    of small pads, up to the Finish (y = 66).
//!
//! Sections 2 to 4 are written in their own coordinates and moved into place by [`SLATE_AT`],
//! [`MOSS_AT`] and [`ICE_AT`] (added to every part's Y and Z), so a section can grow without
//! renumbering the ones after it.
//!
//! Violet marks what you interact with: the spawn, the checkpoint flags (checkpoint pads are
//! silver and turn violet when reached) and the finish arch. Lava is `#c4501e` smooth plastic.
use crate::scripts;
use metallic_core::{Color3, CoreError, InstanceId, UDim2, Value, Vec3, World};

const fn rgb(r: f64, g: f64, b: f64) -> Color3 {
    Color3::new(r, g, b)
}

/// The colours and top material of one section.
#[derive(Clone, Copy)]
struct Palette {
    top: Color3,
    material: &'static str,
    rim: Color3,
    support: Color3,
}

const SAND: Palette = Palette {
    top: rgb(214.0, 178.0, 126.0),
    material: "Sand",
    rim: rgb(150.0, 104.0, 70.0),
    support: rgb(122.0, 88.0, 62.0),
};
const SLATE: Palette = Palette {
    top: rgb(118.0, 142.0, 184.0),
    material: "Concrete",
    rim: rgb(58.0, 72.0, 108.0),
    support: rgb(52.0, 64.0, 94.0),
};
const MOSS: Palette = Palette {
    top: rgb(120.0, 160.0, 92.0),
    material: "Grass",
    rim: rgb(70.0, 96.0, 56.0),
    support: rgb(60.0, 82.0, 50.0),
};
const ICE: Palette = Palette {
    top: rgb(176.0, 214.0, 224.0),
    material: "Ice",
    rim: rgb(70.0, 108.0, 128.0),
    support: rgb(58.0, 90.0, 108.0),
};

const GROUND: Color3 = rgb(40.0, 38.0, 46.0);
const TOWER: Color3 = rgb(50.0, 54.0, 70.0);
const TOWER_CAP: Color3 = rgb(70.0, 76.0, 96.0);
const VIOLET: Color3 = rgb(155.0, 130.0, 212.0);
const SILVER: Color3 = rgb(201.0, 204.0, 210.0);
const STEEL: Color3 = rgb(180.0, 184.0, 191.0);
const LAVA: Color3 = rgb(196.0, 80.0, 30.0);
const FINISH: Color3 = rgb(236.0, 234.0, 240.0);
const HUD_BACK: Color3 = rgb(28.0, 25.0, 34.0);
const HUD_TEXT: Color3 = rgb(231.0, 229.0, 236.0);

/// Top of the ground far below the course.
const GROUND_TOP: f64 = -60.0;
/// Characters below this height die (Workspace.FallenPartsDestroyHeight).
const FALL_HEIGHT: f64 = -30.0;

/// Where sections 2, 3 and 4 sit: (Y, Z) added to the coordinates written in their functions.
/// The route test (`tests/parkour_route.rs`) repeats these numbers.
const SLATE_AT: (f64, f64) = (0.0, -61.0);
const MOSS_AT: (f64, f64) = (0.0, -115.0);
const ICE_AT: (f64, f64) = (10.0, -115.0);

/// The tower climb (section 3 coordinates): column centre (x, z), pad distance from it, first pad
/// angle and step (degrees; 0 points to +Z), first pad top, rise per pad and pad count. The last
/// pad faces -Z, toward Checkpoint 3. The route test repeats these numbers.
const TOWER_CENTRE: (f64, f64) = (0.0, -450.0);
const TOWER_RADIUS: f64 = 8.0;
const TOWER_FIRST_ANGLE: f64 = 250.0;
const TOWER_ANGLE_STEP: f64 = 50.0;
const TOWER_FIRST_TOP: f64 = 22.0;
const TOWER_RISE: f64 = 2.5;
const TOWER_PADS: usize = 14;

/// Builds the whole scene.
pub fn build() -> Result<World, CoreError> {
    let mut b = Builder::new()?;
    b.level()?;
    b.gui()?;
    Ok(b.w)
}

struct Builder {
    w: World,
    ws: InstanceId,
    course: InstanceId,
    scenery: InstanceId,
    lava: InstanceId,
    /// (Y, Z) added to every part's position: the section being built.
    at: (f64, f64),
}

impl Builder {
    fn new() -> Result<Builder, CoreError> {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").ok_or(CoreError::InvalidInstance)?;
        w.set(ws, "FallenPartsDestroyHeight", Value::Number(FALL_HEIGHT))?;
        let course = folder(&mut w, ws, "Course")?;
        let scenery = folder(&mut w, ws, "Scenery")?;
        let lava = folder(&mut w, ws, "Lava")?;
        Ok(Builder {
            w,
            ws,
            course,
            scenery,
            lava,
            at: (0.0, 0.0),
        })
    }

    /// An anchored part at `pos` in the current section's coordinates.
    #[allow(clippy::too_many_arguments)]
    fn part(
        &mut self,
        class: &str,
        parent: InstanceId,
        name: &str,
        size: [f64; 3],
        pos: [f64; 3],
        color: Color3,
        material: &str,
    ) -> Result<InstanceId, CoreError> {
        let w = &mut self.w;
        let p = w.create(class, Some(parent))?;
        w.set(p, "Name", Value::String(name.into()))?;
        w.set(
            p,
            "Size",
            Value::Vector3(Vec3::new(size[0], size[1], size[2])),
        )?;
        w.set(
            p,
            "Position",
            Value::Vector3(Vec3::new(pos[0], pos[1] + self.at.0, pos[2] + self.at.1)),
        )?;
        w.set(p, "Color", Value::Color3(color))?;
        w.set(p, "Material", Value::String(material.into()))?;
        w.set(p, "Anchored", Value::Bool(true))?;
        Ok(p)
    }

    /// A 1-stud slab in the section's top colour with its walkable top at `top`, centred on
    /// (x, z), `sx` by `sz` studs.
    #[allow(clippy::too_many_arguments)]
    fn slab(
        &mut self,
        name: &str,
        pal: Palette,
        x: f64,
        top: f64,
        z: f64,
        sx: f64,
        sz: f64,
    ) -> Result<InstanceId, CoreError> {
        let course = self.course;
        self.part(
            "Part",
            course,
            name,
            [sx, 1.0, sz],
            [x, top - 0.5, z],
            pal.top,
            pal.material,
        )
    }

    /// The darker rim under a slab: slightly wider, showing as an edge 0.2 studs lower.
    #[allow(clippy::too_many_arguments)]
    fn rim(
        &mut self,
        name: &str,
        pal: Palette,
        x: f64,
        top: f64,
        z: f64,
        sx: f64,
        sz: f64,
    ) -> Result<(), CoreError> {
        let course = self.course;
        self.part(
            "Part",
            course,
            &format!("{name}Rim"),
            [sx + 0.6, 1.4, sz + 0.6],
            [x, top - 0.9, z],
            pal.rim,
            "SmoothPlastic",
        )?;
        Ok(())
    }

    /// A platform: slab, rim and a support column down to the ground. Returns the slab.
    #[allow(clippy::too_many_arguments)]
    fn platform(
        &mut self,
        name: &str,
        pal: Palette,
        x: f64,
        top: f64,
        z: f64,
        sx: f64,
        sz: f64,
    ) -> Result<InstanceId, CoreError> {
        let slab = self.slab(name, pal, x, top, z, sx, sz)?;
        self.rim(name, pal, x, top, z, sx, sz)?;
        self.support(pal, x, top - 1.6, z, (sx.min(sz) * 0.3).clamp(1.6, 4.0))?;
        Ok(slab)
    }

    /// A square support column from `from_y` (section coordinates) down into the ground.
    fn support(
        &mut self,
        pal: Palette,
        x: f64,
        from_y: f64,
        z: f64,
        width: f64,
    ) -> Result<(), CoreError> {
        let scenery = self.scenery;
        self.column(scenery, "Support", pal.support, x, from_y + 0.2, z, width)?;
        Ok(())
    }

    /// A square concrete column from `top` (section coordinates) down into the ground.
    #[allow(clippy::too_many_arguments)]
    fn column(
        &mut self,
        parent: InstanceId,
        name: &str,
        color: Color3,
        x: f64,
        top: f64,
        z: f64,
        width: f64,
    ) -> Result<InstanceId, CoreError> {
        // The ground does not move with the section, so work out the height in world Y.
        let bottom = GROUND_TOP - 1.0 - self.at.0;
        self.part(
            "Part",
            parent,
            name,
            [width, top - bottom, width],
            [x, (top + bottom) / 2.0, z],
            color,
            "Concrete",
        )
    }

    fn script(
        &mut self,
        class: &str,
        parent: InstanceId,
        name: &str,
        source: &str,
    ) -> Result<(), CoreError> {
        let s = self.w.create(class, Some(parent))?;
        self.w.set(s, "Name", Value::String(name.into()))?;
        self.w.set(s, "Source", Value::String(source.into()))?;
        Ok(())
    }

    /// A pole with a violet flag next to a checkpoint or the spawn.
    fn flag(&mut self, x: f64, top: f64, z: f64) -> Result<(), CoreError> {
        let scenery = self.scenery;
        self.part(
            "Part",
            scenery,
            "FlagPole",
            [0.4, 9.0, 0.4],
            [x, top + 4.5, z],
            SILVER,
            "SmoothPlastic",
        )?;
        self.part(
            "Part",
            scenery,
            "Flag",
            [0.2, 1.6, 2.6],
            [x, top + 8.0, z + 1.5],
            VIOLET,
            "SmoothPlastic",
        )?;
        Ok(())
    }

    /// Two pillars and a violet beam across the course at `z`.
    fn arch(
        &mut self,
        pillars: Color3,
        x: f64,
        top: f64,
        z: f64,
        half_width: f64,
    ) -> Result<(), CoreError> {
        let scenery = self.scenery;
        for side in [-1.0, 1.0] {
            self.part(
                "Part",
                scenery,
                "ArchPillar",
                [1.6, 11.0, 1.6],
                [x + side * half_width, top + 5.5, z],
                pillars,
                "SmoothPlastic",
            )?;
        }
        self.part(
            "Part",
            scenery,
            "ArchBeam",
            [half_width * 2.0 + 1.6, 1.4, 1.6],
            [x, top + 11.7, z],
            VIOLET,
            "SmoothPlastic",
        )?;
        Ok(())
    }

    /// An 8x8 platform moved back and forth along Z by its script, centred on (x, z) with its
    /// top at `top`.
    fn mover(&mut self, name: &str, x: f64, top: f64, z: f64) -> Result<(), CoreError> {
        let ws = self.ws;
        let mover = self.part(
            "Part",
            ws,
            name,
            [8.0, 1.0, 8.0],
            [x, top - 0.5, z],
            STEEL,
            "Metal",
        )?;
        self.script(
            "Script",
            mover,
            "MoveBackAndForth",
            scripts::MOVING_PLATFORM,
        )
    }

    fn level(&mut self) -> Result<(), CoreError> {
        let ws = self.ws;
        let scenery = self.scenery;
        self.part(
            "Part",
            scenery,
            "Ground",
            [1600.0, 4.0, 1600.0],
            [0.0, GROUND_TOP - 2.0, -300.0],
            GROUND,
            "Concrete",
        )?;
        let lava = self.lava;
        self.script("Script", lava, "KnockOutOnTouch", scripts::LAVA)?;
        let checkpoints = folder(&mut self.w, ws, "Checkpoints")?;
        self.script("Script", checkpoints, "ShowProgress", scripts::CHECKPOINTS)?;

        self.sandstone(checkpoints)?;
        self.at = SLATE_AT;
        self.slate(checkpoints)?;
        self.at = MOSS_AT;
        self.moss(checkpoints)?;
        self.at = ICE_AT;
        self.ice()?;
        self.at = (0.0, 0.0);
        self.towers()?;

        let storage = self
            .w
            .root("ReplicatedStorage")
            .ok_or(CoreError::InvalidInstance)?;
        for name in ["CheckpointReached", "Finished"] {
            let r = self.w.create("RemoteEvent", Some(storage))?;
            self.w.set(r, "Name", Value::String(name.into()))?;
        }
        Ok(())
    }

    /// Section 1: spawn island, easy jumps, steps and a zigzag up to Checkpoint 1.
    fn sandstone(&mut self, checkpoints: InstanceId) -> Result<(), CoreError> {
        let ws = self.ws;
        self.platform("SpawnIsland", SAND, 0.0, 0.0, 0.0, 28.0, 28.0)?;
        self.part(
            "SpawnLocation",
            ws,
            "SpawnLocation",
            [8.0, 0.4, 8.0],
            [0.0, 0.2, 4.0],
            VIOLET,
            "SmoothPlastic",
        )?;
        self.flag(-6.0, 0.0, 4.0)?;
        self.arch(SAND.rim, 0.0, 0.0, -12.0, 7.0)?;
        let jumps = [
            (0.0, 0.0, -22.0),
            (3.0, 0.0, -34.0),
            (-2.0, 2.0, -46.0),
            (2.0, 3.0, -58.0),
            (0.0, 5.0, -70.0),
        ];
        for (i, (x, top, z)) in jumps.into_iter().enumerate() {
            self.platform(&format!("Jump{}", i + 1), SAND, x, top, z, 8.0, 8.0)?;
        }
        let steps = [(6.0, -80.0), (8.0, -89.0), (9.0, -98.0), (11.0, -107.0)];
        for (i, (top, z)) in steps.into_iter().enumerate() {
            self.platform(&format!("Step{}", i + 1), SAND, 0.0, top, z, 6.0, 6.0)?;
        }
        let zigzag = [
            (-5.0, 11.0, -118.0),
            (3.0, 12.0, -129.0),
            (-3.0, 12.0, -141.0),
            (2.0, 14.0, -152.0),
        ];
        for (i, (x, top, z)) in zigzag.into_iter().enumerate() {
            self.platform(&format!("Zigzag{}", i + 1), SAND, x, top, z, 6.0, 6.0)?;
        }
        self.platform("PlankBridge", SAND, 0.0, 14.0, -170.0, 4.0, 26.0)?;
        let hops = [
            (-4.0, 15.0, -190.0),
            (3.0, 15.0, -201.0),
            (-2.0, 14.0, -212.0),
        ];
        for (i, (x, top, z)) in hops.into_iter().enumerate() {
            self.platform(&format!("Hop{}", i + 1), SAND, x, top, z, 6.0, 6.0)?;
        }
        self.checkpoint(checkpoints, SAND, 1, 0.0, 14.0, -228.0)
    }

    /// Section 2: the lava walk, the first moving platform, the beams over the lava river and the
    /// mirrored second lava walk, up to Checkpoint 2.
    fn slate(&mut self, checkpoints: InstanceId) -> Result<(), CoreError> {
        self.lava_walk(14.0, -175.0, &FIRST_WALK)?;
        // The mover crosses the 23-stud gap from the walk's end (z -229) to the dock (z -252),
        // stopping 0.5 studs from each.
        self.mover("MovingPlatform", 0.0, 14.0, -240.5)?;
        self.platform("Dock", SLATE, 0.0, 14.0, -258.0, 12.0, 12.0)?;
        self.lava_river()?;
        self.lava_walk(14.0, -300.0, &SECOND_WALK)?;
        self.checkpoint(checkpoints, SLATE, 2, 0.0, 14.0, -362.0)
    }

    /// Section 3: the spinning bar arena, ledges, a beam, stepping stones over lava and the
    /// tower climb, up to Checkpoint 3.
    fn moss(&mut self, checkpoints: InstanceId) -> Result<(), CoreError> {
        let (ws, scenery) = (self.ws, self.scenery);
        self.platform("Arena", MOSS, 0.0, 14.0, -334.0, 30.0, 30.0)?;
        self.part(
            "Part",
            scenery,
            "BarHub",
            [3.0, 5.0, 3.0],
            [0.0, 16.5, -334.0],
            MOSS.rim,
            "SmoothPlastic",
        )?;
        let bar = self.part(
            "Part",
            ws,
            "SpinningBar",
            [28.0, 1.2, 1.2],
            [0.0, 15.4, -334.0],
            STEEL,
            "Metal",
        )?;
        self.script("Script", bar, "Spin", scripts::SPINNING_BAR)?;
        self.platform("Ledge1", MOSS, 4.0, 15.0, -356.0, 6.0, 6.0)?;
        self.platform("Ledge2", MOSS, -3.0, 17.0, -367.0, 6.0, 6.0)?;
        self.platform("Beam", MOSS, -3.0, 18.0, -385.0, 2.0, 20.0)?;
        self.platform("Ledge3", MOSS, -3.0, 18.0, -403.0, 6.0, 6.0)?;
        self.lava_pool()?;
        self.tower_climb()?;
        self.checkpoint(checkpoints, MOSS, 3, 0.0, 55.0, -474.0)
    }

    /// Section 4: long jumps, the second moving platform and a slalom up to the Finish.
    fn ice(&mut self) -> Result<(), CoreError> {
        let ws = self.ws;
        let long_jumps = [
            (4.0, 46.0, -490.0),
            (-2.0, 48.0, -502.0),
            (3.0, 49.0, -515.0),
            (-1.0, 50.0, -528.0),
        ];
        for (i, (x, top, z)) in long_jumps.into_iter().enumerate() {
            self.platform(&format!("LongJump{}", i + 1), ICE, x, top, z, 6.0, 6.0)?;
        }
        self.platform("Dock2", ICE, 0.0, 50.0, -540.0, 10.0, 10.0)?;
        self.mover("MovingPlatform2", 0.0, 50.0, -556.5)?;
        self.platform("Dock3", ICE, 0.0, 50.0, -573.0, 10.0, 10.0)?;
        let slalom = [
            (4.0, 51.0, -585.0),
            (-4.0, 52.0, -594.0),
            (4.0, 53.0, -603.0),
            (-4.0, 54.0, -612.0),
            (0.0, 55.0, -621.0),
        ];
        for (i, (x, top, z)) in slalom.into_iter().enumerate() {
            self.platform(&format!("Slalom{}", i + 1), ICE, x, top, z, 5.0, 5.0)?;
        }
        self.platform("FinishIsland", ICE, 0.0, 56.0, -636.0, 20.0, 20.0)?;
        let finish = self.part(
            "Part",
            ws,
            "Finish",
            [10.0, 0.4, 10.0],
            [0.0, 56.2, -638.0],
            FINISH,
            "SmoothPlastic",
        )?;
        self.script("Script", finish, "FinishLine", scripts::FINISH)?;
        self.arch(ICE.rim, 0.0, 56.0, -638.0, 7.0)
    }

    /// A 16x16 platform with a silver checkpoint pad in the middle and a flag.
    #[allow(clippy::too_many_arguments)]
    fn checkpoint(
        &mut self,
        folder: InstanceId,
        pal: Palette,
        order: u32,
        x: f64,
        top: f64,
        z: f64,
    ) -> Result<(), CoreError> {
        self.platform(
            &format!("Checkpoint{order}Island"),
            pal,
            x,
            top,
            z,
            16.0,
            16.0,
        )?;
        let cp = self.part(
            "Checkpoint",
            folder,
            &format!("Checkpoint{order}"),
            [8.0, 0.4, 8.0],
            [x, top + 0.2, z],
            SILVER,
            "SmoothPlastic",
        )?;
        self.w.set(cp, "Order", Value::Number(f64::from(order)))?;
        self.flag(x - 6.0, top, z - 5.0)
    }

    /// Floor tiles with lava strips between them, 54 studs long from `z0` toward -Z. Each of the
    /// `rows` leaves safe lanes that move from side to side.
    fn lava_walk(&mut self, top: f64, z0: f64, rows: &[LavaRow; 4]) -> Result<(), CoreError> {
        let half = 7.0;
        let course = self.course;
        // Under-slab: the dark rim along the whole walk.
        self.part(
            "Part",
            course,
            "LavaWalkRim",
            [half * 2.0 + 0.6, 1.4, 54.6],
            [0.0, top - 0.9, z0 - 27.0],
            SLATE.rim,
            "SmoothPlastic",
        )?;
        self.support(SLATE, -4.0, top - 1.6, z0 - 11.0, 3.0)?;
        self.support(SLATE, 4.0, top - 1.6, z0 - 43.0, 3.0)?;
        let mut z = z0;
        for (i, (offset, len, safe)) in rows.iter().enumerate() {
            let start = z0 - offset;
            let name = format!("LavaWalkFloor{}", i + 1);
            self.tile(&name, top, (-half, half), (z, start))?;
            // Lava fills the row except the safe lanes, which are floor.
            let mut x = -half;
            for &(a, b) in safe.iter() {
                if a > x {
                    self.lava_strip(top, (x, a), (start, start - len))?;
                }
                self.tile("SafeLane", top, (a, b), (start, start - len))?;
                x = b;
            }
            if x < half {
                self.lava_strip(top, (x, half), (start, start - len))?;
            }
            z = start - len;
        }
        self.tile("LavaWalkFloor5", top, (-half, half), (z, z0 - 54.0))
    }

    /// A lava-walk floor tile covering `xs` and `zs` (from, to; z from > z to) with its top at
    /// `top`.
    fn tile(
        &mut self,
        name: &str,
        top: f64,
        xs: (f64, f64),
        zs: (f64, f64),
    ) -> Result<(), CoreError> {
        let (x, z) = ((xs.0 + xs.1) / 2.0, (zs.0 + zs.1) / 2.0);
        self.slab(name, SLATE, x, top, z, xs.1 - xs.0, zs.0 - zs.1)?;
        Ok(())
    }

    /// A lava strip covering `xs` and `zs` (as for [`Builder::tile`]), flush with the floor top.
    fn lava_strip(&mut self, top: f64, xs: (f64, f64), zs: (f64, f64)) -> Result<(), CoreError> {
        let lava = self.lava;
        self.part(
            "Part",
            lava,
            "Lava",
            [xs.1 - xs.0, 1.0, zs.0 - zs.1],
            [(xs.0 + xs.1) / 2.0, top - 0.5, (zs.0 + zs.1) / 2.0],
            LAVA,
            "SmoothPlastic",
        )?;
        Ok(())
    }

    /// The lava river (z -264 to -300, 2 studs below the path) crossed on two 3-stud beams with
    /// a resting pad between them; the second beam is offset, so you turn on the pad.
    fn lava_river(&mut self) -> Result<(), CoreError> {
        let (lava, course) = (self.lava, self.course);
        self.part(
            "Part",
            lava,
            "LavaRiver",
            [16.0, 1.0, 36.0],
            [0.0, 11.5, -282.0],
            LAVA,
            "SmoothPlastic",
        )?;
        self.part(
            "Part",
            course,
            "LavaRiverRim",
            [16.6, 1.4, 36.6],
            [0.0, 11.1, -282.0],
            SLATE.rim,
            "SmoothPlastic",
        )?;
        self.support(SLATE, 0.0, 10.4, -282.0, 4.0)?;
        self.slab("RiverBeam1", SLATE, -2.0, 14.0, -271.0, 3.0, 14.0)?;
        self.slab("RiverRest", SLATE, 0.0, 14.0, -281.0, 6.0, 6.0)?;
        self.slab("RiverBeam2", SLATE, 2.0, 14.0, -292.0, 3.0, 16.0)?;
        Ok(())
    }

    /// A lava pool (z -410 to -434, top y = 19) crossed on four stepping stones (top y = 20).
    fn lava_pool(&mut self) -> Result<(), CoreError> {
        let (lava, course) = (self.lava, self.course);
        self.part(
            "Part",
            lava,
            "LavaPool",
            [16.0, 1.0, 24.0],
            [0.0, 18.5, -422.0],
            LAVA,
            "SmoothPlastic",
        )?;
        self.part(
            "Part",
            course,
            "LavaPoolRim",
            [16.6, 1.4, 24.6],
            [0.0, 18.1, -422.0],
            MOSS.rim,
            "SmoothPlastic",
        )?;
        self.support(MOSS, 0.0, 17.4, -422.0, 4.0)?;
        for (i, (x, z)) in [(-3.0, -413.0), (2.0, -419.0), (-2.0, -425.0), (3.0, -431.0)]
            .into_iter()
            .enumerate()
        {
            self.part(
                "Part",
                course,
                &format!("SteppingStone{}", i + 1),
                [4.0, 2.5, 4.0],
                [x, 18.75, z],
                MOSS.top,
                MOSS.material,
            )?;
        }
        Ok(())
    }

    /// The tower climb: a wide base (top y = 20) around a column with fourteen 5x5 pads spiralling
    /// up it, 2.5 studs apart in height. A slip lands back on the base instead of in the void.
    fn tower_climb(&mut self) -> Result<(), CoreError> {
        let (cx, cz) = TOWER_CENTRE;
        let course = self.course;
        self.slab("TowerBase", MOSS, cx, 20.0, cz, 26.0, 26.0)?;
        self.rim("TowerBase", MOSS, cx, 20.0, cz, 26.0, 26.0)?;
        let top = tower_pad(TOWER_PADS - 1).1 + 2.0;
        self.column(course, "TowerColumn", MOSS.support, cx, top, cz, 6.0)?;
        for i in 0..TOWER_PADS {
            let (x, top, z) = tower_pad(i);
            self.part(
                "Part",
                course,
                &format!("TowerPad{}", i + 1),
                [5.0, 1.0, 5.0],
                [x, top - 0.5, z],
                MOSS.top,
                MOSS.material,
            )?;
        }
        Ok(())
    }

    /// Graphite towers on both sides of the course: background silhouettes.
    fn towers(&mut self) -> Result<(), CoreError> {
        let scenery = self.scenery;
        // A small fixed pseudo-random sequence so the layout never changes between builds.
        let mut seed: u32 = 20261030;
        let mut next = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f64::from(seed >> 8) / f64::from(1u32 << 24)
        };
        for i in 0..36 {
            let side = if i % 2 == 0 { -1.0 } else { 1.0 };
            let x = side * (60.0 + next() * 70.0);
            let z = 40.0 - f64::from(i) * 23.0 - next() * 10.0;
            let width = 6.0 + next() * 8.0;
            // Towers grow with the course, which climbs about 7 studs per 100.
            let top = -25.0 + next() * 60.0 - z * 0.07;
            let height = top - (GROUND_TOP - 1.0);
            self.part(
                "Part",
                scenery,
                "Tower",
                [width, height, width],
                [x, GROUND_TOP - 1.0 + height / 2.0, z],
                TOWER,
                "Concrete",
            )?;
            self.part(
                "Part",
                scenery,
                "TowerCap",
                [width + 1.0, 1.2, width + 1.0],
                [x, top + 0.4, z],
                TOWER_CAP,
                "SmoothPlastic",
            )?;
        }
        Ok(())
    }

    /// The HUD (StarterGui, copied into the player's PlayerGui when the game starts).
    fn gui(&mut self) -> Result<(), CoreError> {
        let w = &mut self.w;
        let starter = w.root("StarterGui").ok_or(CoreError::InvalidInstance)?;
        let hud = w.create("ScreenGui", Some(starter))?;
        w.set(hud, "Name", Value::String("HUD".into()))?;
        let banner = w.create("Frame", Some(hud))?;
        w.set(banner, "Name", Value::String("Banner".into()))?;
        w.set(
            banner,
            "Position",
            Value::UDim2(UDim2::new(0.35, 0.0, 0.03, 0.0)),
        )?;
        w.set(
            banner,
            "Size",
            Value::UDim2(UDim2::new(0.3, 0.0, 0.06, 0.0)),
        )?;
        w.set(banner, "BackgroundColor3", Value::Color3(HUD_BACK))?;
        w.set(banner, "BackgroundTransparency", Value::Number(0.2))?;
        let accent = w.create("Frame", Some(banner))?;
        w.set(accent, "Name", Value::String("Accent".into()))?;
        w.set(
            accent,
            "Position",
            Value::UDim2(UDim2::new(0.0, 0.0, 1.0, -3.0)),
        )?;
        w.set(accent, "Size", Value::UDim2(UDim2::new(1.0, 0.0, 0.0, 3.0)))?;
        w.set(accent, "BackgroundColor3", Value::Color3(VIOLET))?;
        let message = w.create("TextLabel", Some(banner))?;
        w.set(message, "Name", Value::String("Message".into()))?;
        w.set(
            message,
            "Size",
            Value::UDim2(UDim2::new(1.0, 0.0, 1.0, -3.0)),
        )?;
        w.set(message, "BackgroundTransparency", Value::Number(1.0))?;
        w.set(message, "TextColor3", Value::Color3(HUD_TEXT))?;
        w.set(message, "TextSize", Value::Number(28.0))?;
        // No controls text here: the player's own controls plate shows the keys in the player's
        // bindings, bottom left (#480).
        self.script("LocalScript", hud, "ShowStatus", scripts::HUD)
    }
}

/// One lava strip across the walk: where it starts (studs from the walk's start), its length, and
/// the safe lanes (x from, x to).
type LavaRow = (f64, f64, &'static [(f64, f64)]);

/// The first lava walk: the safe lane goes right, left, middle, then either side.
const FIRST_WALK: [LavaRow; 4] = [
    (8.0, 4.0, &[(3.0, 7.0)]),
    (20.0, 4.0, &[(-7.0, -3.0)]),
    (32.0, 4.0, &[(-2.0, 2.0)]),
    (44.0, 5.0, &[(-7.0, -3.5), (3.5, 7.0)]),
];

/// The second lava walk: the first one mirrored left to right.
const SECOND_WALK: [LavaRow; 4] = [
    (8.0, 4.0, &[(-7.0, -3.0)]),
    (20.0, 4.0, &[(3.0, 7.0)]),
    (32.0, 4.0, &[(-2.0, 2.0)]),
    (44.0, 5.0, &[(-7.0, -3.5), (3.5, 7.0)]),
];

/// Centre x, top y and centre z of tower pad `i` (0 is the lowest).
fn tower_pad(i: usize) -> (f64, f64, f64) {
    let (cx, cz) = TOWER_CENTRE;
    let angle = (TOWER_FIRST_ANGLE + TOWER_ANGLE_STEP * i as f64).to_radians();
    (
        cx + TOWER_RADIUS * angle.sin(),
        // Whole-stud tops, so 1-stud pads have their faces on the grid (spec 2.2): the rise
        // alternates 3 and 2 studs (2.5 on average).
        (TOWER_FIRST_TOP + TOWER_RISE * i as f64).ceil(),
        cz + TOWER_RADIUS * angle.cos(),
    )
}

fn folder(w: &mut World, parent: InstanceId, name: &str) -> Result<InstanceId, CoreError> {
    let f = w.create("Folder", Some(parent))?;
    w.set(f, "Name", Value::String(name.into()))?;
    Ok(f)
}
