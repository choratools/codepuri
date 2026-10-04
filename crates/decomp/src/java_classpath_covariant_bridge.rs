//! Byte-derived, straight-through JVM return-covariance bridge evidence.
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BridgeOrigin {
    class_flags: u32,
    major_version: u16,
    class_signature: Option<String>,
    declaration: Declaration,
    bridge: ParsedMethod,
    target: ParsedMethod,
}

impl ClassMetadata {
    /// Prove that unchanged original bytes load `this` and every exact incoming
    /// argument, invoke the same owner's genuine nonbridge declaration, then
    /// return that result. This is body evidence, not a flag-only assertion.
    /// The caller still must establish reference covariance and source lookup.
    pub fn attests_covariant_bridge(&self, bridge: &MethodKey, target: &MethodKey) -> bool {
        let Some(origin) = self.bridge_origins.get(bridge) else {
            return false;
        };
        self.descriptor == bridge.owner
            && self.access_flags == origin.class_flags
            && self.major_version == origin.major_version
            && self.signature == origin.class_signature
            && self.declaration == origin.declaration
            && origin.target.key == *target
            && self.methods.iter().filter(|m| m.key == *bridge).count() == 1
            && self.methods.iter().filter(|m| m.key == *target).count() == 1
            && self.methods.iter().any(|m| m == &origin.bridge)
            && self.methods.iter().any(|m| m == &origin.target)
    }
}

fn reference(descriptor: &str) -> bool {
    descriptor.starts_with('L') || descriptor.starts_with('[')
}

fn load(bytes: &[u8], pos: &mut usize, opcode: u8, slot: u16) -> Option<()> {
    let compact = match opcode {
        0x15 => 0x1a,
        0x16 => 0x1e,
        0x17 => 0x22,
        0x18 => 0x26,
        0x19 => 0x2a,
        _ => return None,
    };
    if slot <= 3 && bytes.get(*pos) == Some(&(compact + slot as u8)) {
        *pos += 1;
    } else if slot <= 255 && bytes.get(*pos..*pos + 2) == Some(&[opcode, slot as u8]) {
        *pos += 2;
    } else if bytes.get(*pos..*pos + 4) == Some(&[0xc4, opcode, (slot >> 8) as u8, slot as u8]) {
        *pos += 4;
    } else {
        return None;
    }
    Some(())
}

fn target(class: &ClassMetadata, cp: &[Cp], bridge: &ParsedMethod) -> Option<ParsedMethod> {
    let flags = bridge.metadata.access_flags;
    if flags & 0x1040 != 0x1040
        || flags & (2 | 8 | 0x100 | 0x400) != 0
        || bridge.key.owner != class.descriptor
        || !reference(&bridge.key.return_type)
    {
        return None;
    }
    let code = bridge.code.as_deref()?;
    if !code.exceptions.is_empty() {
        return None;
    }
    let mut pos = 0;
    load(&code.bytes, &mut pos, 0x19, 0)?;
    let mut slot = 1_u16;
    for parameter in &bridge.key.parameters {
        let (opcode, words) = match parameter.as_bytes().first()? {
            b'Z' | b'B' | b'C' | b'S' | b'I' => (0x15, 1),
            b'J' => (0x16, 2),
            b'F' => (0x17, 1),
            b'D' => (0x18, 2),
            b'L' | b'[' => (0x19, 1),
            _ => return None,
        };
        load(&code.bytes, &mut pos, opcode, slot)?;
        slot = slot.checked_add(words)?;
    }
    if code.max_locals < slot || code.max_stack < slot {
        return None;
    }
    let [0xb6, hi, lo, 0xb0] = code.bytes.get(pos..)? else {
        return None;
    };
    let Some(Cp::Ref(10, owner, name_type)) = cp.get(usize::from(u16::from_be_bytes([*hi, *lo])))
    else {
        return None;
    };
    let Some(Cp::Ref(12, name, descriptor)) = cp.get(usize::from(*name_type)) else {
        return None;
    };
    let (parameters, return_type) = method_descriptor(utf(cp, *descriptor).ok()?).ok()?;
    let key = MethodKey {
        owner: format!("L{};", internal(cp, *owner).ok()?),
        name: utf(cp, *name).ok()?.into(),
        parameters,
        return_type,
    };
    if key.owner != class.descriptor
        || key.name != bridge.key.name
        || key.parameters != bridge.key.parameters
        || key.return_type == bridge.key.return_type
        || !reference(&key.return_type)
    {
        return None;
    }
    let mut targets = class.methods.iter().filter(|m| m.key == key);
    let target = targets.next()?;
    if targets.next().is_some()
        || target.metadata.access_flags & (2 | 8 | 0x40 | 0x100 | 0x400) != 0
    {
        return None;
    }
    Some(target.clone())
}

pub(super) fn origins(class: &ClassMetadata, cp: &[Cp]) -> BTreeMap<MethodKey, BridgeOrigin> {
    class
        .methods
        .iter()
        .filter_map(|bridge| {
            let target = target(class, cp, bridge)?;
            Some((
                bridge.key.clone(),
                BridgeOrigin {
                    class_flags: class.access_flags,
                    major_version: class.major_version,
                    class_signature: class.signature.clone(),
                    declaration: class.declaration.clone(),
                    bridge: bridge.clone(),
                    target,
                },
            ))
        })
        .collect()
}
