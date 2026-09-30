//! Portable DEX 035-041 opcode metadata.

/// Operand encoding used by a Dalvik instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// Format 10x.
    F10x,
    /// Format 12x.
    F12x,
    /// Format 11n.
    F11n,
    /// Format 11x.
    F11x,
    /// Format 10t.
    F10t,
    /// Format 20t.
    F20t,
    /// Format 22x.
    F22x,
    /// Format 21t.
    F21t,
    /// Format 21s.
    F21s,
    /// Format 21h.
    F21h,
    /// Format 21c.
    F21c,
    /// Format 23x.
    F23x,
    /// Format 22b.
    F22b,
    /// Format 22t.
    F22t,
    /// Format 22s.
    F22s,
    /// Format 22c.
    F22c,
    /// Format 30t.
    F30t,
    /// Format 31i.
    F31i,
    /// Format 31t.
    F31t,
    /// Format 31c.
    F31c,
    /// Format 32x.
    F32x,
    /// Format 35c.
    F35c,
    /// Format 3rc.
    F3rc,
    /// Format 45cc.
    F45cc,
    /// Format 4rcc.
    F4rcc,
    /// Format 51l.
    F51l,
    /// Variable-width packed-switch payload.
    PackedSwitchPayload,
    /// Variable-width sparse-switch payload.
    SparseSwitchPayload,
    /// Variable-width array-data payload.
    ArrayDataPayload,
}

impl Format {
    /// Returns the fixed width in 16-bit code units, or `None` for payloads.
    #[must_use]
    pub const fn width(self) -> Option<u8> {
        match self {
            Self::F10x | Self::F12x | Self::F11n | Self::F11x | Self::F10t => Some(1),
            Self::F20t
            | Self::F22x
            | Self::F21t
            | Self::F21s
            | Self::F21h
            | Self::F21c
            | Self::F23x
            | Self::F22b
            | Self::F22t
            | Self::F22s
            | Self::F22c => Some(2),
            Self::F30t
            | Self::F31i
            | Self::F31t
            | Self::F31c
            | Self::F32x
            | Self::F35c
            | Self::F3rc => Some(3),
            Self::F45cc | Self::F4rcc => Some(4),
            Self::F51l => Some(5),
            Self::PackedSwitchPayload | Self::SparseSwitchPayload | Self::ArrayDataPayload => None,
        }
    }
}

/// Broad operation family used for coverage and later lifting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SemanticClass {
    /// Constant materialization.
    Const,
    /// Register movement.
    Move,
    /// Arithmetic or bitwise operation.
    Arith,
    /// Comparison.
    Compare,
    /// Conditional or unconditional branch.
    Branch,
    /// Switch dispatch.
    Switch,
    /// Method or call-site invocation.
    Invoke,
    /// Instance or static field access.
    Field,
    /// Array access or data fill.
    Array,
    /// Allocation.
    New,
    /// Type conversion or check.
    Cast,
    /// Monitor operation.
    Monitor,
    /// Method return.
    Return,
    /// Exception throw.
    Throw,
    /// Data payload.
    Payload,
    /// Debugger-only operation.
    DebugOp,
    /// No operation.
    Nop,
}

/// Metadata for one supported portable opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpcodeInfo {
    /// Opcode byte.
    pub opcode: u8,
    /// Canonical Dalvik mnemonic.
    pub name: &'static str,
    /// Operand encoding.
    pub format: Format,
    /// Broad semantic family.
    pub class: SemanticClass,
}

/// Supported opcode metadata, an explicit reserved value, or a payload marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Opcode {
    /// Portable DEX instruction.
    Supported(OpcodeInfo),
    /// Reserved or unassigned opcode byte.
    Unsupported {
        /// Reserved opcode byte.
        opcode: u8,
    },
    /// Payload pseudo-instruction identified by its full first code unit.
    Payload {
        /// Payload identifier (`0x0100`, `0x0200`, or `0x0300`).
        ident: u16,
    },
}

const fn info(opcode: u8, name: &'static str, format: Format, class: SemanticClass) -> Opcode {
    Opcode::Supported(OpcodeInfo {
        opcode,
        name,
        format,
        class,
    })
}

