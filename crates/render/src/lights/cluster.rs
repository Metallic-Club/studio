//! The froxel grid: which drawn lights touch each cluster of the view, packed into the texels the shaders read (see
//! the [module documentation](super)). CPU only.
//!
//! Layout of the texture (`R32Uint`, [`TEXTURE_WIDTH`] texels a row, read in row order):
//!
//! - texels `0 .. CLUSTER_COUNT`: one header per cluster, `first << 8 | count`, where `first` is the cluster's first
//!   entry in the index list and `count` how many lights it lists;
//! - from texel `CLUSTER_COUNT` on: the index list, four 8-bit light indices to a texel (entry `k` in byte `k % 4`
//!   of texel `CLUSTER_COUNT + k / 4`).
//!
//! Clusters are numbered `(slice × CLUSTERS_Y + row) × CLUSTERS_X + column`, rows from the top of the screen.

use metallic_core::glam::Vec3;

use super::MAX_LIGHTS_PER_CLUSTER;

/// Columns of the grid across the screen.
pub const CLUSTERS_X: u32 = 16;
/// Rows of the grid down the screen.
pub const CLUSTERS_Y: u32 = 9;
/// Depth slices of the grid.
pub const CLUSTERS_Z: u32 = 24;
/// Clusters in the grid.
pub const CLUSTER_COUNT: usize = (CLUSTERS_X * CLUSTERS_Y * CLUSTERS_Z) as usize;
/// The view depth, in metres, where the first slice ends; the other slices are spaced exponentially from here to
/// the farthest drawn light.
pub const FIRST_SLICE: f32 = 1.0;

/// Texels in a row of the cluster texture.
pub(crate) const TEXTURE_WIDTH: u32 = 1024;
/// Rows of the cluster texture: room for every header and a full list in every cluster.
pub(crate) const TEXTURE_HEIGHT: u32 =
    (CLUSTER_COUNT as u32 + CLUSTER_COUNT as u32 * MAX_LIGHTS_PER_CLUSTER / 4).div_ceil(TEXTURE_WIDTH);
/// Texels in the cluster texture.
pub(crate) const TEXTURE_TEXELS: usize = (TEXTURE_WIDTH * TEXTURE_HEIGHT) as usize;

/// How view depths map to slices: slice 0 is everything nearer than [`FIRST_SLICE`]; slice `s ≥ 1` holds the depths
/// whose `log2(depth) × scale + bias` is in `[s − 1, s)`, the last one everything beyond.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Slicing {
    pub(crate) scale: f32,
    pub(crate) bias: f32,
}

impl Slicing {
    /// Slices out to `far` metres.
    pub(crate) fn new(far: f32) -> Self {
        let far = if far.is_finite() {
            far.max(FIRST_SLICE * 4.0)
        } else {
            FIRST_SLICE * 4.0
        };
        let scale = (CLUSTERS_Z - 1) as f32 / (far / FIRST_SLICE).log2();
        Self {
            scale,
            bias: -FIRST_SLICE.log2() * scale,
        }
    }

    /// The slice a view depth is in (the shader's `cluster_index`): 0 nearer than [`FIRST_SLICE`] (or not a number).
    pub(crate) fn slice(&self, depth: f32) -> u32 {
        if depth.is_nan() || depth < FIRST_SLICE {
            return 0;
        }
        let s = (depth.log2() * self.scale + self.bias).max(0.0);
        (s as u32 + 1).min(CLUSTERS_Z - 1)
    }

    /// The depths slice `s` spans (the last one to infinity).
    pub(crate) fn bounds(&self, s: u32) -> (f32, f32) {
        if s == 0 {
            return (0.0, FIRST_SLICE);
        }
        let at = |t: f32| ((t - self.bias) / self.scale).exp2();
        let far = if s + 1 >= CLUSTERS_Z {
            f32::INFINITY
        } else {
            at(s as f32)
        };
        (at((s - 1) as f32), far)
    }
}

