//! Metadata-only Java declarations and conservative encoded initialization.
//! Successful declarations are still subsets, not whole-class recovery.
use crate::dex_java::descriptor_type;
use backend_java::JavaType;
use dex_format::{
    DexFile,
    class::{ClassDef, EncodedMethod},
};

#[path = "java_empty_type_publication.rs"]
pub mod empty_type_publication;

#[path = "java_enum_initialization.rs"]
pub mod enum_initialization;
#[path = "java_declaration_initializers.rs"]
pub mod initializers;

/// Source-level class category, derived from DEX flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassKind {
    /// Ordinary Java class.
    Class,
    /// Java interface.
    Interface,
    /// Java annotation interface.
    Annotation,
    /// Java enumeration.
    Enum,
}

/// Explicit limitations, rather than a misleading ordinary-class fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclarationUnsupported {
    /// Enum constructor reconstruction is not available.
    EnumConstructorsUnavailable,
    /// Java enum syntax cannot preserve the original constructor access flags.
    EnumConstructorAccessUnrepresentable {
        /// Exact original declaring DEX descriptor before source aliases.
        owner: String,
        /// Exact DEX-local constructor definition identity.
        method: u32,
        /// Original access flags; they have not been privatized for emission.
        access_flags: u32,
    },
    /// Access flags are contradictory or unsupported for this declaration.
    InvalidFlags,
    /// Referenced metadata is missing or cannot form a Java declaration.
    InvalidMetadata,
    /// The superclass is incompatible with the declaring type category.
    InvalidSuperclass,
    /// An annotation declares an interface other than the annotation marker.
    InvalidAnnotationInterfaces,
    /// The method does not satisfy the code-free abstract declaration rules.
    NotAbstractDeclaration,
    /// The annotation element has an invalid name, type, or parameters.
    InvalidAnnotationElement,
    /// Annotation defaults are present but cannot yet be emitted.
    AnnotationDefaultsUnavailable,
    /// A value is not a Java constant of the declared type.
    InvalidEncodedValue,
    /// Field initialization needs body-level definite-assignment evidence.
    FieldInitializationUnavailable,
    /// Atomic constructor source planning failed after field inventory proof.
    /// Original declaring descriptor and DEX-local method identities are retained.
    ConstructorPlanningUnavailable {
        /// Original declaring descriptor, before Java aliases.
        owner: String,
        /// Exact original constructor indices supplied to the family planner.
        methods: Vec<u32>,
        /// Unmodified debug form of the underlying backend limitation.
        reason: String,
    },
}

/// Determine category without treating annotation and enum bits as ordinary classes.
pub fn class_kind(flags: u32) -> Result<ClassKind, DeclarationUnsupported> {
    let interface = flags & 0x200 != 0;
    let annotation = flags & 0x2000 != 0;
    let enumeration = flags & 0x4000 != 0;
    if (annotation && !interface) || (enumeration && interface) {
        return Err(DeclarationUnsupported::InvalidFlags);
    }
    Ok(if annotation {
        ClassKind::Annotation
    } else if enumeration {
        ClassKind::Enum
    } else if interface {
        ClassKind::Interface
    } else {
        ClassKind::Class
    })
}

fn descriptor(dex: &DexFile, index: u32) -> Result<&str, DeclarationUnsupported> {
    dex.type_id(index)
        .ok()
        .and_then(|t| dex.string(t.descriptor_idx).ok())
        .ok_or(DeclarationUnsupported::InvalidMetadata)
}

fn ty(dex: &DexFile, index: u32) -> Result<JavaType, DeclarationUnsupported> {
    descriptor_type(descriptor(dex, index)?).ok_or(DeclarationUnsupported::InvalidMetadata)
}

fn class_name(dex: &DexFile, index: u32) -> Result<String, DeclarationUnsupported> {
    match ty(dex, index)? {
        JavaType::Class(name) => Ok(name),
        _ => Err(DeclarationUnsupported::InvalidMetadata),
    }
}

fn type_source(t: &JavaType) -> String {
    match t {
        JavaType::Void => "void".into(),
        JavaType::Boolean => "boolean".into(),
        JavaType::Byte => "byte".into(),
        JavaType::Short => "short".into(),
        JavaType::Char => "char".into(),
        JavaType::Int => "int".into(),
        JavaType::Long => "long".into(),
        JavaType::Float => "float".into(),
        JavaType::Double => "double".into(),
        JavaType::Class(n) => n.clone(),
        JavaType::Array(t) => format!("{}[]", type_source(t)),
    }
}

/// Header without package, opening brace or body. Top-level package/public access
/// is preserved; nested-only visibility is rejected rather than broadened.
pub fn class_declaration(
    dex: &DexFile,
    class: &ClassDef,
) -> Result<String, DeclarationUnsupported> {
    class_declaration_with(dex, class, |index| class_name(dex, index))
}

