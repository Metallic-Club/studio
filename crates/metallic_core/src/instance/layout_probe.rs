//! Manual layout observation for the bounded journal; no performance threshold or timing assertion.
use super::*;
use std::mem::{size_of, size_of_val};

#[test]
#[ignore = "manual journal-layout observation; run explicitly before/after metadata changes"]
fn journal_metadata_layout() {
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    for index in 0..40_000 {
        world
            .set(
                part,
                "Transparency",
                Value::Number((index % 7) as f64 / 10.0 + 0.01),
            )
            .unwrap();
    }
    let entry = &world.journal[0];
    eprintln!(
        "journal_metadata_layout: entry_bytes={} optional_kind_bytes={} public_kind_bytes={} public_change_bytes={} capacity={} retained_entries={} requested_history_buffer_bytes={}",
        size_of::<JournalEntry>(),
        size_of_val(&entry.kind),
        size_of::<ChangeKind>(),
        size_of::<Change>(),
        world.journal.capacity(),
        world.journal.len(),
        world.journal.capacity() * size_of::<JournalEntry>(),
    );
}