/// The camera a grid is built for: the slopes of its view's edges (half-width and half-height over depth).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ClusterCamera {
    pub(crate) tan_x: f32,
    pub(crate) tan_y: f32,
}

/// What a build did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClusterStats {
    pub(crate) lit_clusters: u32,
    pub(crate) entries: u32,
    pub(crate) max_per_cluster: u32,
    pub(crate) overflow: u32,
}

/// The grid's texels and the scratch they are built in. Reused frame to frame; allocates only while the light lists
/// grow to the scene's working size.
#[derive(Debug)]
pub(crate) struct ClusterGrid {
    /// The texture's contents ([`TEXTURE_TEXELS`] of them).
    pub(crate) texels: Vec<u32>,
    /// How many texels from the start the last build wrote.
    pub(crate) used: usize,
    /// `cluster << 8 | light` for every light in every cluster it touches, in light order.
    pairs: Vec<u32>,
    /// Per cluster: its count, then its next entry while filling.
    counts: Vec<u32>,
}

impl Default for ClusterGrid {
    fn default() -> Self {
        Self {
            texels: vec![0; TEXTURE_TEXELS],
            used: CLUSTER_COUNT,
            pairs: Vec::new(),
            counts: vec![0; CLUSTER_COUNT],
        }
    }
}

/// A sphere's extent across the screen in tiles of `count` along one axis: the projected range of its box, whose
/// coordinate across is `[low, high]` at depths `[near, far]`, through edge slope `tan`. `None` when the box reaches
/// behind the camera, where the projection does not bound it.
fn tile_range(low: f32, high: f32, near: f32, far: f32, tan: f32, count: u32) -> (u32, u32) {
    if near <= 1e-4 {
        return (0, count - 1);
    }
    // The smallest and largest slope over the box: a negative edge is widest at the near depth, a positive one at
    // the far depth, and the other way round for the upper bound.
    let min = low / if low < 0.0 { near } else { far };
    let max = high / if high > 0.0 { near } else { far };
    let to_tile = |slope: f32| {
        ((slope / tan + 1.0) * 0.5 * count as f32)
            .floor()
            .clamp(0.0, (count - 1) as f32) as u32
    };
    (to_tile(min), to_tile(max))
}

