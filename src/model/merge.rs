#![allow(unused_variables, dead_code)] // stub: T05 removes this
//! STA-4's merge (T05).

use super::{Before, Carry, ChecksRead, EventKind, ItemsOutcome, Merged, PrRead, ReadMeta, Record};

impl ChecksRead {
    /// POL-8: items added, `total`, `rollup`, `next_page` replaced.
    pub fn append_page(&mut self, page: ChecksRead) {
        unimplemented!("T05")
    }
}

/// STA-4's merge. Pure: no clock, no I/O. `record.rev` is the old one (0 for a new record); the store sets it.
pub fn apply_read(
    before: Before<'_>,
    carry: &Carry,
    read: &PrRead,
    items: ItemsOutcome<'_>,
    meta: &ReadMeta,
) -> Merged {
    unimplemented!("T05")
}

/// MOD-11, POL-21: an open record becomes `gone`. `None` if it isn't open.
pub fn apply_gone(record: &Record) -> Option<(Record, Vec<EventKind>)> {
    unimplemented!("T05")
}

/// POL-12: whether `read` shows `counts.comments` or `counts.reviews` above the record's.
pub fn counts_rose(record: &Record, read: &PrRead) -> bool {
    unimplemented!("T05")
}