#[rustfmt::skip]
const MOVE: [&str; 13] = ["move", "move/from16", "move/16", "move-wide", "move-wide/from16", "move-wide/16", "move-object", "move-object/from16", "move-object/16", "move-result", "move-result-wide", "move-result-object", "move-exception"];
#[rustfmt::skip]
const CONST: [&str; 11] = ["const/4", "const/16", "const", "const/high16", "const-wide/16", "const-wide/32", "const-wide", "const-wide/high16", "const-string", "const-string/jumbo", "const-class"];
#[rustfmt::skip]
const CMP: [&str; 5] = ["cmpl-float", "cmpg-float", "cmpl-double", "cmpg-double", "cmp-long"];
#[rustfmt::skip]
const IF: [&str; 12] = ["if-eq", "if-ne", "if-lt", "if-ge", "if-gt", "if-le", "if-eqz", "if-nez", "if-ltz", "if-gez", "if-gtz", "if-lez"];
#[rustfmt::skip]
const ARRAY: [&str; 14] = ["aget", "aget-wide", "aget-object", "aget-boolean", "aget-byte", "aget-char", "aget-short", "aput", "aput-wide", "aput-object", "aput-boolean", "aput-byte", "aput-char", "aput-short"];
#[rustfmt::skip]
const FIELD: [&str; 28] = ["iget", "iget-wide", "iget-object", "iget-boolean", "iget-byte", "iget-char", "iget-short", "iput", "iput-wide", "iput-object", "iput-boolean", "iput-byte", "iput-char", "iput-short", "sget", "sget-wide", "sget-object", "sget-boolean", "sget-byte", "sget-char", "sget-short", "sput", "sput-wide", "sput-object", "sput-boolean", "sput-byte", "sput-char", "sput-short"];
#[rustfmt::skip]
const INVOKE: [&str; 10] = ["invoke-virtual", "invoke-super", "invoke-direct", "invoke-static", "invoke-interface", "invoke-virtual/range", "invoke-super/range", "invoke-direct/range", "invoke-static/range", "invoke-interface/range"];
#[rustfmt::skip]
const CAST: [&str; 21] = ["neg-int", "not-int", "neg-long", "not-long", "neg-float", "neg-double", "int-to-long", "int-to-float", "int-to-double", "long-to-int", "long-to-float", "long-to-double", "float-to-int", "float-to-long", "float-to-double", "double-to-int", "double-to-long", "double-to-float", "int-to-byte", "int-to-char", "int-to-short"];
#[rustfmt::skip]
const ARITH: [&str; 83] = ["add-int", "sub-int", "mul-int", "div-int", "rem-int", "and-int", "or-int", "xor-int", "shl-int", "shr-int", "ushr-int", "add-long", "sub-long", "mul-long", "div-long", "rem-long", "and-long", "or-long", "xor-long", "shl-long", "shr-long", "ushr-long", "add-float", "sub-float", "mul-float", "div-float", "rem-float", "add-double", "sub-double", "mul-double", "div-double", "rem-double", "add-int/2addr", "sub-int/2addr", "mul-int/2addr", "div-int/2addr", "rem-int/2addr", "and-int/2addr", "or-int/2addr", "xor-int/2addr", "shl-int/2addr", "shr-int/2addr", "ushr-int/2addr", "add-long/2addr", "sub-long/2addr", "mul-long/2addr", "div-long/2addr", "rem-long/2addr", "and-long/2addr", "or-long/2addr", "xor-long/2addr", "shl-long/2addr", "shr-long/2addr", "ushr-long/2addr", "add-float/2addr", "sub-float/2addr", "mul-float/2addr", "div-float/2addr", "rem-float/2addr", "add-double/2addr", "sub-double/2addr", "mul-double/2addr", "div-double/2addr", "rem-double/2addr", "add-int/lit16", "rsub-int", "mul-int/lit16", "div-int/lit16", "rem-int/lit16", "and-int/lit16", "or-int/lit16", "xor-int/lit16", "add-int/lit8", "rsub-int/lit8", "mul-int/lit8", "div-int/lit8", "rem-int/lit8", "and-int/lit8", "or-int/lit8", "xor-int/lit8", "shl-int/lit8", "shr-int/lit8", "ushr-int/lit8"];

