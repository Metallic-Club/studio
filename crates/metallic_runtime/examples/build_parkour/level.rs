//! The Parkour level: a course of floating platforms running along -Z from the spawn island to the
//! finish, about 430 studs long and rising from y = 0 to y = 25.
//!
//! Sections, in order:
//! 1. Spawn island and five easy 4-stud gap jumps that climb gently.
//! 2. Three 3-stud steps up to Checkpoint 1 (y = 12).
//! 3. The lava walk: floor tiles and lava strips; each strip leaves a safe lane that changes side.
//! 4. A moving platform over a 23-stud gap, then Checkpoint 2.
//! 5. The spinning bar arena (jump the bar as it sweeps), then 5-stud jumps and a 2-stud-wide
//!    beam, then stepping stones over a lava pool, then Checkpoint 3 (y = 18.5).
//! 6. Four longer jumps (5 to 7 studs; running helps) up to the Finish (y = 25).
//!
//! Palette (sRGB): platform tops `#706c7a` concrete on graphite `#423e4c` rims, supports and
//! towers in graphite, spawn and accents violet `#9b82d4`, checkpoints silver `#c9ccd2` (they turn
//! violet when reached), lava `#c4501e` smooth plastic, finish `#e8e6ec`.
use crate::scripts;
use metallic_core::{Color3, CoreError, InstanceId, UDim2, Value, Vec3, World};

const fn rgb(r: f64, g: f64, b: f64) -> Color3 {
    Color3::new(r, g, b)
}

const TOP: Color3 = rgb(112.0, 108.0, 122.0);
const RIM: Color3 = rgb(66.0, 62.0, 76.0);
const SUPPORT: Color3 = rgb(62.0, 59.0, 71.0);
const GROUND: Color3 = rgb(40.0, 38.0, 46.0);
const TOWER: Color3 = rgb(70.0, 66.0, 80.0);
const TOWER_CAP: Color3 = rgb(104.0, 100.0, 116.0);
const VIOLET: Color3 = rgb(155.0, 130.0, 212.0);
const SILVER: Color3 = rgb(201.0, 204.0, 210.0);
const STEEL: Color3 = rgb(180.0, 184.0, 191.0);
const LAVA: Color3 = rgb(196.0, 80.0, 30.0);
const FINISH: Color3 = rgb(232.0, 230.0, 236.0);
const HUD_BACK: Color3 = rgb(28.0, 25.0, 34.0);
const HUD_TEXT: Color3 = rgb(231.0, 229.0, 236.0);
const HUD_DIM: Color3 = rgb(190.0, 187.0, 198.0);

/// Top of the ground far below the course.
const GROUND_TOP: f64 = -60.0;
/// Characters below this height die (Workspace.FallenPartsDestroyHeight).
const FALL_HEIGHT: f64 = -30.0;

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
}

impl Builder {
    fn new() -> Result<Builder, CoreError> {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").ok_or(CoreError::InvalidInstance)?;
        w.set(ws, "FallenPartsDestroyHeight", Value::Number(FALL_HEIGHT))?;
        let course = folder(&mut w, ws, "Course")?;
        let scenery = folder(&mut w, ws, "Scenery")?;
        Ok(Builder {
            w,
            ws,
            course,
            scenery,
        })
    }

