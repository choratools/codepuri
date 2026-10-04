//! Exact DEX-to-SSA scope bridge; absent offsets never imply a source span.
use backend_java::{
    JavaResolver, JavaType,
    exceptions::{
        ExceptionMetadata,
        scopes::{Catch, Scope},
    },
};
use decomp_core::{cfg::EdgeKind, ids::BlockId};
use dex_decompiler::ssa::SsaMethod;
use dex_format::code::CodeItem;
use managed_ir::{insn::ManagedInsn, refs::ClassRef};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug)]
pub(crate) struct Metadata {
    scopes: Vec<Scope>,
    types: BTreeMap<String, JavaType>,
}
impl ExceptionMetadata for Metadata {
    fn scopes(&self) -> &[Scope] {
        &self.scopes
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        self.types.get(key).cloned()
    }
}
impl Metadata {
    /// Declaration-level dependency checks use these names; the backend still
    /// validates allowances separately at each instruction's actual catch scope.
    #[allow(dead_code)]
    pub(crate) fn catch_names(&self) -> impl Iterator<Item = &str> {
        self.types.values().filter_map(|t| {
            if let JavaType::Class(n) = t {
                Some(n.as_str())
            } else {
                None
            }
        })
    }
}

pub(crate) fn from_dex(
    ssa: &SsaMethod,
    code: &CodeItem,
    resolver: &impl JavaResolver,
) -> Result<Metadata, String> {
    let reject = |reason: &str| format!("exception scope: {reason}");
    if code.tries.len() != 1 {
        return Err(reject("exactly one DEX try range required"));
    }
    let range = &code.tries[0];
    let end = range
        .start_addr
        .checked_add(u32::from(range.insn_count))
        .ok_or_else(|| reject("source range overflow"))?;
    let handler = code
        .handlers
        .iter()
        .find(|h| h.relative_offset == u32::from(range.handler_off))
        .ok_or_else(|| reject("missing encoded catch table"))?;
    if handler.catch_all_addr.is_some() {
        return Err(reject("catch-all/finally scope unproven"));
    }
    let ir = ssa.ir();
    let cfg = ir.cfg();
    let instruction = |b| -> Result<_, String> {
        let id = *cfg.block(b).map_err(|e| e.to_string())?.payload();
        Ok((id, ir.instruction(id).map_err(|e| e.to_string())?))
    };
    let successor = |b| -> Result<Option<BlockId>, String> {
        let normal: Vec<_> = cfg
            .block(b)
            .map_err(|e| e.to_string())?
            .outgoing()
            .iter()
            .map(|e| cfg.edge(*e).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|e| !matches!(e.kind(), EdgeKind::Exception(_)))
            .collect();
        match normal.as_slice() {
            [] => Ok(None),
            [edge] if edge.kind() == &EdgeKind::Unconditional => Ok(Some(edge.target())),
            _ => Err(reject("protected normal path is nonlinear")),
        }
    };
    let mut protected = Vec::new();
    let mut seen = HashSet::new();
    let mut current = cfg.entry();
    let mut previous = None;
    let continuation = loop {
        if !seen.insert(current) {
            return Err(reject("cyclic normal path"));
        }
        let (id, insn) = instruction(current)?;
        let inside = match ssa.offset(id) {
            Some(offset) => range.start_addr <= offset && offset < end,
            None if protected.is_empty() => false,
            None => {
                // A normal-success commit is adjacent, single-entry, and reads
                // the result of the immediately preceding protected operation.
                // No arbitrary offsetless instruction inherits a surrounding span.
                let commit = if let (ManagedInsn::Move { value, .. }, Some(prev)) = (insn, previous)
                {
                    let (prev_id, _) = instruction(prev)?;
                    ir.value(*value).map_err(|e| e.to_string())?.def() == prev_id
                        && cfg
                            .block(current)
                            .map_err(|e| e.to_string())?
                            .incoming()
                            .len()
                            == 1
                        && ssa
                            .offset(prev_id)
                            .is_some_and(|o| range.start_addr <= o && o < end)
                } else {
                    false
                };
                if !commit {
                    // A join Phi is outside the range only when its next real
                    // instruction is outside and every intermediate node is Phi.
                    let mut anchor = current;
                    let mut visited = HashSet::new();
                    loop {
                        if !visited.insert(anchor) {
                            return Err(reject("cyclic synthetic provenance"));
                        }
                        let (aid, ai) = instruction(anchor)?;
                        if let Some(o) = ssa.offset(aid) {
                            if range.start_addr <= o && o < end {
                                return Err(reject("unproven synthetic protected provenance"));
                            }
                            break;
                        }
                        if !matches!(ai, ManagedInsn::Phi { .. }) {
                            return Err(reject("unproven synthetic protected provenance"));
                        }
                        anchor = successor(anchor)?
                            .ok_or_else(|| reject("missing synthetic source anchor"))?;
                    }
                }
                commit
            }
        };
        if inside {
            protected.push(current);
        } else if !protected.is_empty() {
            break current;
        }
        previous = Some(current);
        current = successor(current)?.ok_or_else(|| reject("no exclusive normal continuation"))?;
    };
    if protected.is_empty() {
        return Err(reject("source range has no placed protected instruction"));
    }
    // Every placed real instruction in the source range must be in this chain.
    for b in cfg.block_ids() {
        let (id, _) = instruction(b)?;
        if ssa
            .offset(id)
            .is_some_and(|o| range.start_addr <= o && o < end)
            && !protected.contains(&b)
        {
            return Err(reject("source range is not one exact linear chain"));
        }
    }
    let mut catches = Vec::new();
    let mut types = BTreeMap::new();
    for catch in &handler.handlers {
        let key = catch.type_idx.to_string();
        let targets: HashSet<_> = cfg
            .edge_ids()
            .filter_map(|id| {
                let e = cfg.edge(id).ok()?;
                (e.kind() == &EdgeKind::Exception(key.clone())).then_some(e.target())
            })
            .collect();
        if targets.len() != 1 {
            return Err(reject("catch target identity is ambiguous or absent"));
        }
        let entry = *targets
            .iter()
            .next()
            .ok_or_else(|| reject("missing handler entry"))?;
        let mut anchor = entry;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(anchor) {
                return Err(reject("cyclic handler provenance"));
            }
            let (id, insn) = instruction(anchor)?;
            if let Some(offset) = ssa.offset(id) {
                if offset != catch.address {
                    return Err(reject("encoded handler address disagrees with SSA"));
                }
                break;
            }
            if !matches!(
                insn,
                ManagedInsn::Phi { .. }
                    | ManagedInsn::Opaque {
                        family_tag: 0x8002,
                        ..
                    }
            ) {
                return Err(reject("unproven synthetic handler provenance"));
            }
            anchor = successor(anchor)?.ok_or_else(|| reject("missing handler source anchor"))?;
        }
        let name = resolver
            .class(ClassRef::new(catch.type_idx))
            .ok_or_else(|| reject("caught type metadata unavailable"))?;
        if types.insert(key.clone(), JavaType::Class(name)).is_some() {
            return Err(reject("duplicate caught type"));
        }
        catches.push(Catch { key, entry });
    }
    let scopes = vec![Scope {
        protected,
        catches,
        continuation,
        has_unproven_finally: false,
    }];
    backend_java::exceptions::scopes::structure(cfg, &scopes)
        .map_err(|e| reject(&e.to_string()))?;
    Ok(Metadata { scopes, types })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use backend_java::{DeclarationBindings, JavaMethod, SourceResult};
    use dex_format::{
        RawSpan,
        code::{EncodedCatchHandler, TryItem, TypeAddrPair},
    };
    use managed_ir::{
        refs::{FieldRef, MethodRef, StringRef},
        types::ManagedType,
    };
    struct Types;
    impl dex_decompiler::TypeContext for Types {
        fn enclosing_return(&self) -> Option<ManagedType> {
            Some(ManagedType::Int)
        }
        fn method_return(&self, _: u32) -> Option<ManagedType> {
            Some(ManagedType::Int)
        }
        fn method_params(&self, _: u32, params: &mut Vec<ManagedType>) -> bool {
            params.clear();
            params.push(ManagedType::Reference(ClassRef::new(0)));
            true
        }
    }
    struct Resolver;
    impl JavaResolver for Resolver {
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            a == b
                || matches!((a,b), (JavaType::Class(a), JavaType::Class(b)) if a == "java.lang.ArithmeticException" && b == "java.lang.Throwable")
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            Some(
                if r.index() == 0 {
                    "java.lang.Throwable"
                } else {
                    "java.lang.ArithmeticException"
                }
                .into(),
            )
        }
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            Some(JavaMethod {
                owner: "DexCatch".into(),
                name: "report".into(),
                is_static: true,
                return_type: JavaType::Int,
                parameters: vec![JavaType::Class("java.lang.Throwable".into())],
                throws: vec![],
            })
        }
        fn field(&self, _: FieldRef) -> Option<backend_java::JavaField> {
            None
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    #[test]
    fn exact_dex_offsets_and_success_commit_compile_and_execute() {
        let span = RawSpan { offset: 0, size: 0 };
        let units: [u16; 11] = [
            0x0013, 10, 0x0093, 0x0200, 0x0628, 0x010d, 0x1071, 0, 1, 0x000a, 0x000f,
        ];
        let code = CodeItem {
            registers_size: 3,
            ins_size: 1,
            outs_size: 1,
            debug_info_off: 0,
            insns_size: units.len() as u32,
            insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
            tries: vec![TryItem {
                start_addr: 2,
                insn_count: 3,
                handler_off: 1,
                span,
            }],
            handlers: vec![EncodedCatchHandler {
                relative_offset: 1,
                handlers: vec![TypeAddrPair {
                    type_idx: 1,
                    address: 5,
                }],
                catch_all_addr: None,
                span,
            }],
            span,
        };
        let ssa = dex_decompiler::DexPasses::run_ssa_with(&code, &Types).unwrap();
        let metadata = from_dex(&ssa, &code, &Resolver).unwrap();
        assert_eq!(
            metadata.scopes[0].protected.len(),
            3,
            "operation, exact adjacent success commit, and source-covered goto"
        );
        let m = JavaMethod {
            owner: "DexCatch".into(),
            name: "divide".into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![JavaType::Int],
            throws: vec![],
        };
        let bindings =
            crate::java_initial_parameters::parameter_bindings(ssa.ir(), &m, 3, 1).unwrap();
        let crate::java_initial_parameters::InitialParameter::Bound(value) = bindings.parameters[0]
        else {
            panic!("missing input");
        };
        let result = backend_java::emit_method_declaration_with_exceptions(
            ssa.ir(),
            &m,
            &DeclarationBindings {
                parameters: vec![Some(value)],
                ..Default::default()
            },
            &Resolver,
            Some(&metadata),
        );
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}");
        };
        let source = format!(
            "public class DexCatch {{ {source} static int report(Throwable e) {{ if(!(e instanceof ArithmeticException)) throw new AssertionError();return -1; }} public static void main(String[] a) {{ if(divide(2)!=5 || divide(0)!=-1 || divide(-2)!=-5) throw new AssertionError();System.out.println(\"DEX scope assertions passed\"); }} }}"
        );
        let dir = std::env::temp_dir().join(format!("decomphify-dex-scope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("DexCatch.java"), &source).unwrap();
        let compiled = std::process::Command::new("javac")
            .arg("DexCatch.java")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}\n{source}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let run = std::process::Command::new("java")
            .args(["-cp", ".", "DexCatch"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "DEX scope assertions passed\n"
        );
        let mut bad = code.clone();
        bad.handlers[0].handlers[0].address = 6;
        assert!(
            from_dex(&ssa, &bad, &Resolver)
                .unwrap_err()
                .contains("encoded handler address")
        );
    }
}
