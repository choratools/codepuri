//! DEX method flags translated into a Java declaration, before body lowering.
//!
//! `bridge` and `synthetic` are class-file metadata, not Java keywords. Keeping
//! them in the plan does not promise that recompilation reproduces those bits.

const PUBLIC: u32 = 0x1;
const PRIVATE: u32 = 0x2;
const PROTECTED: u32 = 0x4;
const STATIC: u32 = 0x8;
const FINAL: u32 = 0x10;
const SYNCHRONIZED: u32 = 0x20;
const BRIDGE: u32 = 0x40;
const VARARGS: u32 = 0x80;
const NATIVE: u32 = 0x100;
const ABSTRACT: u32 = 0x400;
const STRICT: u32 = 0x800;
const SYNTHETIC: u32 = 0x1000;
const CONSTRUCTOR: u32 = 0x10000;
const DECLARED_SYNCHRONIZED: u32 = 0x20000;
const KNOWN: u32 = PUBLIC
    | PRIVATE
    | PROTECTED
    | STATIC
    | FINAL
    | SYNCHRONIZED
    | BRIDGE
    | VARARGS
    | NATIVE
    | ABSTRACT
    | STRICT
    | SYNTHETIC
    | CONSTRUCTOR
    | DECLARED_SYNCHRONIZED;

/// The declaring type must agree with the compilation unit being emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodOwner {
    /// Ordinary class declaration.
    Class {
        /// Whether the declaring class permits abstract methods.
        is_abstract: bool,
    },
    /// Uses modern Java interface rules (default, static, and private methods).
    Interface,
    /// Annotation interface whose methods are annotation elements.
    Annotation,
}

/// Method category established from the original DEX name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodKind {
    /// Named method with ordinary declaration rules.
    Ordinary,
    /// Instance initializer named `<init>`.
    Constructor,
    /// Static initializer named `<clinit>`.
    ClassInitializer,
}

/// Metadata needed to validate method flags before body lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodFlagInput {
    /// Original encoded method access flags.
    pub access_flags: u32,
    /// Category and abstractness of the declaring type.
    pub owner: MethodOwner,
    /// Determine from the original DEX name, not a sanitized Java identifier.
    pub kind: MethodKind,
    /// Whether the DEX method has a code item.
    pub has_code: bool,
    /// True only if there is a last parameter and its original type is an array.
    pub last_parameter_is_array: bool,
}

/// Whether the declaration requires a body or a terminating semicolon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodBody {
    /// A concrete method must be emitted with a body.
    Required,
    /// Emit a semicolon; do not ask the body backend to fabricate a body.
    Forbidden,
}

/// Validated source modifiers and retained class-file metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDeclarationPlan {
    /// Ordered Java keywords, without trailing whitespace.
    pub modifiers: Vec<&'static str>,
    /// Body requirement determined by native and abstract flags.
    pub body: MethodBody,
    /// Render the last array parameter as its component type followed by `...`.
    pub varargs: bool,
    /// Original bridge metadata; not a Java source keyword.
    pub bridge: bool,
    /// Original synthetic metadata; not a Java source keyword.
    pub synthetic: bool,
    /// Whether the Java declaration must acquire the instance/class monitor.
    /// For managed DEX methods, the body lowerer must recognize the matching
    /// method-wide monitor scaffold before removing it. This flag alone is not
    /// permission to discard arbitrary monitor instructions or to claim a body
    /// is supported; unrelated synchronized blocks still need normal lowering.
    pub synchronized: bool,
}

/// Reason method metadata cannot form a supported Java declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodFlagError(
    /// Static explanation of the rejected flag combination or metadata.
    pub &'static str,
);

impl std::fmt::Display for MethodFlagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for MethodFlagError {}

