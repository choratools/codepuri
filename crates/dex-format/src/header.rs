//! DEX header parsing.

use crate::reader::{Endian, Reader};
use crate::{DexError, RawSpan};

const STANDARD_HEADER_SIZE: u32 = 0x70;
const CONTAINER_HEADER_SIZE: u32 = 0x78;

/// A count-and-offset pair from the DEX header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionHeader {
    /// Number of table entries.
    pub size: u32,
    /// Physical-container-relative byte offset.
    pub offset: u32,
}

/// Byte order selected by the header endian tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// Standard little-endian DEX.
    Little,
    /// Reverse-endian DEX.
    Big,
}

/// Fully decoded standard DEX header, including v041 container fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Exact eight-byte magic.
    pub magic: [u8; 8],
    /// Numeric magic version.
    pub version: u32,
    /// Adler-32 checksum stored in the file.
    pub checksum: u32,
    /// SHA-1 signature stored in the file.
    pub signature: [u8; 20],
    /// Logical DEX size, or distance to the next v041 header.
    pub file_size: u32,
    /// Header size, 112 bytes through v040 and 120 bytes in v041.
    pub header_size: u32,
    /// Validated byte order.
    pub byte_order: ByteOrder,
    /// Link section count and offset.
    pub link: SectionHeader,
    /// Map-list offset.
    pub map_off: u32,
    /// String identifier table.
    pub string_ids: SectionHeader,
    /// Type identifier table.
    pub type_ids: SectionHeader,
    /// Prototype identifier table.
    pub proto_ids: SectionHeader,
    /// Field identifier table.
    pub field_ids: SectionHeader,
    /// Method identifier table.
    pub method_ids: SectionHeader,
    /// Class definition table.
    pub class_defs: SectionHeader,
    /// Legacy data section count and offset.
    pub data: SectionHeader,
    /// Physical container size; equals `file_size` before v041.
    pub container_size: u32,
    /// Physical offset of this header; zero before v041.
    pub header_offset: u32,
    /// Exact header byte range.
    pub span: RawSpan,
}

impl Header {
    pub(crate) fn parse(bytes: &[u8], base: usize) -> Result<(Self, Endian), DexError> {
        let little = Reader::new(bytes, Endian::Little);
        let magic_slice = little.slice(base, 8)?;
        let magic: [u8; 8] = magic_slice
            .try_into()
            .map_err(|_| DexError::malformed(base, "invalid magic length"))?;
        if &magic[0..4] != b"dex\n" || magic[7] != 0 {
            return Err(DexError::malformed(base, "invalid DEX magic"));
        }
        let version_text = std::str::from_utf8(&magic[4..7])
            .map_err(|_| DexError::malformed(base + 4, "version is not ASCII"))?;
        let version = version_text
            .parse::<u32>()
            .map_err(|_| DexError::malformed(base + 4, "version is not decimal"))?;
        if !matches!(version, 35 | 37 | 38 | 39 | 40 | 41) {
            return Err(DexError::UnsupportedVersion(version));
        }

        let endian_tag = little.u32(base + 40)?;
        let endian = match endian_tag {
            0x1234_5678 => Endian::Little,
            0x7856_3412 => Endian::Big,
            _ => return Err(DexError::malformed(base + 40, "invalid endian tag")),
        };
        let reader = Reader::new(bytes, endian);
        let expected_size = if version == 41 {
            CONTAINER_HEADER_SIZE
        } else {
            STANDARD_HEADER_SIZE
        };
        let header_size = reader.u32(base + 36)?;
        if header_size != expected_size {
            return Err(DexError::malformed(base + 36, "unexpected header size"));
        }
        let header_bytes = usize::try_from(header_size)
            .map_err(|_| DexError::malformed(base + 36, "header size does not fit usize"))?;
        reader.slice(base, header_bytes)?;

        let file_size = reader.u32(base + 32)?;
        let (container_size, header_offset) = if version == 41 {
            (reader.u32(base + 112)?, reader.u32(base + 116)?)
        } else {
            (file_size, 0)
        };
        validate_bounds(
            bytes,
            base,
            version,
            file_size,
            container_size,
            header_offset,
        )?;
        let signature: [u8; 20] = reader
            .slice(base + 12, 20)?
            .try_into()
            .map_err(|_| DexError::malformed(base + 12, "invalid signature length"))?;
        let header = Self {
            magic,
            version,
            checksum: reader.u32(base + 8)?,
            signature,
            file_size,
            header_size,
            byte_order: match endian {
                Endian::Little => ByteOrder::Little,
                Endian::Big => ByteOrder::Big,
            },
            link: section(reader, base + 44)?,
            map_off: reader.u32(base + 52)?,
            string_ids: section(reader, base + 56)?,
            type_ids: section(reader, base + 64)?,
            proto_ids: section(reader, base + 72)?,
            field_ids: section(reader, base + 80)?,
            method_ids: section(reader, base + 88)?,
            class_defs: section(reader, base + 96)?,
            data: section(reader, base + 104)?,
            container_size,
            header_offset,
            span: RawSpan::from_usize(base, header_bytes)?,
        };
        if header.map_off == 0 {
            return Err(DexError::malformed(base + 52, "map offset is zero"));
        }
        Ok((header, endian))
    }
}

fn section(reader: Reader<'_>, offset: usize) -> Result<SectionHeader, DexError> {
    Ok(SectionHeader {
        size: reader.u32(offset)?,
        offset: reader.u32(offset + 4)?,
    })
}

fn validate_bounds(
    bytes: &[u8],
    base: usize,
    version: u32,
    file_size: u32,
    container_size: u32,
    header_offset: u32,
) -> Result<(), DexError> {
    let logical_size = usize::try_from(file_size)
        .map_err(|_| DexError::malformed(base + 32, "logical size does not fit usize"))?;
    let logical_end = base
        .checked_add(logical_size)
        .ok_or_else(|| DexError::malformed(base + 32, "logical size overflow"))?;
    if logical_end > bytes.len() {
        return Err(DexError::malformed(
            base + 32,
            "logical DEX exceeds container",
        ));
    }
    if version == 41 {
        if usize::try_from(header_offset).ok() != Some(base) {
            return Err(DexError::malformed(base + 116, "incorrect header offset"));
        }
        if usize::try_from(container_size).ok() != Some(bytes.len()) {
            return Err(DexError::malformed(base + 112, "incorrect container size"));
        }
    } else if base != 0 || logical_end != bytes.len() {
        return Err(DexError::malformed(
            base + 32,
            "file size does not match input",
        ));
    }
    Ok(())
}
