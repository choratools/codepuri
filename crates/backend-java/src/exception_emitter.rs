//! Transactional adapter to the ordinary typed instruction emitter.
use super::*;
use decomp_core::ids::BlockId;

pub(super) struct ExceptionEmitter<'e, 'a, R> {
    pub emitter: &'e mut Emitter<'a, R>,
    pub method: &'a JavaMethod,
    pub caught: Vec<JavaType>,
    pub protected: bool,
}
impl<R: JavaResolver> exceptions::TypedEmitter for ExceptionEmitter<'_, '_, R> {
    type Error = Unsupported;
    fn hoist(&mut self, locals: &[ValueId]) -> Result<(), Unsupported> {
        for &v in locals {
            let ty = self.emitter.ty(v)?;
            let mut name = format!("v{}", self.emitter.next);
            self.emitter.next += 1;
            while self.emitter.reserved.contains(&name) {
                name = format!("v{}", self.emitter.next);
                self.emitter.next += 1;
            }
            let default = match ty {
                JavaType::Boolean => "false",
                JavaType::Long => "0L",
                JavaType::Float => "0.0f",
                JavaType::Double => "0.0d",
                JavaType::Class(_) | JavaType::Array(_) => "null",
                JavaType::Void => return Err(Unsupported::Type("void exception local".into())),
                _ => "0",
            };
            self.emitter
                .lines
                .push(format!("{} {name} = {default};", ty.source()?));
            self.emitter.reserved.insert(name.clone());
            self.emitter.values.insert(v, (name, ty));
            self.emitter.hoisted.insert(v);
        }
        Ok(())
    }
    fn instruction(&mut self, id: InsnId) -> Result<(), Unsupported> {
        let insn = self
            .emitter
            .ir
            .instruction(id)
            .map_err(|e| Unsupported::Instruction(e.to_string()))?;
        let mut signature = self.method.clone();
        // These allowances exist only while emitting the protected body. A
        // handler's own checked calls are checked against declaration throws.
        if self.protected
            && let ManagedInsn::Invoke { method, .. } = insn
        {
            let target = self
                .emitter
                .resolver
                .method(*method)
                .ok_or_else(|| Unsupported::Metadata("unresolved protected invoke".into()))?;
            for name in target.throws {
                if self.caught.iter().any(|catch| {
                    self.emitter
                        .resolver
                        .assignable(&JavaType::Class(name.clone()), catch)
                }) {
                    signature.throws.push(name);
                }
            }
        }
        // Folding a proved argument expression at NEW keeps that call inside
        // the original protected scope. A current-instruction-only allowance
        // misses its checked exceptions. Require exact ordered original handler
        // destinations, not merely a coincidentally equal caught type.
        if self.protected {
            let allocation = match insn {
                ManagedInsn::NewInstance { .. } => Some(id),
                ManagedInsn::Invoke {
                    kind: InvokeKind::Special,
                    result: None,
                    args,
                    ..
                } => args
                    .first()
                    .and_then(|receiver| self.emitter.allocation_root(*receiver)),
                _ => None,
            };
            let handlers = |instruction: InsnId| -> Option<Vec<(BlockId, String)>> {
                let block = self.emitter.ir.cfg().block_ids().find_map(|b| {
                    let block = self.emitter.ir.cfg().block(b).ok()?;
                    (*block.payload() == instruction).then_some(block)
                })?;
                let mut ordered = Vec::new();
                for edge in block.outgoing() {
                    let edge = self.emitter.ir.cfg().edge(*edge).ok()?;
                    if let EdgeKind::Exception(key) = edge.kind() {
                        ordered.push((edge.target(), key.clone()));
                    }
                }
                Some(ordered)
            };
            if let Some(allocation) = allocation
                && let Ok(plan) = self.emitter.object_plan(allocation)
                && (id == plan.allocation || id == plan.initializer)
            {
                let original = handlers(allocation);
                let mut extra = Vec::new();
                for argument in &plan.argument_gap {
                    let Ok(ManagedInsn::Invoke { method, .. }) =
                        self.emitter.ir.instruction(*argument)
                    else {
                        continue;
                    };
                    let target = self.emitter.resolver.method(*method).ok_or_else(|| {
                        Unsupported::Metadata(
                            "folded checked invoke declaration unavailable".into(),
                        )
                    })?;
                    let Err(uncovered) = checked_exceptions::check(
                        self.emitter.resolver,
                        &target.throws,
                        &self.method.throws,
                    ) else {
                        continue;
                    };
                    // Authorize the complete folded trace atomically. Otherwise
                    // one checked producer (or the initializer's own allowance)
                    // could lend the same exception type to a foreign-scope call.
                    if original.as_ref().is_none_or(Vec::is_empty)
                        || handlers(plan.initializer) != original
                        || handlers(*argument) != original
                        || uncovered.types.iter().any(|name| {
                            let ty = JavaType::Class(name.clone());
                            !self.caught.iter().any(|catch| {
                                ty == *catch || self.emitter.resolver.assignable(&ty, catch)
                            })
                        })
                    {
                        return Err(Unsupported::Metadata(
                            "folded checked invoke original handlers/coverage".into(),
                        ));
                    }
                    extra.extend(uncovered.types);
                }
                signature.throws.extend(extra);
            }
        }
        self.emitter.current = Some(id);
        self.emitter.instruction(insn, &signature)?;
        Ok(())
    }
    fn copies(&mut self, copies: &[branching::Copy]) -> Result<(), Unsupported> {
        // The renderer does not inspect edge identity; plan already validates it.
        let edge = self
            .emitter
            .ir
            .cfg()
            .edge_ids()
            .next()
            .ok_or_else(|| Unsupported::ControlFlow("missing exception copy edge".into()))?;
        render_steps(
            self.emitter,
            &[branching::Step::Copies {
                edge,
                copies: copies.to_vec(),
            }],
            self.method,
        )
    }
    fn begin_try(&mut self) -> Result<(), Unsupported> {
        self.protected = true;
        self.emitter.lines.push("try {".into());
        Ok(())
    }
    fn begin_catch(&mut self, ty: &JavaType, exception: ValueId) -> Result<(), Unsupported> {
        self.protected = false;
        let (local, expected) = self.emitter.get(exception)?;
        self.emitter.compatible(ty, &expected)?;
        let mut name = format!("caught{}", self.emitter.next);
        self.emitter.next += 1;
        while self.emitter.reserved.contains(&name) {
            name = format!("caught{}", self.emitter.next);
            self.emitter.next += 1;
        }
        self.emitter.reserved.insert(name.clone());
        self.emitter
            .lines
            .push(format!("}} catch ({} {name}) {{", ty.source()?));
        self.emitter.lines.push(format!("{local} = {name};"));
        Ok(())
    }
    fn end_catches(&mut self) -> Result<(), Unsupported> {
        self.protected = false;
        self.emitter.lines.push("}".into());
        Ok(())
    }
}
