//! Erased Java signatures from descriptors and DEX metadata.
//! Class descriptors describe names, not proof of local class membership or nesting.

use backend_java::{JavaMethod, JavaType, Unsupported, sanitize_identifier};
use dex_format::DexFile;

/// A full erased method descriptor, excluding the receiver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodDescriptor {
    /// Erased parameter types in declaration order, excluding the receiver.
    pub parameters: Vec<JavaType>,
    /// Erased return type, including `void`.
    pub return_type: JavaType,
}

fn invalid(message: impl Into<String>) -> Unsupported {
    Unsupported::Metadata(message.into())
}

/// Validate an exact source identifier without silently renaming a referenced symbol.
/// Declaration writers may use `sanitize_identifier` separately for source hints.
pub fn source_identifier(name: &str) -> Result<String, Unsupported> {
    if !name.is_empty() && sanitize_identifier(name) == name {
        Ok(name.into())
    } else {
        Err(invalid(format!("unsupported source identifier {name:?}")))
    }
}

fn take_type(input: &str, allow_void: bool) -> Result<(JavaType, &str), Unsupported> {
    let rank = input.bytes().take_while(|b| *b == b'[').count();
    if rank > 255 {
        return Err(invalid("array rank exceeds 255"));
    }
    let base = &input[rank..];
    let (mut ty, rest) = match base.as_bytes().first().copied() {
        Some(b'L') => {
            let end = base
                .find(';')
                .ok_or_else(|| invalid("unterminated class descriptor"))?;
            let body = &base[1..end];
            let name = body
                .split('/')
                .map(source_identifier)
                .collect::<Result<Vec<_>, _>>()?
                .join(".");
            (JavaType::Class(name), &base[end + 1..])
        }
        Some(tag) => {
            let ty = match tag {
                b'V' if allow_void && rank == 0 => JavaType::Void,
                b'Z' => JavaType::Boolean,
                b'B' => JavaType::Byte,
                b'S' => JavaType::Short,
                b'C' => JavaType::Char,
                b'I' => JavaType::Int,
                b'J' => JavaType::Long,
                b'F' => JavaType::Float,
                b'D' => JavaType::Double,
                _ => return Err(invalid(format!("invalid type descriptor {input:?}"))),
            };
            (ty, &base[1..])
        }
        None => return Err(invalid("missing descriptor type")),
    };
    for _ in 0..rank {
        ty = JavaType::Array(Box::new(ty));
    }
    Ok((ty, rest))
}

/// Parse exactly one descriptor. Void is supported only as a scalar return type.
pub fn parse_type_descriptor(descriptor: &str) -> Result<JavaType, Unsupported> {
    let (ty, rest) = take_type(descriptor, true)?;
    if !rest.is_empty() {
        return Err(invalid("trailing type descriptor data"));
    }
    Ok(ty)
}

/// Compatibility hook for resolvers whose metadata interface returns `Option`.
pub fn descriptor_type(descriptor: &str) -> Option<JavaType> {
    parse_type_descriptor(descriptor).ok()
}

/// Parse a complete method descriptor, retaining parameter order and array rank.
/// Generic arguments are absent from descriptors; class types remain erased.
pub fn parse_method_descriptor(descriptor: &str) -> Result<MethodDescriptor, Unsupported> {
    let mut rest = descriptor
        .strip_prefix('(')
        .ok_or_else(|| invalid("missing method descriptor opening parenthesis"))?;
    let mut parameters = Vec::new();
    while !rest.starts_with(')') {
        let (ty, next) = take_type(rest, false)?;
        parameters.push(ty);
        rest = next;
    }
    let return_type = parse_type_descriptor(&rest[1..])?;
    Ok(MethodDescriptor {
        parameters,
        return_type,
    })
}

impl MethodDescriptor {
    /// Check a caller's logical argument count (wide primitives count once).
    pub fn validate_arity(&self, expected: usize) -> Result<(), Unsupported> {
        if self.parameters.len() == expected {
            Ok(())
        } else {
            Err(invalid(format!(
                "signature arity mismatch: expected {expected}, descriptor has {}",
                self.parameters.len()
            )))
        }
    }
}

/// Resolve a type index in its owning DEX type pool, including external classes,
/// primitives and arrays. A class-definition lookup is intentionally unnecessary.
pub fn type_from_pool(dex: &DexFile, index: u32) -> Result<JavaType, Unsupported> {
    let id = dex.type_id(index).map_err(|e| invalid(e.to_string()))?;
    let descriptor = dex
        .string(id.descriptor_idx)
        .map_err(|e| invalid(e.to_string()))?;
    parse_type_descriptor(descriptor)
}

/// Construct a signature directly from a method/prototype pool entry.
/// The caller supplies independently established dispatch metadata; this function
/// does not infer static dispatch from class membership or an invocation site.
pub fn method_from_pool(
    dex: &DexFile,
    index: u32,
    is_static: bool,
) -> Result<JavaMethod, Unsupported> {
    let method = dex.method_id(index).map_err(|e| invalid(e.to_string()))?;
    let proto = dex
        .proto_id(u32::from(method.proto_idx))
        .map_err(|e| invalid(e.to_string()))?;
    let JavaType::Class(owner) = type_from_pool(dex, u32::from(method.class_idx))? else {
        return Err(invalid("method owner is not a class"));
    };
    let raw_name = dex
        .string(method.name_idx)
        .map_err(|e| invalid(e.to_string()))?;
    let name = if raw_name == "<init>" || raw_name == "<clinit>" {
        raw_name.into()
    } else {
        source_identifier(raw_name)?
    };
    let parameters = proto
        .parameters
        .as_ref()
        .map(|list| {
            list.types
                .iter()
                .map(|i| {
                    let ty = type_from_pool(dex, u32::from(*i))?;
                    if ty == JavaType::Void {
                        Err(invalid("void parameter"))
                    } else {
                        Ok(ty)
                    }
                })
                .collect::<Result<Vec<_>, Unsupported>>()
        })
        .transpose()?
        .unwrap_or_default();
    let return_type = type_from_pool(dex, proto.return_type_idx)?;
    if (raw_name == "<init>" && (is_static || return_type != JavaType::Void))
        || (raw_name == "<clinit>"
            && (!is_static || return_type != JavaType::Void || !parameters.is_empty()))
    {
        return Err(invalid(
            "invalid constructor or class initializer signature",
        ));
    }
    Ok(JavaMethod {
        owner,
        name,
        is_static,
        return_type,
        parameters,
        throws: vec![],
    })
}

#[cfg(test)]
#[path = "java_descriptors_tests.rs"]
mod tests;
