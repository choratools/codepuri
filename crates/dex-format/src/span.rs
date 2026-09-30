//! Byte ranges retained from the original DEX container.

/// A half-open byte range in the original physical DEX container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawSpan {
    /// Inclusive byte offset from the beginning of the physical container.
    pub offset: u32,
    /// Number of bytes in the range.
    pub size: u32,
}

impl RawSpan {
    /// Creates a checked span from host-sized values.
    pub(crate) fn from_usize(offset: usize, size: usize) -> Result<Self, crate::DexError> {
        Ok(Self {
            offset: u32::try_from(offset)
                .map_err(|_| crate::DexError::malformed(offset, "span offset exceeds u32"))?,
            size: u32::try_from(size)
                .map_err(|_| crate::DexError::malformed(offset, "span size exceeds u32"))?,
        })
    }
}