    /// An anchored part.
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
            Value::Vector3(Vec3::new(pos[0], pos[1], pos[2])),
        )?;
        w.set(p, "Color", Value::Color3(color))?;
        w.set(p, "Material", Value::String(material.into()))?;
        w.set(p, "Anchored", Value::Bool(true))?;
        Ok(p)
    }

    /// A platform whose walkable top is at `top`, centred on (x, z): a 1-stud concrete slab on a
    /// slightly wider dark rim that shows as an edge 0.2 studs lower, and a support column down to
    /// the ground. Returns the top slab.
    fn platform(
        &mut self,
        name: &str,
        x: f64,
        top: f64,
        z: f64,
        sx: f64,
        sz: f64,
    ) -> Result<InstanceId, CoreError> {
        let course = self.course;
        let slab = self.part(
            "Part",
            course,
            name,
            [sx, 1.0, sz],
            [x, top - 0.5, z],
            TOP,
            "Concrete",
        )?;
        self.part(
            "Part",
            course,
            &format!("{name}Rim"),
            [sx + 0.6, 1.4, sz + 0.6],
            [x, top - 0.9, z],
            RIM,
            "SmoothPlastic",
        )?;
        self.support(x, top - 1.6, z, (sx.min(sz) * 0.3).clamp(1.6, 4.0))?;
        Ok(slab)
    }

    /// A square column from `from_y` down into the ground.
    fn support(&mut self, x: f64, from_y: f64, z: f64, width: f64) -> Result<(), CoreError> {
        let bottom = GROUND_TOP - 1.0;
        let height = from_y + 0.2 - bottom;
        let scenery = self.scenery;
        self.part(
            "Part",
            scenery,
            "Support",
            [width, height, width],
            [x, bottom + height / 2.0, z],
            SUPPORT,
            "Concrete",
        )?;
        Ok(())
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
    fn arch(&mut self, x: f64, top: f64, z: f64, half_width: f64) -> Result<(), CoreError> {
        let scenery = self.scenery;
        for side in [-1.0, 1.0] {
            self.part(
                "Part",
                scenery,
                "ArchPillar",
                [1.6, 11.0, 1.6],
                [x + side * half_width, top + 5.5, z],
                RIM,
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

    fn level(&mut self) -> Result<(), CoreError> {
        let ws = self.ws;
        let scenery = self.scenery;
        self.part(
            "Part",
            scenery,
            "Ground",
            [1600.0, 4.0, 1600.0],
            [0.0, GROUND_TOP - 2.0, -200.0],
            GROUND,
            "Concrete",
        )?;

        // 1. Spawn island and easy jumps.
        self.platform("SpawnIsland", 0.0, 0.0, 0.0, 28.0, 28.0)?;
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
        self.arch(0.0, 0.0, -12.0, 7.0)?;
        let easy = [
            (0.0, 0.0, -22.0),
            (3.0, 0.0, -34.0),
            (-2.0, 1.5, -46.0),
            (2.0, 3.0, -58.0),
            (0.0, 4.5, -70.0),
        ];
        for (i, (x, top, z)) in easy.into_iter().enumerate() {
            self.platform(&format!("Jump{}", i + 1), x, top, z, 8.0, 8.0)?;
        }

        // 2. Steps up to Checkpoint 1.
        for (i, (top, z)) in [(6.0, -80.0), (7.5, -89.0), (9.0, -98.0), (10.5, -107.0)]
            .into_iter()
            .enumerate()
        {
            self.platform(&format!("Step{}", i + 1), 0.0, top, z, 6.0, 6.0)?;
        }
        let checkpoints = folder(&mut self.w, ws, "Checkpoints")?;
        self.checkpoint(checkpoints, 1, 0.0, 12.0, -121.0)?;

        // 3. The lava walk.
        self.lava_walk()?;

        // 4. Moving platform over the 23-stud gap (it stops 0.5 studs from each dock), then
        // Checkpoint 2.
        let mover = self.part(
            "Part",
            ws,
            "MovingPlatform",
            [8.0, 1.0, 8.0],
            [0.0, 11.5, -194.5],
            STEEL,
            "Metal",
        )?;
        self.script(
            "Script",
            mover,
            "MoveBackAndForth",
            scripts::MOVING_PLATFORM,
        )?;
        self.checkpoint(checkpoints, 2, 0.0, 12.0, -214.0)?;

        // 5. Spinning bar arena, jumps, beam, lava pool, Checkpoint 3.
        self.platform("Arena", 0.0, 12.0, -240.0, 30.0, 30.0)?;
        self.part(
            "Part",
            scenery,
            "BarHub",
            [3.0, 5.0, 3.0],
            [0.0, 14.5, -240.0],
            RIM,
            "SmoothPlastic",
        )?;
        let bar = self.part(
            "Part",
            ws,
            "SpinningBar",
            [28.0, 1.2, 1.2],
            [0.0, 13.4, -240.0],
            STEEL,
            "Metal",
        )?;
        self.script("Script", bar, "Spin", scripts::SPINNING_BAR)?;
        self.platform("Ledge1", 4.0, 13.5, -262.0, 6.0, 6.0)?;
        self.platform("Ledge2", -3.0, 15.0, -273.0, 6.0, 6.0)?;
        self.platform("Beam", -3.0, 16.5, -291.0, 2.0, 20.0)?;
        self.platform("Ledge3", -3.0, 16.5, -309.0, 6.0, 6.0)?;
        self.lava_pool()?;
        self.checkpoint(checkpoints, 3, 0.0, 18.5, -350.0)?;

        // 6. Final jumps and the finish.
        let last = [
            (4.0, 20.0, -366.0),
            (-2.0, 21.5, -378.0),
            (3.0, 23.0, -391.0),
            (-1.0, 24.0, -404.0),
        ];
        for (i, (x, top, z)) in last.into_iter().enumerate() {
            self.platform(&format!("Final{}", i + 1), x, top, z, 6.0, 6.0)?;
        }
        self.platform("FinishIsland", 0.0, 25.0, -422.0, 20.0, 20.0)?;
        let finish = self.part(
            "Part",
            ws,
            "Finish",
            [10.0, 0.4, 10.0],
            [0.0, 25.2, -424.0],
            FINISH,
            "SmoothPlastic",
        )?;
        self.script("Script", finish, "FinishLine", scripts::FINISH)?;
        self.arch(0.0, 25.0, -424.0, 7.0)?;

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

    /// A 16x16 platform with a silver checkpoint pad in the middle and a flag.
    fn checkpoint(
        &mut self,
        folder: InstanceId,
        order: u32,
        x: f64,
        top: f64,
        z: f64,
    ) -> Result<(), CoreError> {
        self.platform(&format!("Checkpoint{order}Island"), x, top, z, 16.0, 16.0)?;
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
        if order == 1 {
            self.script("Script", folder, "ShowProgress", scripts::CHECKPOINTS)?;
        }
        self.flag(x - 6.0, top, z - 5.0)
    }

    /// Floor tiles with lava strips between them (z from -129 to -183, top y = 12). Each strip
    /// leaves a safe lane (x ranges listed) that moves from side to side.
    fn lava_walk(&mut self) -> Result<(), CoreError> {
        let ws = self.ws;
        let hazards = folder(&mut self.w, ws, "Lava")?;
        self.script("Script", hazards, "KnockOutOnTouch", scripts::LAVA)?;
        let course = self.course;
        let (top, half) = (12.0, 7.0);
        // Under-slab: the dark rim along the whole walk.
        self.part(
            "Part",
            course,
            "LavaWalkRim",
            [half * 2.0 + 0.6, 1.4, 53.9],
            [0.0, top - 0.9, -156.35],
            RIM,
            "SmoothPlastic",
        )?;
        self.support(-4.0, top - 1.6, -140.0, 3.0)?;
        self.support(4.0, top - 1.6, -172.0, 3.0)?;
        let rows: [LavaRow; 4] = [
            (-137.0, 4.0, &[(3.0, 7.0)]),
            (-149.0, 4.0, &[(-7.0, -3.0)]),
            (-161.0, 4.0, &[(-2.0, 2.0)]),
            (-173.0, 5.0, &[(-7.0, -3.5), (3.5, 7.0)]),
        ];
        let mut z = -129.0;
        for (i, (start, len, safe)) in rows.iter().enumerate() {
            self.tile(&format!("LavaWalkFloor{}", i + 1), -half, half, z, *start)?;
            // Lava fills the row except the safe lanes, which are floor.
            let mut x = -half;
            for &(a, b) in safe.iter() {
                if a > x {
                    self.lava(hazards, x, a, *start, start - len)?;
                }
                self.tile("SafeLane", a, b, *start, start - len)?;
                x = b;
            }
            if x < half {
                self.lava(hazards, x, half, *start, start - len)?;
            }
            z = start - len;
        }
        self.tile("LavaWalkFloor5", -half, half, z, -183.0)?;
        Ok(())
    }

    /// A floor tile covering x0..x1 and z0..z1 (z0 > z1) with its top at the lava walk height.
    fn tile(&mut self, name: &str, x0: f64, x1: f64, z0: f64, z1: f64) -> Result<(), CoreError> {
        let course = self.course;
        self.part(
            "Part",
            course,
            name,
            [x1 - x0, 1.0, z0 - z1],
            [(x0 + x1) / 2.0, 11.5, (z0 + z1) / 2.0],
            TOP,
            "Concrete",
        )?;
        Ok(())
    }

    /// A lava strip covering x0..x1 and z0..z1, flush with the floor.
    fn lava(
        &mut self,
        parent: InstanceId,
        x0: f64,
        x1: f64,
        z0: f64,
        z1: f64,
    ) -> Result<(), CoreError> {
        self.part(
            "Part",
            parent,
            "Lava",
            [x1 - x0, 1.0, z0 - z1],
            [(x0 + x1) / 2.0, 11.5, (z0 + z1) / 2.0],
            LAVA,
            "SmoothPlastic",
        )?;
        Ok(())
    }

    /// A lava pool (z -316 to -340, top y = 17) crossed on four stepping stones (top y = 18.5).
    fn lava_pool(&mut self) -> Result<(), CoreError> {
        let lava = self
            .w
            .find_first_child(self.ws, "Lava")
            .ok_or(CoreError::InvalidInstance)?;
        self.part(
            "Part",
            lava,
            "LavaPool",
            [16.0, 1.0, 24.0],
            [0.0, 16.5, -328.0],
            LAVA,
            "SmoothPlastic",
        )?;
        let course = self.course;
        self.part(
            "Part",
            course,
            "LavaPoolRim",
            [16.6, 1.4, 24.6],
            [0.0, 16.1, -328.0],
            RIM,
            "SmoothPlastic",
        )?;
        self.support(0.0, 15.4, -328.0, 4.0)?;
        for (i, (x, z)) in [(-3.0, -319.0), (2.0, -325.0), (-2.0, -331.0), (3.0, -337.0)]
            .into_iter()
            .enumerate()
        {
            self.part(
                "Part",
                course,
                &format!("SteppingStone{}", i + 1),
                [4.0, 2.5, 4.0],
                [x, 17.25, z],
                TOP,
                "Concrete",
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
        for i in 0..22 {
            let side = if i % 2 == 0 { -1.0 } else { 1.0 };
            let x = side * (48.0 + next() * 70.0);
            let z = 40.0 - f64::from(i) * 23.0 - next() * 10.0;
            let width = 6.0 + next() * 8.0;
            let top = -25.0 + next() * 60.0;
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
                "Metal",
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
        let help = w.create("TextLabel", Some(hud))?;
        w.set(help, "Name", Value::String("Controls".into()))?;
        w.set(
            help,
            "Position",
            Value::UDim2(UDim2::new(0.0, 16.0, 1.0, -48.0)),
        )?;
        w.set(
            help,
            "Size",
            Value::UDim2(UDim2::new(0.0, 560.0, 0.0, 32.0)),
        )?;
        w.set(help, "BackgroundColor3", Value::Color3(HUD_BACK))?;
        w.set(help, "BackgroundTransparency", Value::Number(0.35))?;
        w.set(help, "TextColor3", Value::Color3(HUD_DIM))?;
        w.set(help, "TextSize", Value::Number(18.0))?;
        w.set(
            help,
            "Text",
            Value::String("WASD move   Space jump   Shift run   Right-drag camera".into()),
        )?;
        self.script("LocalScript", hud, "ShowStatus", scripts::HUD)
    }
}

/// One lava strip across the walk: z where it starts, its length, and the safe lanes (x from, x to).
type LavaRow = (f64, f64, &'static [(f64, f64)]);

fn folder(w: &mut World, parent: InstanceId, name: &str) -> Result<InstanceId, CoreError> {
    let f = w.create("Folder", Some(parent))?;
    w.set(f, "Name", Value::String(name.into()))?;
    Ok(f)
}
