//! Conservative APK/DEX to Java subset integration. No unsupported body is emitted.
use crate::output::*;
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings};
use dex_format::DexFile;
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    value::ValueId,
};
use std::collections::{BTreeMap, BTreeSet};

#[path = "java_profile.rs"]
mod profile;
pub use profile::{BatchProfile, ClassProfile, JavaProfile};
pub(crate) use profile::{Timer, stage};

/// Parse exactly one DEX type descriptor, retaining literal `$` binary names.
pub fn descriptor_type(d: &str) -> Option<JavaType> {
    crate::java_descriptors::descriptor_type(d)
}

/// Metadata resolver for a single DEX pool. External dispatch flags are unknown
/// and rejected rather than guessed from callsites.
pub struct DexJavaResolver<'a> {
    /// Pool owning every resolved reference.
    pub dex: &'a DexFile,
}
impl DexJavaResolver<'_> {
    fn ty(&self, i: u32) -> Option<JavaType> {
        descriptor_type(
            self.dex
                .string(self.dex.type_id(i).ok()?.descriptor_idx)
                .ok()?,
        )
    }
    fn method_flags(&self, i: u32) -> Option<u32> {
        self.dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
            .find(|m| m.method_idx == i)
            .map(|m| m.access_flags)
    }
}
impl JavaResolver for DexJavaResolver<'_> {
    fn class(&self, r: ClassRef) -> Option<String> {
        match self.ty(r.index())? {
            JavaType::Class(n) => Some(n),
            _ => None,
        }
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        let mut i = self.dex.string_ids.get(r.index() as usize)?.string_data_off as usize;
        let bytes = self.dex.raw_bytes();
        while *bytes.get(i)? & 0x80 != 0 {
            i += 1;
        }
        i += 1;
        let mut units = vec![];
        loop {
            let b = *bytes.get(i)?;
            i += 1;
            if b == 0 {
                break;
            }
            let v = if b < 0x80 {
                u16::from(b)
            } else if b & 0xe0 == 0xc0 {
                let c = *bytes.get(i)?;
                i += 1;
                (u16::from(b & 31) << 6) | u16::from(c & 63)
            } else if b & 0xf0 == 0xe0 {
                let c = *bytes.get(i)?;
                let d = *bytes.get(i + 1)?;
                i += 2;
                (u16::from(b & 15) << 12) | (u16::from(c & 63) << 6) | u16::from(d & 63)
            } else {
                return None;
            };
            units.push(v);
        }
        Some(units)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let m = self.dex.method_id(r.index()).ok()?;
        let p = self.dex.proto_id(u32::from(m.proto_idx)).ok()?;
        Some(JavaMethod {
            owner: self.class(ClassRef::new(u32::from(m.class_idx)))?,
            name: self.dex.string(m.name_idx).ok()?.into(),
            is_static: self.method_flags(r.index())? & 8 != 0,
            return_type: self.ty(p.return_type_idx)?,
            parameters: p
                .parameters
                .as_ref()
                .map(|p| {
                    p.types
                        .iter()
                        .map(|t| self.ty(u32::from(*t)))
                        .collect::<Option<Vec<_>>>()
                })
                .unwrap_or(Some(vec![]))?,
            throws: vec![],
        })
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        let f = self.dex.field_id(r.index()).ok()?;
        let is_static = self
            .dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .find_map(|d| {
                if d.static_fields.iter().any(|f| f.field_idx == r.index()) {
                    Some(true)
                } else if d.instance_fields.iter().any(|f| f.field_idx == r.index()) {
                    Some(false)
                } else {
                    None
                }
            })?;
        Some(JavaField {
            owner: self.class(ClassRef::new(u32::from(f.class_idx)))?,
            name: self.dex.string(f.name_idx).ok()?.into(),
            ty: self.ty(u32::from(f.type_idx))?,
            is_static,
        })
    }
}