/// Validate ordinary method flags and plan an honest Java declaration.
/// Constructors and class initializers have different declaration rules and
/// deliberately go to their own lowering path.
pub fn plan_method_declaration(
    input: MethodFlagInput,
) -> Result<MethodDeclarationPlan, MethodFlagError> {
    let flags = input.access_flags;
    let has = |mask| flags & mask != 0;
    let reject = |reason| Err(MethodFlagError(reason));
    if input.kind != MethodKind::Ordinary {
        return reject("constructor/class initializer requires separate lowering");
    }
    if flags & !KNOWN != 0 {
        return reject("unknown or non-method access flags");
    }
    if has(CONSTRUCTOR) {
        return reject("constructor flag on ordinary method");
    }
    if (flags & (PUBLIC | PRIVATE | PROTECTED)).count_ones() > 1 {
        return reject("contradictory method visibility");
    }
    let abstract_method = has(ABSTRACT);
    let native = has(NATIVE);
    if native && has(STRICT) {
        return reject("native method cannot be strictfp");
    }
    if abstract_method
        && has(PRIVATE | STATIC | FINAL | NATIVE | SYNCHRONIZED | STRICT | DECLARED_SYNCHRONIZED)
    {
        return reject("abstract method has incompatible modifiers");
    }
    // DEX ACC_SYNCHRONIZED is only valid for native methods. Managed methods
    // use ACC_DECLARED_SYNCHRONIZED, with monitor operations in their code.
    if has(SYNCHRONIZED) && !native {
        return reject("ACC_SYNCHRONIZED requires a native method");
    }
    if has(DECLARED_SYNCHRONIZED) && native {
        return reject("ACC_DECLARED_SYNCHRONIZED requires a non-native method");
    }
    if has(VARARGS) && !input.last_parameter_is_array {
        return reject("varargs method requires a final array parameter");
    }
    let body = if abstract_method || native {
        MethodBody::Forbidden
    } else {
        MethodBody::Required
    };
    if input.has_code != (body == MethodBody::Required) {
        return reject(if input.has_code {
            "native/abstract method must not have code"
        } else {
            "concrete method requires code"
        });
    }
    match input.owner {
        MethodOwner::Class { is_abstract: false } if abstract_method => {
            return reject("abstract method requires an abstract class");
        }
        MethodOwner::Interface | MethodOwner::Annotation => {
            if has(PROTECTED | FINAL | NATIVE | SYNCHRONIZED | DECLARED_SYNCHRONIZED) {
                return reject("modifier is not legal on an interface method");
            }
            if !has(PUBLIC | PRIVATE) {
                return reject("interface method requires public or private access");
            }
            if input.owner == MethodOwner::Annotation
                && (!abstract_method || has(PRIVATE | STATIC | VARARGS | BRIDGE))
            {
                return reject("annotation element requires a public abstract declaration");
            }
        }
        _ => {}
    }
    let mut modifiers = Vec::new();
    for (mask, keyword) in [
        (PUBLIC, "public"),
        (PRIVATE, "private"),
        (PROTECTED, "protected"),
        (STATIC, "static"),
        (FINAL, "final"),
        (ABSTRACT, "abstract"),
    ] {
        if has(mask) {
            modifiers.push(keyword);
        }
    }
    if input.owner == MethodOwner::Interface && !abstract_method && !has(STATIC | PRIVATE) {
        modifiers.push("default");
    }
    let synchronized = has(SYNCHRONIZED | DECLARED_SYNCHRONIZED);
    if synchronized {
        modifiers.push("synchronized");
    }
    if native {
        modifiers.push("native");
    }
    if has(STRICT) {
        modifiers.push("strictfp");
    }
    Ok(MethodDeclarationPlan {
        modifiers,
        body,
        varargs: has(VARARGS),
        bridge: has(BRIDGE),
        synthetic: has(SYNTHETIC),
        synchronized,
    })
}

#[cfg(test)]
#[path = "java_method_flags_tests.rs"]
mod tests;

/// Executable synchronization strategy, separate from reflective DEX metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynchronizationExecution {
    /// No method-level synchronization metadata is present.
    Ordinary,
    /// Native ACC_SYNCHRONIZED requires Java's implicit instance/class lock.
    NativeImplicitMonitor,
    /// Managed ACC_DECLARED_SYNCHRONIZED has no execution effect. Preserve all
    /// original monitor instructions through normal body lowering, without
    /// adding an implicit Java monitor or deleting/hoisting the explicit ones.
    ExplicitBodyMonitors,
}

