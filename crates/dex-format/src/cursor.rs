use crate::DexError;
use crate::uleb::{read_sleb128, read_uleb128};

pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) const fn new(bytes: &'a [u8], position: usize) -> Self {
        Self { bytes, position }
    }

    pub(crate) const fn position(&self) -> usize {
        self.position
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, DexError> {
        let value = self
            .bytes
            .get(self.position)
            .copied()
            .ok_or(DexError::OutOfBounds {
                offset: self.position,
                size: 1,
                len: self.bytes.len(),
            })?;
        self.position += 1;
        Ok(value)
    }

    pub(crate) fn bytes(&mut self, count: usize) -> Result<&'a [u8], DexError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(DexError::OutOfBounds {
                offset: self.position,
                size: count,
                len: self.bytes.len(),
            })?;
        let output = self
            .bytes
            .get(self.position..end)
            .ok_or(DexError::OutOfBounds {
                offset: self.position,
                size: count,
                len: self.bytes.len(),
            })?;
        self.position = end;
        Ok(output)
    }

    pub(crate) fn uleb(&mut self) -> Result<u32, DexError> {
        let (value, consumed) = read_uleb128(self.bytes, self.position)?;
        self.position += consumed;
        Ok(value)
    }

    pub(crate) fn sleb(&mut self) -> Result<i32, DexError> {
        let (value, consumed) = read_sleb128(self.bytes, self.position)?;
        self.position += consumed;
        Ok(value)
    }
}
