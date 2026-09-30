//! DEX method code, try ranges, and catch handlers.

use crate::cursor::Cursor;
use crate::reader::Reader;
use crate::{DexError, RawSpan};

/// A typed exception handler target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeAddrPair {
    /// Exception type index.
    pub type_idx: u32,
    /// Target address in 16-bit code units.
    pub address: u32,
}

/// One encoded catch-handler sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedCatchHandler {
    /// Byte offset from the start of the handler-list size field.
    pub relative_offset: u32,
    /// Typed catches in test order.
    pub handlers: Vec<TypeAddrPair>,
    /// Optional catch-all target.
    pub catch_all_addr: Option<u32>,
    /// Exact encoded byte range.
    pub span: RawSpan,
}

/// A protected instruction range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryItem {
    /// First protected instruction address.
    pub start_addr: u32,
    /// Protected length in 16-bit code units.
    pub insn_count: u16,
    /// Byte offset into the encoded handler list.
    pub handler_off: u16,
    /// Exact fixed-width byte range.
    pub span: RawSpan,
}

/// A method code item retaining undecoded instruction bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeItem {
    /// Register count used by the method.
    pub registers_size: u16,
    /// Incoming argument word count.
    pub ins_size: u16,
    /// Outgoing argument word count.
    pub outs_size: u16,
    /// Debug-information item offset, or zero.
    pub debug_info_off: u32,
    /// Instruction count in 16-bit code units.
    pub insns_size: u32,
    /// Exact raw instruction bytes in file byte order.
    pub insns: Vec<u8>,
    /// Protected ranges.
    pub tries: Vec<TryItem>,
    /// Encoded catch handlers.
    pub handlers: Vec<EncodedCatchHandler>,
    /// Exact complete code-item byte range.
    pub span: RawSpan,
}

pub(crate) fn parse_code(reader: Reader<'_>, offset: u32) -> Result<CodeItem, DexError> {
    let start = usize::try_from(offset)
        .map_err(|_| DexError::malformed(0, "code offset does not fit usize"))?;
    reader.slice(start, 16)?;
    let registers_size = reader.u16(start)?;
    let ins_size = reader.u16(start + 2)?;
    let outs_size = reader.u16(start + 4)?;
    let tries_size = usize::from(reader.u16(start + 6)?);
    let debug_info_off = reader.u32(start + 8)?;
    let insns_size = reader.u32(start + 12)?;
    let instruction_bytes = usize::try_from(insns_size)
        .ok()
        .and_then(|count| count.checked_mul(2))
        .ok_or_else(|| DexError::malformed(start + 12, "instruction size overflow"))?;
    let insns_offset = start + 16;
    let insns = reader.slice(insns_offset, instruction_bytes)?.to_vec();
    let mut cursor = insns_offset
        .checked_add(instruction_bytes)
        .ok_or_else(|| DexError::malformed(insns_offset, "code offset overflow"))?;
    if tries_size != 0 && insns_size & 1 != 0 {
        if reader.u16(cursor)? != 0 {
            return Err(DexError::malformed(cursor, "nonzero code-item padding"));
        }
        cursor += 2;
    }
    let tries_bytes = tries_size
        .checked_mul(8)
        .ok_or_else(|| DexError::malformed(cursor, "try table size overflow"))?;
    reader.slice(cursor, tries_bytes)?;
    let mut tries: Vec<TryItem> = Vec::with_capacity(tries_size);
    for index in 0..tries_size {
        let item_offset = cursor + index * 8;
        let item = TryItem {
            start_addr: reader.u32(item_offset)?,
            insn_count: reader.u16(item_offset + 4)?,
            handler_off: reader.u16(item_offset + 6)?,
            span: RawSpan::from_usize(item_offset, 8)?,
        };
        let end = item
            .start_addr
            .checked_add(u32::from(item.insn_count))
            .ok_or_else(|| {
                DexError::malformed(item_offset, "protected instruction range overflow")
            })?;
        if end > insns_size {
            return Err(DexError::malformed(
                item_offset,
                "try range exceeds instructions",
            ));
        }
        if let Some(previous) = tries.last()
            && previous.start_addr + u32::from(previous.insn_count) > item.start_addr
        {
            return Err(DexError::malformed(item_offset, "try ranges overlap"));
        }
        tries.push(item);
    }
    cursor += tries_bytes;
    let handlers = if tries_size == 0 {
        Vec::new()
    } else {
        parse_handlers(reader.slice(0, reader.len())?, cursor, &tries)?
    };
    let end = match handlers.last() {
        Some(handler) => {
            let raw_end = handler
                .span
                .offset
                .checked_add(handler.span.size)
                .ok_or_else(|| DexError::malformed(cursor, "handler span overflow"))?;
            usize::try_from(raw_end)
                .map_err(|_| DexError::malformed(cursor, "handler end does not fit usize"))?
        }
        None => cursor,
    };
    Ok(CodeItem {
        registers_size,
        ins_size,
        outs_size,
        debug_info_off,
        insns_size,
        insns,
        tries,
        handlers,
        span: RawSpan::from_usize(start, end - start)?,
    })
}

fn parse_handlers(
    bytes: &[u8],
    start: usize,
    tries: &[TryItem],
) -> Result<Vec<EncodedCatchHandler>, DexError> {
    let mut cursor = Cursor::new(bytes, start);
    let count = usize::try_from(cursor.uleb()?)
        .map_err(|_| DexError::malformed(start, "handler count does not fit usize"))?;
    if count > cursor.remaining() {
        return Err(DexError::malformed(start, "handler count exceeds input"));
    }
    let mut handlers = Vec::with_capacity(count);
    for _ in 0..count {
        let item_start = cursor.position();
        let relative_offset = u32::try_from(item_start - start)
            .map_err(|_| DexError::malformed(item_start, "handler offset exceeds u32"))?;
        let signed_size = cursor.sleb()?;
        let typed_count = usize::try_from(signed_size.unsigned_abs()).map_err(|_| {
            DexError::malformed(item_start, "typed handler count does not fit usize")
        })?;
        if typed_count > cursor.remaining() / 2 {
            return Err(DexError::malformed(
                item_start,
                "typed handler count exceeds input",
            ));
        }
        let mut typed = Vec::with_capacity(typed_count);
        for _ in 0..typed_count {
            typed.push(TypeAddrPair {
                type_idx: cursor.uleb()?,
                address: cursor.uleb()?,
            });
        }
        let catch_all_addr = (signed_size <= 0).then(|| cursor.uleb()).transpose()?;
        handlers.push(EncodedCatchHandler {
            relative_offset,
            handlers: typed,
            catch_all_addr,
            span: RawSpan::from_usize(item_start, cursor.position() - item_start)?,
        });
    }
    for try_item in tries {
        if !handlers
            .iter()
            .any(|handler| handler.relative_offset == u32::from(try_item.handler_off))
        {
            return Err(DexError::malformed(
                usize::try_from(try_item.span.offset).map_err(|_| {
                    DexError::malformed(start, "try item offset does not fit usize")
                })?,
                "try item references no catch handler",
            ));
        }
    }
    Ok(handlers)
}
