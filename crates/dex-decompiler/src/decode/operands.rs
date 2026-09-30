use super::payload::{read_i32, read_u32};
use super::reader::CodeUnits;
use super::{DecodeError, DecodeReason, Format};

pub(crate) struct Operands {
    pub(crate) registers: Vec<u16>,
    pub(crate) literal: Option<i64>,
    pub(crate) pool_index: Option<u32>,
    pub(crate) secondary_pool_index: Option<u32>,
    pub(crate) branch_offset: Option<i32>,
}

impl Operands {
    fn empty() -> Self {
        Self {
            registers: Vec::new(),
            literal: None,
            pool_index: None,
            secondary_pool_index: None,
            branch_offset: None,
        }
    }
}

pub(crate) fn decode(
    reader: &CodeUnits<'_>,
    offset: u32,
    first: u16,
    opcode: u8,
    format: Format,
) -> Result<Operands, DecodeError> {
    let mut operands = Operands::empty();
    let high = first.to_le_bytes()[1];
    match format {
        Format::F10x => {}
        Format::F12x => operands
            .registers
            .extend([u16::from(high & 0x0f), u16::from(high >> 4)]),
        Format::F11n => {
            operands.registers.push(u16::from(high & 0x0f));
            operands.literal = Some(i64::from(sign_nibble(high >> 4)));
        }
        Format::F11x => operands.registers.push(u16::from(high)),
        Format::F10t => operands.branch_offset = Some(i32::from(i8::from_ne_bytes([high]))),
        Format::F20t => {
            operands.branch_offset = Some(i32::from(read_i16(reader, offset + 1, offset, opcode)?))
        }
        Format::F22x => operands
            .registers
            .extend([u16::from(high), reader.unit(offset + 1, offset, opcode)?]),
        Format::F21t => {
            operands.registers.push(u16::from(high));
            operands.branch_offset = Some(i32::from(read_i16(reader, offset + 1, offset, opcode)?));
        }
        Format::F21s => {
            operands.registers.push(u16::from(high));
            operands.literal = Some(i64::from(read_i16(reader, offset + 1, offset, opcode)?));
        }
        Format::F21h => {
            operands.registers.push(u16::from(high));
            let value = i64::from(read_i16(reader, offset + 1, offset, opcode)?);
            operands.literal = Some(if opcode == 0x19 {
                value << 48
            } else {
                value << 16
            });
        }
        Format::F21c => {
            operands.registers.push(u16::from(high));
            operands.pool_index = Some(u32::from(reader.unit(offset + 1, offset, opcode)?));
        }
        Format::F23x => {
            let pair = reader.unit(offset + 1, offset, opcode)?.to_le_bytes();
            operands
                .registers
                .extend([u16::from(high), u16::from(pair[0]), u16::from(pair[1])]);
        }
        Format::F22b => {
            let pair = reader.unit(offset + 1, offset, opcode)?.to_le_bytes();
            operands
                .registers
                .extend([u16::from(high), u16::from(pair[0])]);
            operands.literal = Some(i64::from(i8::from_ne_bytes([pair[1]])));
        }
        Format::F22t => {
            operands
                .registers
                .extend([u16::from(high & 0x0f), u16::from(high >> 4)]);
            operands.branch_offset = Some(i32::from(read_i16(reader, offset + 1, offset, opcode)?));
        }
        Format::F22s => {
            operands
                .registers
                .extend([u16::from(high & 0x0f), u16::from(high >> 4)]);
            operands.literal = Some(i64::from(read_i16(reader, offset + 1, offset, opcode)?));
        }
        Format::F22c => {
            operands
                .registers
                .extend([u16::from(high & 0x0f), u16::from(high >> 4)]);
            operands.pool_index = Some(u32::from(reader.unit(offset + 1, offset, opcode)?));
        }
        Format::F30t => {
            operands.branch_offset = Some(read_i32(reader, offset + 1, offset, opcode)?)
        }
        Format::F31i => {
            operands.registers.push(u16::from(high));
            operands.literal = Some(i64::from(read_i32(reader, offset + 1, offset, opcode)?));
        }
        Format::F31t => {
            operands.registers.push(u16::from(high));
            operands.branch_offset = Some(read_i32(reader, offset + 1, offset, opcode)?);
        }
        Format::F31c => {
            operands.registers.push(u16::from(high));
            operands.pool_index = Some(read_u32(reader, offset + 1, offset, opcode)?);
        }
        Format::F32x => operands.registers.extend([
            reader.unit(offset + 1, offset, opcode)?,
            reader.unit(offset + 2, offset, opcode)?,
        ]),
        Format::F35c => decode_packed(reader, offset, high, opcode, &mut operands)?,
        Format::F45cc => {
            decode_packed(reader, offset, high, opcode, &mut operands)?;
            operands.secondary_pool_index =
                Some(u32::from(reader.unit(offset + 3, offset, opcode)?));
        }
        Format::F3rc => decode_range(reader, offset, high, opcode, &mut operands)?,
        Format::F4rcc => {
            decode_range(reader, offset, high, opcode, &mut operands)?;
            operands.secondary_pool_index =
                Some(u32::from(reader.unit(offset + 3, offset, opcode)?));
        }
        Format::F51l => {
            operands.registers.push(u16::from(high));
            let mut bytes = [0_u8; 8];
            for index in 0_u32..4 {
                let pair = reader
                    .unit(offset + 1 + index, offset, opcode)?
                    .to_le_bytes();
                let byte_index =
                    usize::try_from(index * 2).map_err(|_| truncated(offset, opcode))?;
                bytes[byte_index..byte_index + 2].copy_from_slice(&pair);
            }
            operands.literal = Some(i64::from_le_bytes(bytes));
        }
        Format::PackedSwitchPayload | Format::SparseSwitchPayload | Format::ArrayDataPayload => {}
    }
    Ok(operands)
}

