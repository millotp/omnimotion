//! The host allocator does the work; `HEAP.free()` reports what the firmware's 160 kB heap would have left
//! after its usual allocations.

pub struct Heap(());

impl Heap {
    pub fn free(&self) -> usize {
        112 * 1024
    }
}

pub static HEAP: Heap = Heap(());