/// Body-safe modifiers; this is not a proof that the body can be emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodBodyModifiers {
    /// Source modifiers preserving executable locking semantics.
    pub modifiers: Vec<&'static str>,
    /// Synchronization strategy required by original DEX semantics.
    pub execution: SynchronizationExecution,
    /// Managed declaration metadata is retained here, but a non-synchronized
    /// Java declaration cannot reproduce its reflective synchronized modifier.
    pub declared_synchronized_metadata: bool,
}

/// Validate original flags and select execution-preserving source modifiers.
///
/// ACC_DECLARED_SYNCHRONIZED (0x20000) is reflection-only DEX metadata;
/// ACC_SYNCHRONIZED (0x20) acquires a monitor and is legal only on native methods.
/// The body backend must still prove every explicit monitor operation and
/// exceptional release. Unsupported bodies remain unavailable. This helper
/// never attests that a scaffold spans the entire method, and never removes it.
/// See the AOSP Dalvik executable format access_flags table.
pub fn plan_body_modifiers(input: MethodFlagInput) -> Result<MethodBodyModifiers, MethodFlagError> {
    let mut plan = plan_method_declaration(input)?;
    let execution = if input.access_flags & SYNCHRONIZED != 0 {
        SynchronizationExecution::NativeImplicitMonitor
    } else if input.access_flags & DECLARED_SYNCHRONIZED != 0 {
        plan.modifiers
            .retain(|modifier| *modifier != "synchronized");
        SynchronizationExecution::ExplicitBodyMonitors
    } else {
        SynchronizationExecution::Ordinary
    };
    Ok(MethodBodyModifiers {
        modifiers: plan.modifiers,
        execution,
        declared_synchronized_metadata: input.access_flags & DECLARED_SYNCHRONIZED != 0,
    })
}

/// Exact empty managed method scaffold whose lock spans its whole body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptySynchronizedBody {
    /// The original method returns void after releasing its instance lock.
    Void,
    /// The original method returns the unchanged receiver after release.
    Receiver,
}

/// Prove the narrow `monitor-enter this; monitor-exit this; return` shape.
/// This permits replacing precisely those two instructions with Java's method
/// modifier. No potentially throwing/effectful body operation, extra code,
/// exception handler, alternate lock or static receiver is accepted.
pub fn empty_instance_monitor_scaffold(
    input: MethodFlagInput,
    code: &dex_format::code::CodeItem,
    return_type: &backend_java::JavaType,
    receiver_type: &backend_java::JavaType,
    resolver: &impl backend_java::JavaResolver,
) -> Result<EmptySynchronizedBody, MethodFlagError> {
    plan_method_declaration(input)?;
    if input.access_flags & DECLARED_SYNCHRONIZED == 0
        || input.access_flags & STATIC != 0
        || !code.tries.is_empty()
        || code.ins_size == 0
    {
        return Err(MethodFlagError("not an empty instance monitor scaffold"));
    }
    let receiver = code
        .registers_size
        .checked_sub(code.ins_size)
        .ok_or(MethodFlagError("invalid incoming register frame"))?;
    let insns = dex_decompiler::decode_method(code)
        .map_err(|_| MethodFlagError("invalid monitor scaffold instructions"))?;
    let [enter, exit, ret] = insns.as_slice() else {
        return Err(MethodFlagError("monitor scaffold has extra operations"));
    };
    let opcode = |insn: &dex_decompiler::DecodedInsn| match insn.opcode {
        dex_decompiler::decode::Opcode::Supported(op) => Some(op.opcode),
        _ => None,
    };
    if opcode(enter) != Some(0x1d)
        || opcode(exit) != Some(0x1e)
        || enter.registers != [receiver]
        || exit.registers != [receiver]
    {
        return Err(MethodFlagError(
            "monitor scaffold uses a different receiver",
        ));
    }
    match opcode(ret) {
        Some(0x0e) if *return_type == backend_java::JavaType::Void => {
            Ok(EmptySynchronizedBody::Void)
        }
        Some(0x11)
            if ret.registers == [receiver] && resolver.assignable(receiver_type, return_type) =>
        {
            Ok(EmptySynchronizedBody::Receiver)
        }
        _ => Err(MethodFlagError(
            "monitor scaffold return lacks exact type proof",
        )),
    }
}
