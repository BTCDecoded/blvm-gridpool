//! V2.2 leaves two-block reorg rollback unfinished. This library does not guess one.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReorgError {
    UnsupportedTwoBlock,
}

/// A two-block reorg is an explicit unsupported outcome. Depth 1 does not rewind state.
pub fn note_reorg(depth: u32) -> Result<(), ReorgError> {
    if depth >= 2 {
        Err(ReorgError::UnsupportedTwoBlock)
    } else {
        Ok(())
    }
}
