//! Bounds-checked access to little-endian Dalvik code units.

use super::{DecodeError, DecodeReason};

/// Cursor over a validated raw instruction byte slice.
pub(crate) struct CodeUnits<'a> {
    bytes: &'a [u8],
    position: u32,
    len: u32,
}

impl<'a> CodeUnits<'a> {
    pub(crate) const fn new(bytes: &'a [u8], len: u32) -> Self {
        Self {
            bytes,
            position: 0,
            len,
        }
    }

    pub(crate) const fn position(&self) -> u32 {
        self.position
    }

    pub(crate) const fn len(&self) -> u32 {
        self.len
    }

    pub(crate) const fn remaining(&self) -> u32 {
        self.len - self.position
    }

    pub(crate) fn advance(&mut self, width: u32, opcode: u8) -> Result<(), DecodeError> {
        if width > self.remaining() {
            return Err(DecodeError::new(
                self.position,
                opcode,
                DecodeReason::Truncated {
                    needed: width,
                    remaining: self.remaining(),
                },
            ));
        }
        self.position += width;
        Ok(())
    }

    pub(crate) fn unit(&self, offset: u32, insn: u32, opcode: u8) -> Result<u16, DecodeError> {
        if offset >= self.len {
            return Err(DecodeError::new(
                insn,
                opcode,
                DecodeReason::Truncated {
                    needed: offset.saturating_sub(insn).saturating_add(1),
                    remaining: self.len.saturating_sub(insn),
                },
            ));
        }
        let byte = usize::try_from(offset)
            .ok()
            .and_then(|value| value.checked_mul(2));
        let pair = byte.and_then(|start| self.bytes.get(start..start + 2));
        match pair {
            Some(bytes) => Ok(u16::from_le_bytes([bytes[0], bytes[1]])),
            None => Err(DecodeError::new(
                insn,
                opcode,
                DecodeReason::Truncated {
                    needed: offset.saturating_sub(insn).saturating_add(1),
                    remaining: self.len.saturating_sub(insn),
                },
            )),
        }
    }
}
