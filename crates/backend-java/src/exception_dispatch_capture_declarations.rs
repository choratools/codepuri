//! Original handler-producer declarations, validated before Phi typing.
use super::*;

pub(super) struct Captures {
    pub types: Vec<Vec<JavaType>>,
    pub captures: HashMap<BlockId, ValueId>,
    pub blocks: HashSet<BlockId>,
    pub contextual: HashMap<ValueId, JavaType>,
}

pub(super) fn collect(
    ir: &MethodIr<SsaForm>,
    metadata: &[&(impl RegionMetadata + ?Sized)],
    resolver: &impl JavaResolver,
) -> Result<Captures, Error> {
    let scopes = metadata
        .iter()
        .map(|m| m.region_scope().clone())
        .collect::<Vec<_>>();
    decomp_structure::exception_dispatch::plan_scopes(ir.cfg(), &scopes).map_err(Error::Scope)?;
    let mut types = Vec::new();
    let throwable = JavaType::Class("java.lang.Throwable".into());
    for md in metadata {
        let mut table = Vec::new();
        for (index, catch) in md.region_scope().catches.iter().enumerate() {
            let ty = md
                .catch_type(&catch.key)
                .ok_or(Error::Unsupported("unresolved dispatch catch type"))?;
            if !matches!(ty, JavaType::Class(_))
                || md.hierarchy().assignable(&ty, &throwable) != Compatibility::Assignable
            {
                return Err(Error::Unsupported("dispatch catch lacks Throwable proof"));
            }
            if md.is_catch_all(&catch.key)
                && (index + 1 != md.region_scope().catches.len() || ty != throwable)
            {
                return Err(Error::Unsupported("catch-all not final Throwable dispatch"));
            }
            table.push(ty);
        }
        types.push(table);
    }
    let mut captures = HashMap::new();
    let mut capture_blocks = HashSet::new();
    let mut contextual = HashMap::new();
    for (scope, md) in metadata.iter().enumerate() {
        for (index, catch) in md.region_scope().catches.iter().enumerate() {
            let mut block = catch.entry;
            let mut seen = HashSet::new();
            while seen.insert(block) {
                let node = ir.cfg().block(block).map_err(invalid)?;
                let insn = ir.instruction(*node.payload()).map_err(invalid)?;
                if matches!(insn, ManagedInsn::Phi { .. }) {
                    let [edge] = ir.cfg().block(block).map_err(invalid)?.outgoing() else {
                        return Err(Error::Unsupported("handler Phi has ambiguous successor"));
                    };
                    block = ir.cfg().edge(*edge).map_err(invalid)?.target();
                    continue;
                }
                if let ManagedInsn::Opaque {
                    family_tag: 0x8002,
                    reads,
                    writes,
                    effects,
                } = insn
                {
                    if !reads.is_empty()
                        || writes.len() != 1
                        || !matches!(*effects, EffectFlags::NONE | EffectFlags::MAY_THROW)
                    {
                        return Err(Error::Unsupported("invalid original caught-object binding"));
                    }
                    // A handler's move-exception can only be reached through
                    // its entry Phi cluster; normal entry would use stale data.
                    if ir
                        .cfg()
                        .block(catch.entry)
                        .map_err(invalid)?
                        .incoming()
                        .iter()
                        .any(|&e| {
                            ir.cfg()
                                .edge(e)
                                .is_ok_and(|e| !matches!(e.kind(), EdgeKind::Exception(_)))
                        })
                    {
                        return Err(Error::Unsupported("caught-object handler has normal entry"));
                    }
                    let value = writes[0];
                    let inferred = match ir.value(value).map_err(invalid)?.ty() {
                        managed_ir::types::ManagedType::Unknown => None,
                        ty => Some(
                            crate::managed_type(ty, resolver)
                                .map_err(|e| Error::Invalid(format!("{e:?}")))?,
                        ),
                    };
                    let expected = capture_types::domain(
                        &types[scope][index],
                        contextual.get(&value),
                        inferred.as_ref(),
                        md.hierarchy(),
                    )?;
                    contextual.insert(value, expected);
                    captures.insert(catch.entry, value);
                    capture_blocks.insert(block);
                }
                break;
            }
        }
    }
    Ok(Captures {
        types,
        captures,
        blocks: capture_blocks,
        contextual,
    })
}