fn decode_packed(
    reader: &CodeUnits<'_>,
    offset: u32,
    high: u8,
    opcode: u8,
    out: &mut Operands,
) -> Result<(), DecodeError> {
    let count = high >> 4;
    if count > 5 {
        return Err(DecodeError::new(
            offset,
            opcode,
            DecodeReason::InvalidArgumentCount { count },
        ));
    }
    out.pool_index = Some(u32::from(reader.unit(offset + 1, offset, opcode)?));
    let packed = reader.unit(offset + 2, offset, opcode)?;
    let values = [
        packed & 0x0f,
        (packed >> 4) & 0x0f,
        (packed >> 8) & 0x0f,
        packed >> 12,
        u16::from(high & 0x0f),
    ];
    out.registers
        .extend(values[..usize::from(count)].iter().copied());
    Ok(())
}

fn decode_range(
    reader: &CodeUnits<'_>,
    offset: u32,
    count: u8,
    opcode: u8,
    out: &mut Operands,
) -> Result<(), DecodeError> {
    out.pool_index = Some(u32::from(reader.unit(offset + 1, offset, opcode)?));
    let start = u32::from(reader.unit(offset + 2, offset, opcode)?);
    out.registers.reserve(usize::from(count));
    for delta in 0..u32::from(count) {
        let register = u16::try_from(start + delta).map_err(|_| {
            DecodeError::new(
                offset,
                opcode,
                DecodeReason::RegisterOutOfRange {
                    register: u16::MAX,
                    registers_size: u16::MAX,
                },
            )
        })?;
        out.registers.push(register);
    }
    Ok(())
}

fn read_i16(
    reader: &CodeUnits<'_>,
    offset: u32,
    insn: u32,
    opcode: u8,
) -> Result<i16, DecodeError> {
    Ok(i16::from_le_bytes(
        reader.unit(offset, insn, opcode)?.to_le_bytes(),
    ))
}

const fn sign_nibble(value: u8) -> i8 {
    i8::from_ne_bytes([(value << 4)]) >> 4
}

const fn truncated(offset: u32, opcode: u8) -> DecodeError {
    DecodeError::new(
        offset,
        opcode,
        DecodeReason::Truncated {
            needed: 5,
            remaining: 0,
        },
    )
}