/// Validate a header using a frozen registry for all class names.
pub fn class_declaration_with(
    dex: &DexFile,
    class: &ClassDef,
    name: impl Fn(u32) -> Result<String, DeclarationUnsupported>,
) -> Result<String, DeclarationUnsupported> {
    let kind = class_kind(class.access_flags)?;
    if kind == ClassKind::Enum {
        return Err(DeclarationUnsupported::EnumConstructorsUnavailable);
    }
    let flags = class.access_flags;
    if flags & !(0x1 | 0x10 | 0x200 | 0x400 | 0x1000 | 0x2000) != 0
        || (flags & 0x10 != 0 && (flags & 0x400 != 0 || kind != ClassKind::Class))
    {
        return Err(DeclarationUnsupported::InvalidFlags);
    }
    let owner = name(class.class_idx)?;
    let simple = owner
        .rsplit('.')
        .next()
        .ok_or(DeclarationUnsupported::InvalidMetadata)?;
    let mut out = if flags & 1 != 0 {
        "public ".to_string()
    } else {
        String::new()
    };
    if kind == ClassKind::Class {
        if flags & 0x400 != 0 {
            out.push_str("abstract ");
        }
        if flags & 0x10 != 0 {
            out.push_str("final ");
        }
    }
    out.push_str(match kind {
        ClassKind::Class => "class ",
        ClassKind::Interface => "interface ",
        ClassKind::Annotation => "@interface ",
        ClassKind::Enum => unreachable!(),
    });
    out.push_str(simple);
    if class.superclass_idx != u32::MAX {
        let superclass = name(class.superclass_idx)?;
        if superclass != "java.lang.Object" {
            if kind != ClassKind::Class {
                return Err(DeclarationUnsupported::InvalidSuperclass);
            }
            out.push_str(&format!(" extends {superclass}"));
        }
    }
    let interfaces = class
        .interfaces
        .as_ref()
        .map(|list| {
            list.types
                .iter()
                .map(|i| name(u32::from(*i)))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    if kind == ClassKind::Annotation {
        if interfaces
            .iter()
            .any(|n| n != "java.lang.annotation.Annotation")
        {
            return Err(DeclarationUnsupported::InvalidAnnotationInterfaces);
        }
        initializers::annotation_defaults_with(dex, class, &name)?;
    } else if !interfaces.is_empty() {
        out.push_str(if kind == ClassKind::Interface {
            " extends "
        } else {
            " implements "
        });
        out.push_str(&interfaces.join(", "));
    }
    Ok(out)
}

fn annotation_type(dex: &DexFile, t: &JavaType) -> bool {
    match t {
        JavaType::Void => false,
        JavaType::Array(component) => {
            !matches!(**component, JavaType::Array(_)) && annotation_type(dex, component)
        }
        JavaType::Class(n) if n == "java.lang.String" || n == "java.lang.Class" => true,
        JavaType::Class(n) => dex.classes.iter().any(|c| {
            class_name(dex, c.class_idx).is_ok_and(|name| name == *n)
                && matches!(
                    class_kind(c.access_flags),
                    Ok(ClassKind::Enum | ClassKind::Annotation)
                )
        }),
        _ => true,
    }
}

/// Render a code-free abstract method or annotation element, terminated by `;`.
/// Concrete/static/default methods must use the body pipeline instead.
pub fn abstract_method_declaration(
    dex: &DexFile,
    class: &ClassDef,
    method: &EncodedMethod,
) -> Result<String, DeclarationUnsupported> {
    let kind = class_kind(class.access_flags)?;
    if kind == ClassKind::Enum {
        return Err(DeclarationUnsupported::EnumConstructorsUnavailable);
    }
    let f = method.access_flags;
    if (kind == ClassKind::Class
        && (class.access_flags & 0x400 == 0 || class.access_flags & 0x10 != 0))
        || method.code.is_some()
        || method.code_off != 0
        || f & 0x400 == 0
        || f & !(0x1 | 0x4 | 0x400 | 0x1000 | 0x40 | 0x80) != 0
        || f & 5 == 5
        || (kind != ClassKind::Class && f & 5 != 1)
    {
        return Err(DeclarationUnsupported::NotAbstractDeclaration);
    }
    let raw = dex
        .method_id(method.method_idx)
        .map_err(|_| DeclarationUnsupported::InvalidMetadata)?;
    if u32::from(raw.class_idx) != class.class_idx {
        return Err(DeclarationUnsupported::InvalidMetadata);
    }
    let name = dex
        .string(raw.name_idx)
        .map_err(|_| DeclarationUnsupported::InvalidMetadata)?;
    if name.is_empty() || backend_java::sanitize_identifier(name) != name {
        return Err(DeclarationUnsupported::InvalidMetadata);
    }
    let proto = dex
        .proto_id(u32::from(raw.proto_idx))
        .map_err(|_| DeclarationUnsupported::InvalidMetadata)?;
    let ret = ty(dex, proto.return_type_idx)?;
    let parameters = proto
        .parameters
        .as_ref()
        .map(|p| {
            p.types
                .iter()
                .map(|i| ty(dex, u32::from(*i)))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    if parameters.contains(&JavaType::Void) {
        return Err(DeclarationUnsupported::InvalidMetadata);
    }
    if kind == ClassKind::Annotation
        && (!parameters.is_empty()
            || !annotation_type(dex, &ret)
            || matches!(
                name,
                "getClass"
                    | "hashCode"
                    | "toString"
                    | "annotationType"
                    | "clone"
                    | "finalize"
                    | "notify"
                    | "notifyAll"
                    | "wait"
            ))
    {
        return Err(DeclarationUnsupported::InvalidAnnotationElement);
    }
    let access = if f & 1 != 0 {
        "public "
    } else if f & 4 != 0 {
        "protected "
    } else {
        ""
    };
    let abstract_modifier = if kind == ClassKind::Annotation {
        ""
    } else {
        "abstract "
    };
    let args = parameters
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{} p{i}", type_source(t)))
        .collect::<Vec<_>>()
        .join(", ");
    let default = if kind == ClassKind::Annotation {
        initializers::annotation_defaults(dex, class)?
            .remove(&method.method_idx)
            .map(|value| format!(" default {value}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    Ok(format!(
        "{access}{abstract_modifier}{} {name}({args}){default};",
        type_source(&ret)
    ))
}
