//! Material keys (#678): which material table row draws which material.
//!
//! A part's `Material` is a key: a material asset's id, or a material's name (ADR 0012). Every key the scene meets
//! gets a row of the material table ([`crate::MaterialTable`]), numbered from 1 in the order the scene first meets
//! them; row 0 is the default look (an empty `Material`). A key keeps its row for the scene's life. The renderer fills
//! each row from what it was told about the key ([`crate::PartRenderer::set_material`]), or from the built-in material
//! of that name, or plastic.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::parts::MAX_MATERIAL_ROWS;

/// A revision no other change, in any scene of the process, has had.
fn next_revision() -> u64 {
    static REVISION: AtomicU64 = AtomicU64::new(1);
    REVISION.fetch_add(1, Ordering::Relaxed)
}

/// A scene's material keys and their rows.
#[derive(Debug)]
pub(crate) struct MaterialKeys {
    /// Row `i + 1` draws `keys[i]`.
    keys: Vec<Arc<str>>,
    rows: HashMap<Arc<str>, u16>,
    /// The key last looked up and its row: parts of one material usually come together.
    last: Option<(Arc<str>, u16)>,
    revision: u64,
}

impl Default for MaterialKeys {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            rows: HashMap::new(),
            last: None,
            revision: next_revision(),
        }
    }
}

impl MaterialKeys {
    /// The row of `key`, numbered now when it is new; 0 for an empty key or once every row is taken. Allocates only
    /// for a new key.
    pub(crate) fn row(&mut self, key: &str) -> u16 {
        if key.is_empty() {
            return 0;
        }
        if let Some((last, row)) = &self.last
            && **last == *key
        {
            return *row;
        }
        let row = match self.rows.get(key) {
            Some(&row) => row,
            None => {
                if self.keys.len() + 1 >= MAX_MATERIAL_ROWS {
                    // Every row is taken: the rest draw as plastic.
                    return 0;
                }
                let key: Arc<str> = Arc::from(key);
                self.keys.push(Arc::clone(&key));
                let row = u16::try_from(self.keys.len()).unwrap_or(0);
                self.rows.insert(key, row);
                self.revision = next_revision();
                row
            }
        };
        if let Some(key) = self.keys.get(usize::from(row).wrapping_sub(1)) {
            self.last = Some((Arc::clone(key), row));
        }
        row
    }

    /// Every key, row `i + 1` for the `i`th.
    pub(crate) fn keys(&self) -> &[Arc<str>] {
        &self.keys
    }

    /// Changes whenever a key is added; no two scenes share one.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_get_rows_from_one_in_the_order_they_are_met() {
        let mut keys = MaterialKeys::default();
        let start = keys.revision();
        assert_eq!(keys.row(""), 0);
        assert_eq!(keys.revision(), start, "no key, no change");
        assert_eq!(keys.row("Neon"), 1);
        assert_eq!(keys.row("0123456789abcdef0123456789abcdef"), 2);
        assert_eq!(keys.row("Neon"), 1);
        assert_ne!(keys.revision(), start);
        assert_eq!(keys.keys().len(), 2);
    }

    #[test]
    fn past_the_table_keys_draw_with_the_default_row() {
        let mut keys = MaterialKeys::default();
        for n in 1..MAX_MATERIAL_ROWS {
            assert_eq!(usize::from(keys.row(&format!("m{n}"))), n);
        }
        assert_eq!(keys.row("one too many"), 0);
    }
}