impl ClusterGrid {
    /// Builds the grid of `spheres` (view-space centres, x right, y up, z the depth ahead, and radii; index `i` is
    /// light `i` of the drawn list), at most `cap` lights per cluster, each cluster keeping the first (most
    /// important) lights that touch it.
    pub(crate) fn build(
        &mut self,
        spheres: &[(Vec3, f32)],
        camera: &ClusterCamera,
        slicing: &Slicing,
        cap: u32,
    ) -> ClusterStats {
        let mut stats = ClusterStats::default();
        self.pairs.clear();
        self.counts.fill(0);
        let (tan_x, tan_y) = (camera.tan_x.max(1e-4), camera.tan_y.max(1e-4));
        // Once per build: each slice's depths, and each column's and row's slopes (rows top down: row 0 spans
        // [1 − step, 1]), scaled to the view's edges.
        let slices: [(f32, f32); CLUSTERS_Z as usize] = std::array::from_fn(|s| slicing.bounds(s as u32));
        let step_x = 2.0 / CLUSTERS_X as f32;
        let step_y = 2.0 / CLUSTERS_Y as f32;
        let columns: [(f32, f32); CLUSTERS_X as usize] = std::array::from_fn(|x| {
            (
                (x as f32 * step_x - 1.0) * tan_x,
                ((x + 1) as f32 * step_x - 1.0) * tan_x,
            )
        });
        let rows: [(f32, f32); CLUSTERS_Y as usize] = std::array::from_fn(|y| {
            (
                (1.0 - (y + 1) as f32 * step_y) * tan_y,
                (1.0 - y as f32 * step_y) * tan_y,
            )
        });
        // The extent of a cluster across, at depths d0 to d1, between slopes a and b.
        let across = |(a, b): (f32, f32), d0: f32, d1: f32| ((a * d0).min(a * d1), (b * d0).max(b * d1));
        for (light, &(center, radius)) in spheres.iter().enumerate().take(256) {
            let (near, far) = (center.z - radius, center.z + radius);
            if far <= 0.0 || !center.is_finite() || !radius.is_finite() {
                continue;
            }
            let (s0, s1) = (slicing.slice(near.max(0.0)), slicing.slice(far));
            let r2 = radius * radius;
            for s in s0..=s1 {
                let (d0, d1) = slices[s as usize];
                let d1 = d1.min(far);
                // In this slice the sphere is at most `reach` across: bound its tiles by that, not by its radius.
                let dz = center.z - center.z.clamp(d0, d1);
                let reach2 = r2 - dz * dz;
                if reach2 < 0.0 {
                    continue;
                }
                let reach = reach2.sqrt();
                let (z0, z1) = (d0.max(near), d1);
                let (x0, x1) = tile_range(center.x - reach, center.x + reach, z0, z1, tan_x, CLUSTERS_X);
                // Rows count down from the top: the highest slope is row 0.
                let (y0, y1) = tile_range(-(center.y + reach), -(center.y - reach), z0, z1, tan_y, CLUSTERS_Y);
                for y in y0..=y1 {
                    let (lo_y, hi_y) = across(rows[y as usize], d0, d1);
                    let dy = center.y - center.y.clamp(lo_y, hi_y);
                    let partial = dy * dy + dz * dz;
                    if partial > r2 {
                        continue;
                    }
                    for x in x0..=x1 {
                        let (lo_x, hi_x) = across(columns[x as usize], d0, d1);
                        let dx = center.x - center.x.clamp(lo_x, hi_x);
                        if partial + dx * dx > r2 {
                            continue;
                        }
                        let cluster = ((s * CLUSTERS_Y + y) * CLUSTERS_X + x) as usize;
                        let count = &mut self.counts[cluster];
                        if *count >= cap {
                            stats.overflow += 1;
                            continue;
                        }
                        *count += 1;
                        self.pairs.push((cluster as u32) << 8 | light as u32);
                    }
                }
            }
        }
        // Headers: each cluster's first entry and count; then the counts become each cluster's next entry.
        let mut first = 0u32;
        for (header, count) in self.texels[..CLUSTER_COUNT].iter_mut().zip(&mut self.counts) {
            *header = first << 8 | *count;
            if *count > 0 {
                stats.lit_clusters += 1;
                stats.max_per_cluster = stats.max_per_cluster.max(*count);
            }
            let next = first + *count;
            *count = first;
            first = next;
        }
        stats.entries = first;
        let list_texels = (first as usize).div_ceil(4);
        self.used = CLUSTER_COUNT + list_texels;
        self.texels[CLUSTER_COUNT..self.used].fill(0);
        for &pair in &self.pairs {
            let cluster = (pair >> 8) as usize;
            let at = self.counts[cluster];
            self.counts[cluster] += 1;
            self.texels[CLUSTER_COUNT + (at / 4) as usize] |= (pair & 0xff) << ((at % 4) * 8);
        }
        stats
    }