/// Map incoming register words to synthetic initial definitions, never to a
/// later assignment. Category-2 parameters must share one identity in both words.
pub fn parameter_bindings(
    ir: &MethodIr<SsaForm>,
    m: &JavaMethod,
    registers: u16,
    incoming: u16,
) -> Result<ParameterBindings, String> {
    let width = |t: &JavaType| {
        if matches!(t, JavaType::Long | JavaType::Double) {
            2u16
        } else {
            1
        }
    };
    let expected = m.parameters.iter().map(width).sum::<u16>() + u16::from(!m.is_static);
    if expected != incoming {
        return Err("incoming descriptor word count mismatch".into());
    }
    let mut reg = registers
        .checked_sub(incoming)
        .ok_or("incoming registers exceed frame")?;
    let initial = |r: u16| -> Result<ValueId, String> {
        let values = ir
            .register_slots()
            .values()
            .find(|s| s.index() == r)
            .ok_or_else(|| format!("missing incoming register {r}"))?;
        let candidates: Vec<_> = values.values().iter().copied().filter(|v| {
            ir.value(*v).ok().and_then(|v| ir.instruction(v.def()).ok()).is_some_and(|i| matches!(i, ManagedInsn::Opaque { family_tag: 0x8001, reads, writes, effects } if reads.is_empty() && writes.contains(v) && *effects == managed_ir::effects::EffectFlags::NONE))
        }).collect();
        if candidates.len() == 1 {
            Ok(candidates[0])
        } else {
            Err(format!("missing or ambiguous initial register {r}"))
        }
    };
    let mut b = ParameterBindings::default();
    if !m.is_static {
        b.receiver = Some(initial(reg)?);
        reg += 1;
    }
    for t in &m.parameters {
        let v = initial(reg)?;
        if width(t) == 2 && initial(reg + 1)? != v {
            return Err("split wide incoming parameter".into());
        }
        b.parameters.push(v);
        reg += width(t);
    }
    Ok(b)
}

/// Counts for selected classes; filtered/limited classes are counted separately.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JavaReport {
    /// Exact original method identities omitted with an unwritten compilation unit.
    /// Diagnostic evidence only; never a body, declaration or publication token.
    pub unpublished_method_reasons: BTreeMap<String, String>,
    /// Written subset units whose original initializer family is unavailable.
    pub unavailable_initialization_units: usize,
    /// Rejections separated by ordinary, constructor and static-initializer identity.
    pub category_reasons: BTreeMap<String, usize>,
    /// Explicit original constructors emitted with complete supported bodies.
    pub complete_constructors: usize,
    /// Complete supported original class initializer blocks.
    pub complete_static_initializers: usize,
    /// Actually emitted ordinary bodies with unavailable method declarations.
    pub semantic_bodies_with_unresolved_dependencies: usize,
    /// Actually emitted ordinary bodies outside legacy strict body closure.
    pub semantic_bodies_outside_strict_dependency_closure: usize,
    /// Genuine abstract/native declarations, never counted as complete bodies.
    pub declaration_only_methods: usize,
    /// Genuine required field declarations in accepted subset classes.
    pub fields_emitted: usize,
    /// Total class definitions encountered.
    pub classes_seen: usize,
    /// Subset compilation units accepted by the sink.
    pub classes_emitted: usize,
    /// Ordinary methods emitted with complete bodies.
    pub complete_methods: usize,
    /// Selected non-constructor methods with no source emitted.
    pub unsupported_methods: usize,
    /// Selected original constructors omitted from output.
    pub unavailable_constructors: usize,
    /// Classes excluded by filter or limit.
    pub skipped_classes: usize,
    /// Explicit per-method rejection reasons and counts.
    pub reasons: BTreeMap<String, usize>,
}
/// Selection controls for the development facade.
#[derive(Clone)]
pub struct JavaOptions {
    /// Substring of the genuine DEX class descriptor.
    pub class_filter: Option<String>,
    /// Original DEX descriptor prefixes selected for source export. Empty means
    /// all owners; retained input declarations remain available as metadata.
    pub source_prefixes: Vec<String>,
    /// Explicit original owner prefixes retained as input but not rendered.
    pub library_prefixes: Vec<String>,
    /// Exact original owners excluded from source. CLI population requires
    /// verified library implementation evidence; input metadata remains intact.
    pub library_owners: BTreeSet<String>,
    /// Maximum number of emitted subset compilation units.
    pub limit: Option<usize>,
    /// Maximum parallel class tasks; output callbacks remain ordered and serial.
    pub workers: usize,
    /// Required Java source release: 17 (default) or 25. Release 25 preserves
    /// proven constructor effects before the exact super/this invocation.
    pub source_release: u16,
    /// Retain immutable seeded SSA results for this export, sharing them across
    /// exception inspection, body emission and initialization proofs. Defaults
    /// to enabled; disable for memory-constrained callers or A/B measurement.
    pub cache_ssa: bool,
    /// Cache resolved Java method and field signatures within each immutable
    /// DEX resolver. Defaults to enabled; disable for A/B measurements.
    pub cache_resolutions: bool,
    /// Share immutable whole-input initialization capability evidence instead
    /// of cloning it for each class. Defaults to enabled.
    pub cache_inventory: bool,
    /// Opt-in measurement mode: skip redundant feasibility re-derivations
    /// (for example the discarded trial structuring before body emission) even
    /// where skipping can change unpublished-reason identity or publication
    /// outcomes. Defaults to false; default-mode output is unaffected.
    pub fast_proofs: bool,
    /// Explicit local JVM declaration inputs; no ambient platform defaults.
    pub classpath: Vec<std::path::PathBuf>,
    /// Remove standalone diagnostic comments from delivered Java and retain
    /// them losslessly in source diagnostics. Defaults to disabled.
    pub compact_source: bool,
}
impl Default for JavaOptions {
    fn default() -> Self {
        Self {
            class_filter: None,
            source_prefixes: Vec::new(),
            library_prefixes: Vec::new(),
            library_owners: BTreeSet::new(),
            limit: None,
            workers: 1,
            source_release: 17,
            cache_ssa: true,
            cache_resolutions: true,
            cache_inventory: true,
            fast_proofs: false,
            classpath: Vec::new(),
            compact_source: false,
        }
    }
}

