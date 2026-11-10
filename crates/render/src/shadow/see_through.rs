//! See-through casters (#122): the partly see-through parts a shadow view holds, grouped into draws.
//!
//! A shadow view's cull puts its see-through casters in its [`VisibleSet`]'s see-through list, in no particular order
//! and of every mesh kind. [`SeeThroughCasters::gather`] sorts them by mesh kind and level of detail, so they draw as
//! one instanced draw per kind and level, as the solid casters do, with the see-through caster pipeline
//! ([`super::caster_pipeline`]). The order within a draw does not matter: the maps hold depth only.

use crate::cull::VisibleSet;
use crate::scene::{InstanceScene, MeshKind};

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