/// Returns complete metadata for any opcode byte.
#[must_use]
#[rustfmt::skip]
pub const fn opcode(op: u8) -> Opcode {
    let (name, format, class) = match op {
        0x00 => ("nop", Format::F10x, SemanticClass::Nop),
        0x01..=0x0d => (MOVE[(op - 1) as usize], match op { 0x01 | 0x04 | 0x07 => Format::F12x, 0x02 | 0x05 | 0x08 => Format::F22x, 0x03 | 0x06 | 0x09 => Format::F32x, _ => Format::F11x }, SemanticClass::Move),
        0x0e => ("return-void", Format::F10x, SemanticClass::Return),
        0x0f..=0x11 => (["return", "return-wide", "return-object"][(op - 0x0f) as usize], Format::F11x, SemanticClass::Return),
        0x12..=0x1c => (CONST[(op - 0x12) as usize], [Format::F11n, Format::F21s, Format::F31i, Format::F21h, Format::F21s, Format::F31i, Format::F51l, Format::F21h, Format::F21c, Format::F31c, Format::F21c][(op - 0x12) as usize], SemanticClass::Const),
        0x1d..=0x1e => (["monitor-enter", "monitor-exit"][(op - 0x1d) as usize], Format::F11x, SemanticClass::Monitor),
        0x1f => ("check-cast", Format::F21c, SemanticClass::Cast), 0x20 => ("instance-of", Format::F22c, SemanticClass::Cast),
        0x21 => ("array-length", Format::F12x, SemanticClass::Array), 0x22 => ("new-instance", Format::F21c, SemanticClass::New), 0x23 => ("new-array", Format::F22c, SemanticClass::New),
        0x24 => ("filled-new-array", Format::F35c, SemanticClass::New), 0x25 => ("filled-new-array/range", Format::F3rc, SemanticClass::New), 0x26 => ("fill-array-data", Format::F31t, SemanticClass::Array),
        0x27 => ("throw", Format::F11x, SemanticClass::Throw),
        0x28..=0x2a => (["goto", "goto/16", "goto/32"][(op - 0x28) as usize], [Format::F10t, Format::F20t, Format::F30t][(op - 0x28) as usize], SemanticClass::Branch),
        0x2b..=0x2c => (["packed-switch", "sparse-switch"][(op - 0x2b) as usize], Format::F31t, SemanticClass::Switch),
        0x2d..=0x31 => (CMP[(op - 0x2d) as usize], Format::F23x, SemanticClass::Compare),
        0x32..=0x3d => (IF[(op - 0x32) as usize], if op <= 0x37 { Format::F22t } else { Format::F21t }, SemanticClass::Branch),
        0x44..=0x51 => (ARRAY[(op - 0x44) as usize], Format::F23x, SemanticClass::Array),
        0x52..=0x6d => (FIELD[(op - 0x52) as usize], if op <= 0x5f { Format::F22c } else { Format::F21c }, SemanticClass::Field),
        0x6e..=0x72 => (INVOKE[(op - 0x6e) as usize], Format::F35c, SemanticClass::Invoke),
        0x74..=0x78 => (INVOKE[(op - 0x74 + 5) as usize], Format::F3rc, SemanticClass::Invoke),
        0x7b..=0x8f => (CAST[(op - 0x7b) as usize], Format::F12x, if op <= 0x80 { SemanticClass::Arith } else { SemanticClass::Cast }),
        0x90..=0xe2 => (ARITH[(op - 0x90) as usize], if op <= 0xaf { Format::F23x } else if op <= 0xcf { Format::F12x } else if op <= 0xd7 { Format::F22s } else { Format::F22b }, SemanticClass::Arith),
        0xfa => ("invoke-polymorphic", Format::F45cc, SemanticClass::Invoke), 0xfb => ("invoke-polymorphic/range", Format::F4rcc, SemanticClass::Invoke),
        0xfc => ("invoke-custom", Format::F35c, SemanticClass::Invoke), 0xfd => ("invoke-custom/range", Format::F3rc, SemanticClass::Invoke),
        0xfe => ("const-method-handle", Format::F21c, SemanticClass::Const), 0xff => ("const-method-type", Format::F21c, SemanticClass::Const),
        _ => return Opcode::Unsupported { opcode: op },
    };
    info(op, name, format, class)
}