impl JavaOptions {
    pub(crate) fn selected_owner(&self, owner: &str) -> bool {
        self.class_filter
            .as_ref()
            .is_none_or(|filter| owner.contains(filter))
            && (self.source_prefixes.is_empty()
                || self
                    .source_prefixes
                    .iter()
                    .any(|prefix| owner.starts_with(prefix)))
            && !self
                .library_prefixes
                .iter()
                .any(|prefix| owner.starts_with(prefix))
            && !self.library_owners.contains(owner)
    }

    pub(crate) fn has_source_scope(&self) -> bool {
        self.class_filter.is_some()
            || !self.source_prefixes.is_empty()
            || !self.library_prefixes.is_empty()
            || !self.library_owners.is_empty()
    }
}

/// Stream supported bodies and metadata declarations with member closure.
/// Every compilation unit is explicitly a subset, not a reconstructed app.
pub fn emit_java(
    files: &[DexFile],
    options: &JavaOptions,
    sink: &mut impl SourceSink,
) -> Result<JavaReport, OutputError> {
    crate::java_integration::emit(files, options, sink, None)
}

/// Export with caller-owned elapsed timings. No library logging is performed.
/// The profile is reset on entry and includes completed work on returned errors.
pub fn emit_java_profiled(
    files: &[DexFile],
    options: &JavaOptions,
    sink: &mut impl SourceSink,
    profile: &mut JavaProfile,
) -> Result<JavaReport, OutputError> {
    *profile = JavaProfile::default();
    crate::java_integration::emit(files, options, sink, Some(profile))
}

#[cfg(test)]
mod tests {
    // Fixture construction and successful-result assertions intentionally panic on failure.
    #![allow(clippy::unwrap_used)]

