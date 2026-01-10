// Constants representing the bit masks
// Bit 0: 0000 0001
const DIRTY_MASK: u8 = 1 << 0; 
// Bit 1: 0000 0010
const TOMBSTONE_MASK: u8 = 1 << 1;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EntryHeader {
    pub timestamp: usize,
    pub key_size: i64,
    pub value_size: i64,
    pub meta: u8
}

impl EntryHeader {
    // --- Dirty Bit Methods ---

    /// Sets the dirty bit to 1 (True)
    pub fn set_dirty(&mut self) {
        self.meta |= DIRTY_MASK;
    }

    /// Sets the dirty bit to 0 (False)
    pub fn clear_dirty(&mut self) {
        self.meta &= !DIRTY_MASK;
    }

    /// Returns true if the dirty bit is set
    pub fn is_dirty(&self) -> bool {
        (self.meta & DIRTY_MASK) != 0
    }

    // --- Tombstone Bit Methods ---

    /// Sets the tombstone bit to 1 (True)
    pub fn set_tombstone(&mut self) {
        self.meta |= TOMBSTONE_MASK;
    }

    /// Sets the tombstone bit to 0 (False)
    pub fn clear_tombstone(&mut self) {
        self.meta &= !TOMBSTONE_MASK;
    }

    /// Returns true if the tombstone bit is set
    pub fn is_tombstone(&self) -> bool {
        (self.meta & TOMBSTONE_MASK) != 0
    }
}

pub struct EntryView<'a> {
    header: &'a EntryHeader,
    data: &'a [u8],
}
