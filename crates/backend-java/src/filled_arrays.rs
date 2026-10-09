//! Filled allocations. Hook: `NewFilledArray { result, target, elements } =>
//! self.filled_array(*result, *target, elements)?`.
use super::*;

impl<R: JavaResolver> Emitter<'_, R> {
    /// Call before the ordinary Const arm's `ty()` lookup. Null has no nominal
    /// Java type; use an exact filled-array consumer's component for its local.
    pub(super) fn filled_array_null_constant(
        &mut self,
        result: ValueId,
        signature: &JavaMethod,
    ) -> Result<bool, Unsupported> {
        let value = self
            .ir
            .value(result)
            .map_err(|e| Unsupported::Type(e.to_string()))?;
        if !matches!(value.ty(), ManagedType::Null | ManagedType::Zero)
            || !matches!(
                self.ir.instruction(value.def()),
                Ok(ManagedInsn::Const {
                    value: Constant::Zero | Constant::Null,
                    ..
                })
            )
        {
            return Ok(false);
        }
        let contexts = contextual_locals::consumer_constraints_indexed(
            self.ir,
            self.object_contextual_index(),
            &[result],
            &signature.return_type,
            self.resolver,
            &self.declarations,
        )
        .map_err(|e| Unsupported::Type(format!("contextual constant constraints: {e:?}")))?;
        // A literal DEX zero has a known category-one bit pattern even when
        // no descriptor consumer exists (dead constants, arithmetic, moves).
        // Keep it as int rather than inventing reference or wide evidence.
        let fallback = JavaType::Int;
        let first = match contexts.first() {
            Some(first) => first,
            None if matches!(value.ty(), ManagedType::Zero) => &fallback,
            None => return Ok(false),
        };
        for context in &contexts {
            contextual_locals::expression(
                self.ir,
                result,
                context,
                &type_compatibility::Hierarchy::default(),
                self.resolver,
            )
            .map_err(|e| Unsupported::Type(format!("contextual constant consumer: {e:?}")))?;
        }
        // A category-one raw zero may have several independent operand roles.
        // Its pure local remains int; actual use-site evidence supplies literals.
        // This never changes a phi declaration or substitutes a selected phi.
        let declared_int_zero = self.hoisted.contains(&result)
            && self.declarations.get(&result) == Some(&JavaType::Int)
            && matches!(
                self.ir.instruction(value.def()),
                Ok(ManagedInsn::Const {
                    value: Constant::Zero,
                    ..
                })
            );
        let expected = if declared_int_zero
            || (self.hoisted.contains(&result) || contexts.iter().any(|ty| ty != first))
                && matches!(value.ty(), ManagedType::Zero)
        {
            &JavaType::Int
        } else {
            first
        };
        // The inferred annotation can be Null even for a raw DEX zero shared
        // by numeric and reference consumers. Only definition-proven null is
        // reference-only; expression() above separately rejects wide raw zero.
        if matches!(
            self.cached_value_evidence(result).as_ref(),
            Ok(type_compatibility::ValueType::Null)
        ) && !expected.reference()
        {
            return Err(Unsupported::Type(
                "null primitive declaration context".into(),
            ));
        }
        let expression = constant(
            if expected.reference() {
                Constant::Null
            } else {
                Constant::Zero
            },
            expected,
            self.resolver,
        )?;
        self.define_local(result, expression, expected.clone(), true)?;
        Ok(true)
    }

    /// A fill-array-data payload encodes unsigned one/two-byte words. Its
    /// inferred signed component interprets those bits; it does not truncate
    /// an arbitrary register integer. Require the exact definition, allocation
    /// descriptor, and every consumer before normalizing that representation.
    pub(super) fn array_payload_constant(
        &mut self,
        result: ValueId,
        payload: Constant,
        signature: &JavaMethod,
    ) -> Result<bool, Unsupported> {
        let Constant::Int(bits) = payload else {
            return Ok(false);
        };
        // The ordinary signed literals take the existing fast path.
        if !(128..=65535).contains(&bits) {
            return Ok(false);
        }
        let ty = self.ty(result)?;
        let signed = match ty {
            JavaType::Byte if (128..=255).contains(&bits) => i32::from(bits as u8 as i8),
            JavaType::Short if (32768..=65535).contains(&bits) => i32::from(bits as u16 as i16),
            _ => return Ok(false),
        };
        let fail = || Unsupported::Type("unproved array payload constant bits".into());
        let record = self.ir.value(result).map_err(|_| fail())?;
        if !matches!(self.ir.instruction(record.def()), Ok(ManagedInsn::Const { result: defined, value }) if *defined == result && *value == payload)
        {
            return Err(fail());
        }
        let mut payload_consumer = false;
        for id in self.object_raw_uses(result) {
            let insn = self.ir.instruction(id).map_err(|_| fail())?;
            if let ManagedInsn::FillArray { array, values } = insn
                && values.contains(&result)
            {
                // SSA may place pure payload constants in its entry prelude.
                // Their bit interpretation requires exact allocation/type evidence,
                // not premature execution of that allocation. FillArray itself
                // still requires the materialized array at its original use.
                let array_ty = self.ty(*array)?;
                if array_ty != JavaType::Array(Box::new(ty.clone())) {
                    return Err(fail());
                }
                let mut current = *array;
                let mut visited = HashSet::new();
                let target = loop {
                    if !visited.insert(current) {
                        return Err(fail());
                    }
                    let record = self.ir.value(current).map_err(|_| fail())?;
                    match self.ir.instruction(record.def()).map_err(|_| fail())? {
                        ManagedInsn::Move { result, value } if *result == current => {
                            current = *value
                        }
                        ManagedInsn::NewArray {
                            result, element, ..
                        } if *result == current => break *element,
                        _ => return Err(fail()),
                    }
                };
                let descriptor = self.resolver.instruction_type(target).ok_or_else(|| {
                    Unsupported::Metadata("unresolved array payload allocation descriptor".into())
                })?;
                if descriptor != ty && descriptor != array_ty {
                    return Err(fail());
                }
                payload_consumer = true;
            }
        }
        if !payload_consumer {
            return Ok(false);
        }
        let contexts = contextual_locals::consumer_constraints_indexed(
            self.ir,
            self.object_contextual_index(),
            &[result],
            &signature.return_type,
            self.resolver,
            &self.declarations,
        )
        .map_err(|_| fail())?;
        if contexts.is_empty() || contexts.iter().any(|context| *context != ty) {
            return Err(fail());
        }
        self.define(result, signed.to_string(), ty)?;
        Ok(true)
    }

    pub(super) fn filled_array(
        &mut self,
        result: Option<ValueId>,
        target: TypeRef,
        elements: &[ValueId],
    ) -> Result<(), Unsupported> {
        let ty = self
            .resolver
            .instruction_type(target)
            .ok_or_else(|| Unsupported::Metadata("unresolved filled-array descriptor".into()))?;
        let JavaType::Array(component) = &ty else {
            return Err(Unsupported::Type(
                "filled allocation requires array descriptor".into(),
            ));
        };
        if matches!(
            component.as_ref(),
            JavaType::Void | JavaType::Long | JavaType::Double
        ) {
            return Err(Unsupported::Type("illegal filled-array component".into()));
        }
        if let Some(result) = result
            && self.ty(result)? != ty
        {
            return Err(Unsupported::Type("filled-array result descriptor".into()));
        }
        let values = elements
            .iter()
            .map(|value| {
                self.get(*value)?;
                match contextual_locals::expression(
                    self.ir,
                    *value,
                    component,
                    &type_compatibility::Hierarchy::default(),
                    self.resolver,
                ) {
                    Ok(type_compatibility::Coercion::NullLiteral) => return Ok("null".into()),
                    Ok(type_compatibility::Coercion::ZeroLiteral(literal)) => {
                        return Ok(literal.into());
                    }
                    _ => {}
                }
                if matches!(
                    contextual_locals::evidence(self.ir, *value, self.resolver),
                    Ok(type_compatibility::ValueType::Null
                        | type_compatibility::ValueType::RawZero)
                ) {
                    return Err(Unsupported::Type(
                        "incompatible filled-array literal context".into(),
                    ));
                }
                let (name, source) = self.get(*value)?;
                if source.reference() && component.reference() {
                    self.compatible(&source, component)?;
                    Ok(name)
                } else {
                    filled_element(name, &source, component)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let expression = format!("new {} {{{}}}", ty.source()?, values.join(", "));
        match result {
            Some(result) => self.define(result, expression, ty),
            None => {
                // Array creation is not a Java statement expression. A local also
                // preserves allocation/initialization exceptions at this point.
                let mut name = format!("discardedArray{}", self.next);
                while self.reserved.contains(&name) {
                    self.next += 1;
                    name = format!("discardedArray{}", self.next);
                }
                self.next += 1;
                self.reserved.insert(name.clone());
                self.lines
                    .push(format!("{} {name} = {expression};", ty.source()?));
                Ok(())
            }
        }
    }
}

fn filled_element(
    name: String,
    source: &JavaType,
    component: &JavaType,
) -> Result<String, Unsupported> {
    if *source == JavaType::Int {
        return Ok(match component {
            JavaType::Boolean => format!("({name} != 0)"),
            JavaType::Byte | JavaType::Short | JavaType::Char => {
                format!("({}) {name}", component.source()?)
            }
            JavaType::Float => format!("java.lang.Float.intBitsToFloat({name})"),
            _ => {
                compatible(source, component)?;
                name
            }
        });
    }
    compatible(source, component)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    struct Resolver(JavaType);
    impl JavaResolver for Resolver {
        fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
            Some(self.0.clone())
        }
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            Some(JavaMethod {
                owner: "Fixture".into(),
                name: "consume".into(),
                is_static: true,
                return_type: JavaType::Void,
                parameters: vec![self.0.clone()],
                throws: vec![],
            })
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            Some(JavaField {
                owner: "Fixture".into(),
                name: "stored".into(),
                ty: self.0.clone(),
                is_static: true,
            })
        }
        fn class(&self, _: ClassRef) -> Option<String> {
            Some("java.lang.Object".into())
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    #[test]
    fn discarded_source_keeps_allocation_between_effects() {
        use managed_ir::{
            method::{InsnArena, ValueArena},
            value::Value,
        };
        let mut insns = InsnArena::default();
        let mut values = ValueArena::default();
        let constant = insns.insert(ManagedInsn::Return { value: None });
        let value = values.insert(Value::new(constant, ManagedType::Int));
        let allocation = insns.insert(ManagedInsn::NewFilledArray {
            result: None,
            target: TypeRef::new(0),
            elements: vec![value, value],
        });
        let ir =
            MethodIr::<SsaForm>::from_parts(decomp_core::cfg::Cfg::new(allocation), insns, values);
        let resolver = Resolver(JavaType::Array(Box::new(JavaType::Float)));
        let mut emitter = Emitter {
            allocation_readable: None,
            object_planning: Default::default(),
            ir: &ir,
            resolver: &resolver,
            values: HashMap::from([(value, ("bits".into(), JavaType::Int))]),
            lines: vec!["int bits = tick();".into()],
            next: 0,
            parameters: HashSet::new(),
            reserved: HashSet::from(["discardedArray0".into()]),
            current: Some(allocation),
            hoisted: HashSet::new(),
            declarations: HashMap::new(),
        };
        emitter
            .filled_array(None, TypeRef::new(0), &[value, value])
            .unwrap();
        emitter.lines.push("after();".into());
        assert_eq!(
            emitter.lines,
            [
                "int bits = tick();",
                "float[] discardedArray1 = new float[] {java.lang.Float.intBitsToFloat(bits), java.lang.Float.intBitsToFloat(bits)};",
                "after();",
            ]
        );
        assert!(emitter.reserved.contains("discardedArray1"));
        let before = emitter.lines.clone();
        let invalid = Resolver(JavaType::Array(Box::new(JavaType::Long)));
        emitter.resolver = &invalid;
        assert!(emitter.filled_array(None, TypeRef::new(0), &[]).is_err());
        assert_eq!(emitter.lines, before);
    }
    #[test]
    fn contextual_family_checks_arena_escapes_and_preserves_selection() {
        use managed_ir::{
            method::{InsnArena, ValueArena},
            value::Value,
        };
        let mut insns = InsnArena::with_key();
        let mut values = ValueArena::with_key();
        let mut define = |ty, f: &dyn Fn(ValueId) -> ManagedInsn| {
            let mut value = None;
            insns.insert_with_key(|id| {
                let v = values.insert(Value::new(id, ty));
                value = Some(v);
                f(v)
            });
            value.unwrap()
        };
        let zero = define(ManagedType::Zero, &|result| ManagedInsn::Const {
            result,
            value: Constant::Zero,
        });
        let seven = define(ManagedType::Int, &|result| ManagedInsn::Const {
            result,
            value: Constant::Int(7),
        });
        let phi = define(ManagedType::Unknown, &|result| ManagedInsn::Phi {
            result,
            inputs: vec![(Default::default(), zero), (Default::default(), seven)],
        });
        let copy = define(ManagedType::Unknown, &|result| ManagedInsn::Move {
            result,
            value: phi,
        });
        let array = define(
            ManagedType::Array {
                element: Box::new(ManagedType::Int),
                nullable: true,
            },
            &|result| ManagedInsn::Opaque {
                family_tag: 0x8003,
                reads: vec![],
                writes: vec![result],
                effects: managed_ir::effects::EffectFlags::NONE,
            },
        );
        let head = values[phi].def();
        insns.insert(ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(0),
            value: copy,
        });
        insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(0),
            args: vec![copy],
        });
        insns.insert(ManagedInsn::ArrayPut {
            array,
            index: zero,
            value: copy,
        });
        // This return is deliberately not the CFG payload: collection must
        // inspect the instruction arena, not just the current emission block.
        insns.insert(ManagedInsn::Return { value: Some(copy) });
        let ir = MethodIr::<SsaForm>::from_parts(decomp_core::cfg::Cfg::new(head), insns, values);
        let resolver = Resolver(JavaType::Int);
        let hierarchy = type_compatibility::Hierarchy::default();
        assert_eq!(
            contextual_locals::consumer_constraints(&ir, &[phi], &JavaType::Int, &resolver)
                .unwrap(),
            vec![JavaType::Int; 4]
        );
        let plan = contextual_locals::checked_phi_plan(
            &ir,
            phi,
            &JavaType::Int,
            &JavaType::Int,
            &hierarchy,
            &resolver,
        )
        .unwrap();
        assert!(plan.locals.contains(&copy));
        assert_eq!(
            plan.inputs[1],
            (seven, type_compatibility::Coercion::Identity)
        );
        assert!(
            contextual_locals::checked_phi_plan(
                &ir,
                phi,
                &JavaType::Int,
                &JavaType::Class("java.lang.String".into()),
                &hierarchy,
                &resolver
            )
            .is_err()
        );
        assert!(
            contextual_locals::checked_phi_plan(
                &ir,
                phi,
                &JavaType::Float,
                &JavaType::Float,
                &hierarchy,
                &resolver
            )
            .is_err()
        );
        let mut insns = ir.instructions().clone();
        insns.insert(ManagedInsn::Opaque {
            family_tag: 0xffff,
            reads: vec![copy],
            writes: vec![],
            effects: managed_ir::effects::EffectFlags::NONE,
        });
        let opaque = MethodIr::<SsaForm>::from_parts(
            decomp_core::cfg::Cfg::new(head),
            insns,
            ir.values().clone(),
        );
        assert!(
            contextual_locals::checked_phi_plan(
                &opaque,
                phi,
                &JavaType::Int,
                &JavaType::Int,
                &hierarchy,
                &resolver
            )
            .is_err()
        );
    }
    #[test]
    fn category_one_source_forms() {
        assert_eq!(
            filled_element("v".into(), &JavaType::Int, &JavaType::Boolean).unwrap(),
            "(v != 0)"
        );
        assert_eq!(
            filled_element("v".into(), &JavaType::Int, &JavaType::Byte).unwrap(),
            "(byte) v"
        );
        assert_eq!(
            filled_element("bits".into(), &JavaType::Int, &JavaType::Float).unwrap(),
            "java.lang.Float.intBitsToFloat(bits)"
        );
        let reference = JavaType::Class("java.lang.Object".into());
        assert_eq!(
            filled_element("null".into(), &reference, &reference).unwrap(),
            "null"
        );
        assert!(filled_element("v".into(), &JavaType::Long, &JavaType::Int).is_err());
    }
}