    use super::*;
    use backend_java::SourceResult;
    fn code(words: &[u16], registers: u16, incoming: u16) -> dex_format::code::CodeItem {
        dex_format::code::CodeItem {
            registers_size: registers,
            ins_size: incoming,
            outs_size: 0,
            debug_info_off: 0,
            insns_size: words.len() as u32,
            insns: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
            tries: vec![],
            handlers: vec![],
            span: dex_format::RawSpan {
                offset: 0,
                size: (words.len() * 2) as u32,
            },
        }
    }
    fn signature(is_static: bool, parameters: Vec<JavaType>) -> JavaMethod {
        JavaMethod {
            owner: "fixture.Calculator".into(),
            name: "calculate".into(),
            is_static,
            return_type: JavaType::Int,
            parameters,
            throws: vec![],
        }
    }
    #[test]
    fn descriptors_are_exact_and_paths_genuine() {
        assert_eq!(
            descriptor_type("[[J"),
            Some(JavaType::Array(Box::new(JavaType::Array(Box::new(
                JavaType::Long
            )))))
        );
        assert_eq!(
            descriptor_type("Lp/Outer$Inner;"),
            Some(JavaType::Class("p.Outer$Inner".into()))
        );
        for invalid in ["[V", "II", "L../Bad;", "Lp/class;", "Lp/Bad", ""] {
            assert_eq!(descriptor_type(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn source_and_library_prefixes_use_original_owners_and_keep_default_scope() {
        let mut options = JavaOptions::default();
        assert!(options.selected_owner("Lthird/Library;"));
        assert!(!options.has_source_scope());
        options.source_prefixes = vec!["Lapp/".into(), "Lpeer/".into()];
        options.library_prefixes = vec!["Lapp/sdk/".into()];
        assert!(options.has_source_scope());
        assert!(options.selected_owner("Lapp/Activity;"));
        assert!(options.selected_owner("Lpeer/Helper;"));
        assert!(!options.selected_owner("Lapp/sdk/Client;"));
        assert!(!options.selected_owner("Lthird/Library;"));
        options.class_filter = Some("Activity".into());
        assert!(options.selected_owner("Lapp/Activity;"));
        assert!(!options.selected_owner("Lpeer/Helper;"));
    }

    #[test]
    fn exact_library_owners_do_not_exclude_neighbors_or_default_scope() {
        let mut options = JavaOptions::default();
        assert!(!options.has_source_scope());
        options
            .library_owners
            .insert("Lcom/google/internal/Verified;".into());
        assert!(options.has_source_scope());
        assert!(!options.selected_owner("Lcom/google/internal/Verified;"));
        assert!(options.selected_owner("Lcom/google/internal/VerifiedNeighbor;"));
        assert!(options.selected_owner("Lcom/google/internal/Application;"));
    }
    #[test]
    fn incoming_this_wide_and_narrow_follow_histories() {
        // v3=this, v4/v5=long, v6=int. Read each before any assignment.
        let c = code(&[0x3007, 0x4104, 0x060f], 7, 4);
        let ssa = dex_decompiler::DexPasses::run_merged(&c).unwrap();
        let m = signature(false, vec![JavaType::Long, JavaType::Int]);
        let b = parameter_bindings(ssa.ir(), &m, 7, 4).unwrap();
        let history = |r| {
            ssa.ir()
                .register_slots()
                .values()
                .find(|s| s.index() == r)
                .unwrap()
                .values()
        };
        assert!(history(3).contains(&b.receiver.unwrap()));
        assert!(history(4).contains(&b.parameters[0]));
        assert!(history(5).contains(&b.parameters[0]));
        assert!(history(6).contains(&b.parameters[1]));
        assert_ne!(b.parameters[0], b.parameters[1]);
        assert!(parameter_bindings(ssa.ir(), &m, 7, 3).is_err());
    }
    #[test]
    fn overwritten_argument_is_not_mistaken_for_initial() {
        let c = code(&[0x1012, 0x000f], 1, 1); // const/4 v0,1; return v0
        let ssa = dex_decompiler::DexPasses::run_merged(&c).unwrap();
        assert!(parameter_bindings(ssa.ir(), &signature(true, vec![JavaType::Int]), 1, 1).is_err());
    }
    #[test]
    fn static_arithmetic_runs_ssa_bindings_and_backend() {
        let dex = DexFile::from_bytes(include_bytes!(
            "../../../tests/corpus/dex/clean/classes.dex"
        ))
        .unwrap();
        let resolver = DexJavaResolver { dex: &dex };
        let c = code(&[0x0090, 0x0201, 0x000f], 3, 2); // add-int v0,v1,v2; return v0
        let ssa = dex_decompiler::DexPasses::run_merged(&c).unwrap();
        let m = signature(true, vec![JavaType::Int, JavaType::Int]);
        let b = parameter_bindings(ssa.ir(), &m, 3, 2).unwrap();
        decomp_structure::structure(ssa.ir().cfg()).unwrap();
        let SourceResult::Complete { source, .. } =
            backend_java::emit_method(ssa.ir(), &m, &b, &resolver)
        else {
            panic!("arithmetic must emit")
        };
        assert!(source.contains("public static int calculate(int p0, int p1)"));
        assert!(source.contains("(p0 + p1)"));
        assert!(source.contains("return (p0 + p1);"));
    }
    #[test]
    fn resolver_preserves_isolated_surrogates() {
        let mut bytes = include_bytes!("../../../tests/corpus/dex/clean/classes.dex").to_vec();
        let original = DexFile::from_bytes(&bytes).unwrap();
        let (index, string) = original
            .strings
            .iter()
            .enumerate()
            .find(|(_, s)| (5..128).contains(&s.span.size))
            .unwrap();
        let offset = string.span.offset as usize;
        let size = string.span.size as usize;
        // Valid MUTF-8 with one unpaired high surrogate; scalar view loses it.
        bytes[offset..offset + size].fill(b'x');
        bytes[offset..offset + 4].copy_from_slice(&[(size - 4) as u8, 0xed, 0xa0, 0x80]);
        bytes[offset + size - 1] = 0;
        let dex = DexFile::from_bytes(&bytes).unwrap();
        let resolver = DexJavaResolver { dex: &dex };
        assert!(dex.strings[index].value.starts_with('\u{fffd}'));
        let mut expected = vec![0xd800];
        expected.extend(std::iter::repeat_n(u16::from(b'x'), size - 5));
        assert_eq!(
            resolver.string(StringRef::new(index as u32)),
            Some(expected)
        );
    }
    #[test]
    fn resolver_matches_real_descriptor_pools() {
        let dex = DexFile::from_bytes(include_bytes!(
            "../../../tests/corpus/dex/clean/classes.dex"
        ))
        .unwrap();
        let resolver = DexJavaResolver { dex: &dex };
        let mut methods = 0;
        for c in &dex.classes {
            if let Some(data) = &c.class_data {
                for encoded in data.direct_methods.iter().chain(&data.virtual_methods) {
                    let m = resolver.method(MethodRef::new(encoded.method_idx)).unwrap();
                    let raw = dex.method_id(encoded.method_idx).unwrap();
                    assert_eq!(m.name, dex.string(raw.name_idx).unwrap());
                    assert_eq!(m.is_static, encoded.access_flags & 8 != 0);
                    assert_eq!(
                        Some(m.owner),
                        resolver.class(ClassRef::new(u32::from(raw.class_idx)))
                    );
                    let p = dex.proto_id(u32::from(raw.proto_idx)).unwrap();
                    assert_eq!(
                        m.parameters.len(),
                        p.parameters.as_ref().map_or(0, |p| p.types.len())
                    );
                    methods += 1;
                }
                for field in data.static_fields.iter().chain(&data.instance_fields) {
                    let f = resolver.field(FieldRef::new(field.field_idx)).unwrap();
                    assert_eq!(f.is_static, field.access_flags & 8 != 0);
                    assert_eq!(
                        f.name,
                        dex.string(dex.field_id(field.field_idx).unwrap().name_idx)
                            .unwrap()
                    );
                }
            }
        }
        assert!(methods > 0);
        assert!(resolver.method(MethodRef::new(u32::MAX)).is_none());
        for (i, s) in dex.strings.iter().enumerate() {
            assert_eq!(
                resolver.string(StringRef::new(i as u32)).unwrap(),
                s.value.encode_utf16().collect::<Vec<_>>()
            );
        }
    }
}
