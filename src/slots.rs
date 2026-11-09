//! Private program slots per performance.
//! Banks C and D (256 slots) are split into blocks of 6 consecutive slots; a performance that is
//! given a block keeps the programs of its Parts 1-6 there, so editing one performance's sound can
//! never overwrite a program another performance uses. 256 / 6 = 42 blocks.
pub const BLOCKS: usize = 42;
pub const PARTS: usize = 6;

/// (bank byte, index) of the slot for `part` (0-5) in `block`: bank 7 = C, 8 = D.
pub fn slot(block: usize, part: usize) -> (u8, u8) {
    let lin = block * PARTS + part;
    (7 + (lin / 128) as u8, (lin % 128) as u8)
}

/// Fixed rule: performance A000..A041 (bank byte 1, index 0..41) owns block = its index.
pub fn block_for_perf(bank: u8, idx: u8) -> Option<usize> {
    (bank == 1 && (idx as usize) < BLOCKS).then_some(idx as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_layout() {
        assert_eq!(slot(0, 0), (7, 0));
        assert_eq!(slot(0, 5), (7, 5));
        assert_eq!(slot(21, 0), (7, 126));
        assert_eq!(slot(21, 2), (8, 0));
        assert_eq!(slot(41, 5), (8, 123)); // 252 of 256 slots used
    }

    #[test]
    fn fixed_rule() {
        assert_eq!(block_for_perf(1, 0), Some(0));
        assert_eq!(block_for_perf(1, 41), Some(41));
        assert_eq!(block_for_perf(1, 42), None);
        assert_eq!(block_for_perf(2, 0), None);
    }
}
