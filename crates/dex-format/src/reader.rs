use crate::DexError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Endian {
    Little,
    Big,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    endian: Endian,
}

impl<'a> Reader<'a> {
    pub(crate) const fn new(bytes: &'a [u8], endian: Endian) -> Self {
        Self { bytes, endian }
    }

    pub(crate) const fn len(self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn slice(self, offset: usize, size: usize) -> Result<&'a [u8], DexError> {
        let end = offset.checked_add(size).ok_or(DexError::OutOfBounds {
            offset,
            size,
            len: self.bytes.len(),
        })?;
        self.bytes.get(offset..end).ok_or(DexError::OutOfBounds {
            offset,
            size,
            len: self.bytes.len(),
        })
    }

    pub(crate) fn u8(self, offset: usize) -> Result<u8, DexError> {
        self.bytes
            .get(offset)
            .copied()
            .ok_or(DexError::OutOfBounds {
                offset,
                size: 1,
                len: self.bytes.len(),
            })
    }

    pub(crate) fn u16(self, offset: usize) -> Result<u16, DexError> {
        let bytes: [u8; 2] = self
            .slice(offset, 2)?
            .try_into()
            .map_err(|_| DexError::malformed(offset, "invalid u16"))?;
        Ok(match self.endian {
            Endian::Little => u16::from_le_bytes(bytes),
            Endian::Big => u16::from_be_bytes(bytes),
        })
    }

    pub(crate) fn u32(self, offset: usize) -> Result<u32, DexError> {
        let bytes: [u8; 4] = self
            .slice(offset, 4)?
            .try_into()
            .map_err(|_| DexError::malformed(offset, "invalid u32"))?;
        Ok(match self.endian {
            Endian::Little => u32::from_le_bytes(bytes),
            Endian::Big => u32::from_be_bytes(bytes),
        })
    }

    pub(crate) fn usize_u32(self, offset: usize) -> Result<usize, DexError> {
        usize::try_from(self.u32(offset)?)
            .map_err(|_| DexError::malformed(offset, "value does not fit usize"))
    }
}

pub(crate) fn checked_table(
    reader: Reader<'_>,
    offset: u32,
    count: u32,
    item_size: usize,
    name: &str,
) -> Result<usize, DexError> {
    let start = usize::try_from(offset)
        .map_err(|_| DexError::malformed(0, format!("{name} offset does not fit usize")))?;
    let count = usize::try_from(count)
        .map_err(|_| DexError::malformed(start, format!("{name} count does not fit usize")))?;
    let size = count
        .checked_mul(item_size)
        .ok_or_else(|| DexError::malformed(start, format!("{name} byte size overflow")))?;
    reader.slice(start, size)?;
    Ok(start)
}