    /// The lights cluster `cluster` lists, in order.
    #[cfg(test)]
    pub(crate) fn cluster_lights(&self, cluster: usize) -> impl Iterator<Item = u32> + '_ {
        let header = self.texels[cluster];
        let (first, count) = (header >> 8, header & 0xff);
        (first..first + count).map(|at| (self.texels[CLUSTER_COUNT + (at / 4) as usize] >> ((at % 4) * 8)) & 0xff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMERA: ClusterCamera = ClusterCamera { tan_x: 1.0, tan_y: 1.0 };

    /// The cluster a view-space point is in, as the shader finds it.
    fn cluster_of(p: Vec3, slicing: &Slicing) -> usize {
        let (nx, ny) = (p.x / p.z / CAMERA.tan_x, p.y / p.z / CAMERA.tan_y);
        let x = (((nx + 1.0) * 0.5 * CLUSTERS_X as f32) as u32).min(CLUSTERS_X - 1);
        let y = (((1.0 - ny) * 0.5 * CLUSTERS_Y as f32) as u32).min(CLUSTERS_Y - 1);
        ((slicing.slice(p.z) * CLUSTERS_Y + y) * CLUSTERS_X + x) as usize
    }

    #[test]
    fn slices_cover_depth_in_order() {
        let slicing = Slicing::new(200.0);
        assert_eq!(slicing.slice(0.5), 0);
        assert_eq!(slicing.slice(1.0), 1);
        assert_eq!(slicing.slice(199.0), CLUSTERS_Z - 1);
        assert_eq!(slicing.slice(1e6), CLUSTERS_Z - 1);
        let mut last = 0;
        for i in 1..2000 {
            let depth = i as f32 * 0.1;
            let s = slicing.slice(depth);
            assert!(s >= last);
            let (lo, hi) = slicing.bounds(s);
            assert!(
                lo <= depth * 1.0001 && depth <= hi * 1.0001,
                "{depth} in slice {s} [{lo}, {hi}]"
            );
            last = s;
        }
    }

    /// Every point inside a light's sphere is in a cluster that lists the light: the grid never misses a lit pixel.
    #[test]
    fn every_point_a_light_reaches_is_in_a_cluster_listing_it() {
        let slicing = Slicing::new(100.0);
        let spheres = [
            (Vec3::new(0.0, 0.0, 10.0), 3.0),
            (Vec3::new(-6.0, 2.0, 7.0), 5.0),
            (Vec3::new(4.0, -3.0, 2.0), 4.0),
            (Vec3::new(0.0, 0.0, 0.5), 2.0),
            (Vec3::new(30.0, 10.0, 60.0), 20.0),
        ];
        let mut grid = ClusterGrid::default();
        let stats = grid.build(&spheres, &CAMERA, &slicing, MAX_LIGHTS_PER_CLUSTER);
        assert_eq!(stats.overflow, 0);
        assert!(stats.entries > 0 && stats.lit_clusters > 0);
        let mut checked = 0;
        for (light, &(center, radius)) in spheres.iter().enumerate() {
            for i in 0..2000 {
                // A spread of points inside the sphere.
                let t = i as f32 / 2000.0;
                let dir = Vec3::new((t * 97.0).sin(), (t * 61.0).cos(), (t * 37.0).sin()).normalize_or_zero();
                let p = center + dir * radius * (t * 13.0).fract();
                if p.z <= 0.15 || (p.x / p.z).abs() > 1.0 || (p.y / p.z).abs() > 1.0 {
                    continue;
                }
                let cluster = cluster_of(p, &slicing);
                assert!(
                    grid.cluster_lights(cluster).any(|l| l == light as u32),
                    "light {light} reaches {p} but cluster {cluster} does not list it"
                );
                checked += 1;
            }
        }
        assert!(checked > 5000);
    }

    #[test]
    fn a_full_cluster_keeps_its_first_lights() {
        let slicing = Slicing::new(50.0);
        let spheres: Vec<_> = (0..10).map(|_| (Vec3::new(0.0, 0.0, 10.0), 1.0)).collect();
        let mut grid = ClusterGrid::default();
        let stats = grid.build(&spheres, &CAMERA, &slicing, 4);
        assert!(stats.overflow > 0);
        assert_eq!(stats.max_per_cluster, 4);
        let cluster = cluster_of(Vec3::new(0.0, 0.0, 10.0), &slicing);
        assert_eq!(grid.cluster_lights(cluster).collect::<Vec<_>>(), [0, 1, 2, 3]);
        // A far light lights only far clusters; nothing near lists it.
        let near = cluster_of(Vec3::new(0.0, 0.0, 2.0), &slicing);
        assert_eq!(grid.cluster_lights(near).count(), 0);
    }
}
