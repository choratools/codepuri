//! Exact typed managed operations; no guessed descriptors or constructors.
use super::*;

impl<R: JavaResolver> Emitter<'_, R> {
    /// Object lowering runs before ordinary managed operations so preinit
    /// aliases never become Java locals until their exact initializer completes.
    pub(super) fn object_instruction(
        &mut self,
        insn: &ManagedInsn,
        signature: &JavaMethod,
    ) -> Result<bool, Unsupported> {
        self.planned_object_instruction(insn, signature)
    }

    fn target_type(&self, target: TypeRef) -> Result<JavaType, Unsupported> {
        self.resolver
            .instruction_type(target)
            .ok_or_else(|| Unsupported::Metadata("unresolved instruction type".into()))
    }

    fn array(&self, value: ValueId) -> Result<(String, JavaType), Unsupported> {
        match self.get(value)? {
            (name, JavaType::Array(component)) => Ok((name, *component)),
            _ => Err(Unsupported::Type("array operand".into())),
        }
    }

    fn index(&self, value: ValueId) -> Result<String, Unsupported> {
        let (name, ty) = self.operand(value, &JavaType::Int)?;
        compatible(&ty, &JavaType::Int)?;
        Ok(name)
    }

    pub(super) fn managed_operation(
        &mut self,
        insn: &ManagedInsn,
        signature: &JavaMethod,
    ) -> Result<(), Unsupported> {
        match insn {
            ManagedInsn::NewArray {
                result,
                element,
                length,
            } => {
                let target = self.target_type(*element)?;
                let ty = self.ty(*result)?;
                // Frontends may supply either the full array descriptor or its component.
                if target != ty && JavaType::Array(Box::new(target)) != ty {
                    return Err(Unsupported::Type("array allocation descriptor".into()));
                }
                let JavaType::Array(component) = &ty else {
                    return Err(Unsupported::Type("allocation result is not array".into()));
                };
                let length = self.index(*length)?;
                let mut base = component.as_ref();
                let mut dimensions = String::new();
                while let JavaType::Array(inner) = base {
                    dimensions.push_str("[]");
                    base = inner;
                }
                self.define(
                    *result,
                    format!("new {}[{length}]{dimensions}", base.source()?),
                    ty,
                )?;
            }
            ManagedInsn::ArrayGet {
                result,
                array,
                index,
            } => {
                let (array, component) = self.array(*array)?;
                let index = self.index(*index)?;
                let result_ty = self.ty(*result)?;
                if component != result_ty
                    && !(matches!(component, JavaType::Byte | JavaType::Short | JavaType::Char)
                        && result_ty == JavaType::Int)
                {
                    return Err(Unsupported::Type("array load component".into()));
                }
                self.define(*result, format!("{array}[{index}]"), result_ty)?;
            }
            ManagedInsn::ArrayPut {
                array,
                index,
                value,
            } => {
                let (array, component) = self.array(*array)?;
                let index = self.index(*index)?;
                let value = self.array_value(*value, &component)?;
                self.lines.push(format!("{array}[{index}] = {value};"));
            }
            ManagedInsn::ArrayLength { result, array } => {
                let (array, _) = self.array(*array)?;
                self.define(*result, format!("{array}.length"), JavaType::Int)?;
            }
            ManagedInsn::FillArray { array, values } => {
                let (array, component) = self.array(*array)?;
                if !component.numeric() && component != JavaType::Boolean {
                    return Err(Unsupported::Type(
                        "fill requires primitive component".into(),
                    ));
                }
                let actual = self.fill_payload_values(values, &component, signature)?;
                // Validate the whole payload before any stores, including an empty payload's null check.
                self.lines.push(format!("if ({array}.length < {}) throw new java.lang.ArrayIndexOutOfBoundsException();", actual.len()));
                for (index, value) in actual.iter().enumerate() {
                    self.lines.push(format!("{array}[{index}] = {value};"));
                }
            }
            ManagedInsn::Cast {
                result,
                value,
                target,
            } => {
                let original_target = *target;
                let target = self.target_type(original_target)?;
                let (name, source) = self.get(*value)?;
                let boolean_word_target = match original_target.index() {
                    0x8000_0081 => Some(JavaType::Long),
                    0x8000_0082 => Some(JavaType::Float),
                    0x8000_0083 => Some(JavaType::Double),
                    0x8000_008d => Some(JavaType::Byte),
                    0x8000_008e => Some(JavaType::Char),
                    0x8000_008f => Some(JavaType::Short),
                    _ => None,
                };
                if source == JavaType::Boolean && boolean_word_target.as_ref() == Some(&target) {
                    let result_ty = self.ty(*result)?;
                    if result_ty != target
                        && !(result_ty == JavaType::Int
                            && matches!(target, JavaType::Byte | JavaType::Short | JavaType::Char))
                    {
                        return Err(Unsupported::Type("numeric cast result".into()));
                    }
                    // Dalvik's boolean register is an int word. Java requires a
                    // typed conditional; evaluating the existing SSA read once
                    // preserves volatile reads and prior producer effects.
                    let spelling = target.source()?;
                    self.define(
                        *result,
                        format!("({name} ? ({spelling}) 1 : ({spelling}) 0)"),
                        result_ty,
                    )?;
                } else if source.numeric() && target.numeric() {
                    let result_ty = self.ty(*result)?;
                    if result_ty != target
                        && !(result_ty == JavaType::Int
                            && matches!(target, JavaType::Byte | JavaType::Short | JavaType::Char))
                    {
                        return Err(Unsupported::Type("numeric cast result".into()));
                    }
                    self.define(*result, format!("({}) {name}", target.source()?), result_ty)?;
                } else if source.reference() && target.reference() {
                    self.define(
                        *result,
                        format!("({}) (java.lang.Object) {name}", target.source()?),
                        target,
                    )?;
                } else {
                    return Err(Unsupported::Type(
                        "cast requires numeric or reference types".into(),
                    ));
                }
            }
            ManagedInsn::InstanceOf {
                result,
                value,
                target,
            } => {
                let target = self.target_type(*target)?;
                let (name, source) = self.get(*value)?;
                if !source.reference() || !target.reference() {
                    return Err(Unsupported::Type("instanceof requires references".into()));
                }
                self.define(
                    *result,
                    format!(
                        "((java.lang.Object) {name} instanceof {})",
                        target.source()?
                    ),
                    JavaType::Boolean,
                )?;
            }
            ManagedInsn::NewInstance { .. } => {
                self.object_instruction(insn, signature)?;
            }
            _ => return Err(Unsupported::Instruction("managed operation".into())),
        }
        Ok(())
    }

    // Folded DEX payload constants are arena-only, not missing register inputs.
    // Validate the batch once: giant payloads must not build an index per element.
    fn fill_payload_values(
        &self,
        values: &[ValueId],
        component: &JavaType,
        signature: &JavaMethod,
    ) -> Result<Vec<String>, Unsupported> {
        let synthetic: HashSet<_> = values
            .iter()
            .copied()
            .filter(|v| !self.values.contains_key(v))
            .collect();
        let fail = || Unsupported::Type("unproved synthetic fill-array payload".into());
        if !synthetic.is_empty() {
            let cfg_defs: HashSet<_> = self
                .ir
                .cfg()
                .block_ids()
                .map(|b| {
                    self.ir
                        .cfg()
                        .block(b)
                        .map(|n| *n.payload())
                        .map_err(|_| fail())
                })
                .collect::<Result<_, _>>()?;
            if self
                .ir
                .register_slots()
                .values()
                .any(|slot| slot.values().iter().any(|v| synthetic.contains(v)))
            {
                return Err(fail());
            }
            for value in &synthetic {
                let record = self.ir.value(*value).map_err(|_| fail())?;
                if cfg_defs.contains(&record.def())
                    || self.ty(*value)? != *component
                    || !matches!(self.ir.instruction(record.def()),
                        Ok(ManagedInsn::Const { result, .. }) if result == value)
                {
                    return Err(fail());
                }
            }
            let mut payload_uses = 0;
            for (_, insn) in self.ir.instructions() {
                if let ManagedInsn::FillArray { array, values } = insn {
                    let count = values.iter().filter(|v| synthetic.contains(v)).count();
                    if count != 0 {
                        if self.array(*array)?.1 != *component {
                            return Err(fail());
                        }
                        payload_uses += count;
                    }
                }
            }
            let locals: Vec<_> = synthetic.iter().copied().collect();
            let constraints = contextual_locals::consumer_constraints_with_declarations(
                self.ir,
                &locals,
                &signature.return_type,
                self.resolver,
                &self.declarations,
            )
            .map_err(|_| fail())?;
            if payload_uses == 0
                || constraints.len() != payload_uses
                || constraints.iter().any(|expected| expected != component)
            {
                return Err(fail());
            }
        }
        values
            .iter()
            .map(|value| {
                if !synthetic.contains(value) {
                    return self.array_value(*value, component);
                }
                let record = self.ir.value(*value).map_err(|_| fail())?;
                let ManagedInsn::Const { value: payload, .. } =
                    self.ir.instruction(record.def()).map_err(|_| fail())?
                else {
                    return Err(fail());
                };
                let interpreted = match (*payload, component) {
                    (Constant::Int(bits), JavaType::Byte) if (-128..=255).contains(&bits) => {
                        Constant::Int(i32::from(bits as u8 as i8))
                    }
                    (Constant::Int(bits), JavaType::Short) if (-32768..=65535).contains(&bits) => {
                        Constant::Int(i32::from(bits as u16 as i16))
                    }
                    (Constant::Int(bits), JavaType::Char) if (0..=65535).contains(&bits) => {
                        Constant::Int(bits)
                    }
                    (Constant::Int(bits @ (0 | 1)), JavaType::Boolean) => {
                        Constant::Boolean(bits != 0)
                    }
                    (Constant::Int(bits), JavaType::Int) => Constant::Int(bits),
                    (Constant::Long(bits), JavaType::Long) => Constant::Long(bits),
                    (Constant::Int(bits), JavaType::Float) => Constant::FloatBits(bits as u32),
                    (Constant::Long(bits), JavaType::Double) => Constant::DoubleBits(bits as u64),
                    (Constant::FloatBits(bits), JavaType::Float) => Constant::FloatBits(bits),
                    (Constant::DoubleBits(bits), JavaType::Double) => Constant::DoubleBits(bits),
                    _ => return Err(fail()),
                };
                constant(interpreted, component, self.resolver)
            })
            .collect()
    }

    fn array_value(&self, value: ValueId, component: &JavaType) -> Result<String, Unsupported> {
        let (name, ty) = if *component == JavaType::Boolean {
            self.operand(value, component)?
        } else {
            self.get(value)?
        };
        if ty == JavaType::Int
            && matches!(component, JavaType::Byte | JavaType::Short | JavaType::Char)
        {
            return Ok(format!("({}) {name}", component.source()?));
        }
        self.compatible(&ty, component)?;
        Ok(name)
    }

    pub(super) fn completed_allocation(
        &self,
        result: Option<ValueId>,
        method: MethodRef,
        args: &[ValueId],
    ) -> Result<(), Unsupported> {
        // Valid constructor invocations are consumed by the object hook before
        // generic invoke emission. Reaching this fallback is always unpaired.
        let _ = (result, method, args);
        Err(Unsupported::Instruction("unpaired initializer".into()))
    }
}
