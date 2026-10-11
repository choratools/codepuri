use super::*;
use decomp_core::{cfg::EdgeKind, ids::BlockId};
use std::collections::HashMap;

struct PureResolver;

struct EffectResolver;
impl JavaResolver for EffectResolver {
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        (reference.index() == 0).then(|| signature("tick", JavaType::Void, vec![]))
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(9)
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        PureResolver.field(reference)
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        PureResolver.class(reference)
    }
    fn string(&self, reference: StringRef) -> Option<Vec<u16>> {
        PureResolver.string(reference)
    }
}

impl JavaResolver for PureResolver {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }

    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: "bytes".into(),
            ty: JavaType::Array(Box::new(JavaType::Byte)),
            is_static: false,
        })
    }

    fn field_access(&self, _: FieldRef) -> Option<u32> {
        Some(0x0010)
    }

    fn class(&self, reference: ClassRef) -> Option<String> {
        (reference.index() == 0).then(|| "Fixture".into())
    }

    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}

struct Threshold {
    instructions: [InsnId; 4],
    condition: ValueId,
    branch: InsnId,
}

fn append_zero(builder: &mut Builder) -> (ValueId, InsnId) {
    let zero = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(0),
    });
    let id = builder.values.get(zero).unwrap().def();
    (zero, id)
}

fn append_threshold(builder: &mut Builder, input: ValueId, zero: ValueId, limit: i64) -> Threshold {
    let constant = builder.value(ManagedType::Long, |result| ManagedInsn::Const {
        result,
        value: Constant::Long(limit),
    });
    let constant_id = builder.values.get(constant).unwrap().def();
    let ordering = builder.value(ManagedType::Int, |result| ManagedInsn::OrderingCompare {
        op: OrderingOp::CmpLong,
        result,
        lhs: input,
        rhs: constant,
    });
    let ordering_id = builder.values.get(ordering).unwrap().def();
    let condition = builder.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
        op: CompareOp::GreaterEqual,
        result,
        lhs: ordering,
        rhs: zero,
    });
    let compare_id = builder.values.get(condition).unwrap().def();
    builder.push(ManagedInsn::Return { value: None });
    let branch = *builder.order.last().unwrap();
    Threshold {
        instructions: [constant_id, ordering_id, compare_id, branch],
        condition,
        branch,
    }
}

fn append_return(builder: &mut Builder, value: i32) -> (InsnId, InsnId) {
    let result = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(value),
    });
    let constant = builder.values.get(result).unwrap().def();
    builder.push(ManagedInsn::Return {
        value: Some(result),
    });
    let returned = *builder.order.last().unwrap();
    (constant, returned)
}

fn graph_for_offsets(
    mut builder: Builder,
    prefix: [InsnId; 3],
    thresholds: [Threshold; 3],
    returns: [(InsnId, InsnId); 4],
) -> MethodIr<SsaForm> {
    let ids = builder.order.clone();
    let mut cfg = Cfg::new(ids[0]);
    let mut blocks = HashMap::new();
    blocks.insert(ids[0], cfg.entry());
    for instruction in ids.iter().skip(1) {
        blocks.insert(*instruction, cfg.add_block(*instruction));
    }

    let add_edge =
        |cfg: &mut Cfg<InsnId>,
         blocks: &HashMap<InsnId, BlockId>,
         from: InsnId,
         to: InsnId,
         kind: EdgeKind| { cfg.add_edge(blocks[&from], blocks[&to], kind).unwrap() };
    for pair in prefix.windows(2) {
        add_edge(&mut cfg, &blocks, pair[0], pair[1], EdgeKind::Unconditional);
    }
    add_edge(
        &mut cfg,
        &blocks,
        prefix[2],
        thresholds[0].instructions[0],
        EdgeKind::Unconditional,
    );
    for threshold in &thresholds {
        for pair in threshold.instructions.windows(2) {
            add_edge(&mut cfg, &blocks, pair[0], pair[1], EdgeKind::Unconditional);
        }
    }

    let true_targets = [
        thresholds[1].instructions[0],
        thresholds[2].instructions[0],
        returns[3].0,
    ];
    let false_targets = [returns[0].0, returns[1].0, returns[2].0];
    for index in 0..3 {
        let threshold = &thresholds[index];
        let true_edge = add_edge(
            &mut cfg,
            &blocks,
            threshold.branch,
            true_targets[index],
            EdgeKind::BranchTrue,
        );
        let false_edge = add_edge(
            &mut cfg,
            &blocks,
            threshold.branch,
            false_targets[index],
            EdgeKind::BranchFalse,
        );
        builder.insns[threshold.branch] = ManagedInsn::Branch {
            condition: Some(threshold.condition),
            then_edge: true_edge,
            else_edge: Some(false_edge),
        };
    }
    for (constant, returned) in returns {
        add_edge(
            &mut cfg,
            &blocks,
            constant,
            returned,
            EdgeKind::Unconditional,
        );
    }
    MethodIr::<SsaForm>::from_parts(cfg, builder.insns, builder.values)
}

fn branch_phi_ir(
    mut builder: Builder,
    branch: InsnId,
    condition: ValueId,
    false_block: InsnId,
    phi: ValueId,
    true_value: ValueId,
    false_value: ValueId,
) -> MethodIr<SsaForm> {
    let ids = builder.order.clone();
    let mut cfg = Cfg::new(ids[0]);
    let mut blocks = HashMap::from([(ids[0], cfg.entry())]);
    for id in ids.iter().skip(1) {
        blocks.insert(*id, cfg.add_block(*id));
    }
    let branch_position = ids.iter().position(|id| *id == branch).unwrap();
    for pair in ids[..=branch_position].windows(2) {
        cfg.add_edge(blocks[&pair[0]], blocks[&pair[1]], EdgeKind::Unconditional)
            .unwrap();
    }
    let phi_id = builder.values.get(phi).unwrap().def();
    let then_edge = cfg
        .add_edge(blocks[&branch], blocks[&phi_id], EdgeKind::BranchTrue)
        .unwrap();
    let else_edge = cfg
        .add_edge(blocks[&branch], blocks[&false_block], EdgeKind::BranchFalse)
        .unwrap();
    let false_join = cfg
        .add_edge(
            blocks[&false_block],
            blocks[&phi_id],
            EdgeKind::Unconditional,
        )
        .unwrap();
    let phi_position = ids.iter().position(|id| *id == phi_id).unwrap();
    for pair in ids[phi_position..].windows(2) {
        cfg.add_edge(blocks[&pair[0]], blocks[&pair[1]], EdgeKind::Unconditional)
            .unwrap();
    }
    builder.insns[branch] = ManagedInsn::Branch {
        condition: Some(condition),
        then_edge,
        else_edge: Some(else_edge),
    };
    builder.insns[phi_id] = ManagedInsn::Phi {
        result: phi,
        inputs: vec![(then_edge, true_value), (false_join, false_value)],
    };
    MethodIr::<SsaForm>::from_parts(cfg, builder.insns, builder.values)
}

fn offset_fixture() -> (MethodIr<SsaForm>, ValueId, ValueId) {
    let mut builder = Builder::default();
    let receiver = builder.parameter(ManagedType::Reference(ClassRef::new(0)));
    let offset = builder.parameter(ManagedType::Long);
    let (zero1, zero1_id) = append_zero(&mut builder);
    let (zero2, zero2_id) = append_zero(&mut builder);
    let (zero3, zero3_id) = append_zero(&mut builder);
    let thresholds = [
        append_threshold(&mut builder, offset, zero1, 256),
        append_threshold(&mut builder, offset, zero2, 65_536),
        append_threshold(&mut builder, offset, zero3, 4_294_967_296),
    ];
    let returns = [
        append_return(&mut builder, 1),
        append_return(&mut builder, 2),
        append_return(&mut builder, 4),
        append_return(&mut builder, 8),
    ];
    (
        graph_for_offsets(builder, [zero1_id, zero2_id, zero3_id], thresholds, returns),
        receiver,
        offset,
    )
}

fn array_type() -> ManagedType {
    ManagedType::Array {
        element: Box::new(ManagedType::Byte),
        nullable: true,
    }
}

fn instance_method(name: &str, return_type: JavaType, parameters: Vec<JavaType>) -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: name.into(),
        is_static: false,
        return_type,
        parameters,
        throws: vec![],
    }
}

#[test]
fn offset_size_uses_single_use_long_relations_and_executes_boundaries() {
    let (ir, receiver, offset) = offset_fixture();
    let SourceResult::Complete { source, .. } = emit_method(
        &ir,
        &instance_method(
            "computeOffsetSizeInBytes",
            JavaType::Int,
            vec![JavaType::Long],
        ),
        &ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![offset],
            names: vec!["offset".into()],
        },
        &PureResolver,
    ) else {
        panic!("offset fixture rejected")
    };
    assert!(source.contains("offset >= 256L"), "{source}");
    assert!(source.contains("offset >= 65536L"), "{source}");
    assert!(source.contains("offset >= 4294967296L"), "{source}");
    assert!(!source.contains("int v"), "{source}");
    assert!(!source.contains("long v"), "{source}");
    assert!(!source.contains("boolean v"), "{source}");
    assert!(source.lines().count() <= 15, "{source}");

    java(
        &source,
        "Fixture f = new Fixture(); if(f.computeOffsetSizeInBytes(255L) != 1 || f.computeOffsetSizeInBytes(256L) != 2 || f.computeOffsetSizeInBytes(65535L) != 2 || f.computeOffsetSizeInBytes(65536L) != 4 || f.computeOffsetSizeInBytes(4294967295L) != 4 || f.computeOffsetSizeInBytes(4294967296L) != 8 || f.computeOffsetSizeInBytes(Long.MAX_VALUE) != 8 || f.computeOffsetSizeInBytes(-1L) != 1) throw new AssertionError();",
    );
}

#[test]
fn data_length_inlines_proven_this_array_read_and_keeps_null_failure() {
    let mut builder = Builder::default();
    let receiver = builder.parameter(ManagedType::Reference(ClassRef::new(0)));
    let bytes = builder.value(array_type(), |result| ManagedInsn::FieldGet {
        result,
        object: Some(receiver),
        field: FieldRef::new(0),
    });
    let length = builder.value(ManagedType::Int, |result| ManagedInsn::ArrayLength {
        result,
        array: bytes,
    });
    builder.push(ManagedInsn::Return {
        value: Some(length),
    });
    let ir = builder.finish();
    let SourceResult::Complete { source, .. } = emit_method(
        &ir,
        &instance_method("length", JavaType::Int, vec![]),
        &ParameterBindings {
            receiver: Some(receiver),
            ..Default::default()
        },
        &PureResolver,
    ) else {
        panic!("length fixture rejected")
    };
    assert!(source.contains("return this.bytes.length;"), "{source}");
    assert!(!source.contains(" v0 = "), "{source}");
    assert_eq!(source.lines().count(), 3, "{source}");

    java(
        &format!(
            "private final byte[] bytes; Fixture(byte[] bytes) {{ this.bytes = bytes; }} {source}"
        ),
        "if(new Fixture(new byte[0]).length() != 0 || new Fixture(new byte[255]).length() != 255) throw new AssertionError(); try { new Fixture(null).length(); throw new AssertionError(); } catch(NullPointerException expected) {}",
    );
}

#[test]
fn intervening_throwing_array_read_blocks_field_expression_movement() {
    let mut builder = Builder::default();
    let receiver = builder.parameter(ManagedType::Reference(ClassRef::new(0)));
    let other = builder.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    });
    let index = builder.parameter(ManagedType::Int);
    let bytes = builder.value(array_type(), |result| ManagedInsn::FieldGet {
        result,
        object: Some(receiver),
        field: FieldRef::new(0),
    });
    let _intervening = builder.value(ManagedType::Int, |result| ManagedInsn::ArrayGet {
        result,
        array: other,
        index,
    });
    let length = builder.value(ManagedType::Int, |result| ManagedInsn::ArrayLength {
        result,
        array: bytes,
    });
    builder.push(ManagedInsn::Return {
        value: Some(length),
    });
    let ir = builder.finish();
    let SourceResult::Complete { source, .. } = emit_method(
        &ir,
        &instance_method(
            "lengthAfterRead",
            JavaType::Int,
            vec![JavaType::Array(Box::new(JavaType::Int)), JavaType::Int],
        ),
        &ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![other, index],
            names: vec!["other".into(), "index".into()],
        },
        &PureResolver,
    ) else {
        panic!("ordered length fixture rejected")
    };
    assert!(!source.contains("this.bytes.length"), "{source}");
    assert!(source.contains("this.bytes"), "{source}");
    assert!(source.contains("other[index]"), "{source}");

    java(
        &format!(
            "private final byte[] bytes; Fixture(byte[] bytes) {{ this.bytes = bytes; }} {source}"
        ),
        "try { new Fixture(null).lengthAfterRead(new int[0], 0); throw new AssertionError(); } catch(ArrayIndexOutOfBoundsException expected) {}",
    );
}

#[test]
fn single_use_integer_arithmetic_keeps_dex_width_and_overflow() {
    let mut builder = Builder::default();
    let sentinel = builder.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    });
    let left = builder.parameter(ManagedType::Int);
    let right = builder.parameter(ManagedType::Int);
    let expected = builder.parameter(ManagedType::Int);
    let _length = builder.value(ManagedType::Int, |result| ManagedInsn::ArrayLength {
        result,
        array: sentinel,
    });
    let sum = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: left,
        rhs: right,
    });
    let matches = builder.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
        op: CompareOp::Equal,
        result,
        lhs: sum,
        rhs: expected,
    });
    builder.push(ManagedInsn::Return {
        value: Some(matches),
    });
    let int_ir = builder.finish();
    let int_method = signature(
        "intSum",
        JavaType::Boolean,
        vec![
            JavaType::Array(Box::new(JavaType::Int)),
            JavaType::Int,
            JavaType::Int,
            JavaType::Int,
        ],
    );
    let int_source = complete_with(
        &int_ir,
        int_method,
        ParameterBindings {
            parameters: vec![sentinel, left, right, expected],
            names: vec![
                "sentinel".into(),
                "left".into(),
                "right".into(),
                "expected".into(),
            ],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(
        int_source.contains("return ((left + right) == expected);"),
        "{int_source}"
    );
    assert!(
        int_source.contains("int v0 = sentinel.length;"),
        "{int_source}"
    );
    assert!(
        !int_source.contains("int v1 = (left + right)"),
        "{int_source}"
    );

    let mut builder = Builder::default();
    let sentinel = builder.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    });
    let left = builder.parameter(ManagedType::Long);
    let right = builder.parameter(ManagedType::Long);
    let expected = builder.parameter(ManagedType::Long);
    let _length = builder.value(ManagedType::Int, |result| ManagedInsn::ArrayLength {
        result,
        array: sentinel,
    });
    let sum = builder.value(ManagedType::Long, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: left,
        rhs: right,
    });
    let matches = builder.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
        op: CompareOp::Equal,
        result,
        lhs: sum,
        rhs: expected,
    });
    builder.push(ManagedInsn::Return {
        value: Some(matches),
    });
    let long_ir = builder.finish();
    let long_method = signature(
        "longSum",
        JavaType::Boolean,
        vec![
            JavaType::Array(Box::new(JavaType::Int)),
            JavaType::Long,
            JavaType::Long,
            JavaType::Long,
        ],
    );
    let long_source = complete_with(
        &long_ir,
        long_method,
        ParameterBindings {
            parameters: vec![sentinel, left, right, expected],
            names: vec![
                "sentinel".into(),
                "left".into(),
                "right".into(),
                "expected".into(),
            ],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(
        long_source.contains("return ((left + right) == expected);"),
        "{long_source}"
    );
    assert!(
        long_source.contains("int v0 = sentinel.length;"),
        "{long_source}"
    );
    assert!(
        !long_source.contains("long v1 = (left + right)"),
        "{long_source}"
    );

    java(
        &(int_source + &long_source),
        "if(!intSum(new int[1], Integer.MAX_VALUE, 1, Integer.MIN_VALUE) || !longSum(new int[1], Long.MAX_VALUE, 1L, Long.MIN_VALUE)) throw new AssertionError();",
    );
}

#[test]
fn add_multiply_chain_inlines_single_use_values_and_exact_constants() {
    let mut builder = Builder::default();
    let input = builder.parameter(ManagedType::Int);
    let one = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let sum = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: input,
        rhs: one,
    });
    let two = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(2),
    });
    let product = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Mul,
        result,
        lhs: sum,
        rhs: two,
    });
    builder.push(ManagedInsn::Return {
        value: Some(product),
    });
    let ir = builder.finish();
    let source = complete_with(
        &ir,
        signature("addMultiply", JavaType::Int, vec![JavaType::Int]),
        ParameterBindings {
            parameters: vec![input],
            names: vec!["p0".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(source.contains("return ((p0 + 1) * 2);"), "{source}");
    assert!(!source.contains(" v0 = "), "{source}");
    java(
        &source,
        "if(addMultiply(5) != 12 || addMultiply(Integer.MAX_VALUE) != 0 || addMultiply(-2) != -2) throw new AssertionError();",
    );
}

#[test]
fn ten_local_assignments_collapse_even_with_an_unrelated_effect_before_them() {
    let mut builder = Builder::default();
    let input = builder.parameter(ManagedType::Int);
    builder.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![],
    });
    let mut value = input;
    for number in 1..=10 {
        let constant = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(number),
        });
        value = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
            op: ArithOp::Add,
            result,
            lhs: value,
            rhs: constant,
        });
    }
    builder.push(ManagedInsn::Return { value: Some(value) });
    let ir = builder.finish();
    let source = complete_with(
        &ir,
        signature("chain", JavaType::Int, vec![JavaType::Int]),
        ParameterBindings {
            parameters: vec![input],
            names: vec!["x".into()],
            ..Default::default()
        },
        &EffectResolver,
    );
    assert!(source.contains("Fixture.tick();"), "{source}");
    assert!(!source.contains("int v"), "{source}");
    assert_eq!(source.matches("return ").count(), 1);
    java(
        &(source + "static int effects; static void tick(){effects++;}"),
        "for(int x:new int[]{0,17,Integer.MAX_VALUE,Integer.MIN_VALUE}){int before=effects;if(chain(x)!=x+55 || effects!=before+1)throw new AssertionError();}",
    );
}

#[test]
fn local_expression_does_not_move_across_an_intervening_invocation() {
    let mut builder = Builder::default();
    let input = builder.parameter(ManagedType::Int);
    let one = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let value = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: input,
        rhs: one,
    });
    builder.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![],
    });
    builder.push(ManagedInsn::Return { value: Some(value) });
    let ir = builder.finish();
    let source = complete_with(
        &ir,
        signature("barrier", JavaType::Int, vec![JavaType::Int]),
        ParameterBindings {
            parameters: vec![input],
            names: vec!["x".into()],
            ..Default::default()
        },
        &EffectResolver,
    );
    assert!(source.contains("int v"), "{source}");
    assert!(source.find("int v").unwrap() < source.find("Fixture.tick();").unwrap());
    java(
        &(source + "static int effects; static void tick(){effects++;}"),
        "if(barrier(9)!=10 || effects!=1)throw new AssertionError();",
    );
}

#[test]
fn local_expression_crosses_only_inert_primitive_work() {
    let mut builder = Builder::default();
    let input = builder.parameter(ManagedType::Int);
    let one = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let incremented = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: input,
        rhs: one,
    });
    let two = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(2),
    });
    let doubled = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Mul,
        result,
        lhs: input,
        rhs: two,
    });
    let result = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: incremented,
        rhs: doubled,
    });
    builder.push(ManagedInsn::Return {
        value: Some(result),
    });
    let ir = builder.finish();
    let source = complete_with(
        &ir,
        signature("inertGap", JavaType::Int, vec![JavaType::Int]),
        ParameterBindings {
            parameters: vec![input],
            names: vec!["x".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(source.contains("return ((x + 1) + (x * 2));"), "{source}");
    assert!(!source.contains("int v"), "{source}");
    java(
        &source,
        "for(int x:new int[]{0,1,-1,Integer.MAX_VALUE,Integer.MIN_VALUE})if(inertGap(x)!=(x+1)+(x*2))throw new AssertionError();",
    );
}

#[test]
fn boolean_primitive_expression_can_feed_a_branch_directly() {
    let mut builder = Builder::default();
    let left = builder.parameter(ManagedType::Boolean);
    let right = builder.parameter(ManagedType::Boolean);
    let condition = builder.value(ManagedType::Boolean, |result| ManagedInsn::Arithmetic {
        op: ArithOp::And,
        result,
        lhs: left,
        rhs: right,
    });
    builder.push(ManagedInsn::Return { value: None });
    let branch = *builder.order.last().unwrap();
    let yes = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    builder.push(ManagedInsn::Return { value: Some(yes) });
    let yes_return = *builder.order.last().unwrap();
    let no = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(0),
    });
    builder.push(ManagedInsn::Return { value: Some(no) });
    let no_return = *builder.order.last().unwrap();

    let ids = builder.order.clone();
    let mut cfg = Cfg::new(ids[0]);
    let mut blocks = HashMap::from([(ids[0], cfg.entry())]);
    for id in ids.iter().skip(1) {
        blocks.insert(*id, cfg.add_block(*id));
    }
    cfg.add_edge(blocks[&ids[0]], blocks[&branch], EdgeKind::Unconditional)
        .unwrap();
    let then_edge = cfg
        .add_edge(
            blocks[&branch],
            blocks[&builder.values.get(yes).unwrap().def()],
            EdgeKind::BranchTrue,
        )
        .unwrap();
    let else_edge = cfg
        .add_edge(
            blocks[&branch],
            blocks[&builder.values.get(no).unwrap().def()],
            EdgeKind::BranchFalse,
        )
        .unwrap();
    cfg.add_edge(
        blocks[&builder.values.get(yes).unwrap().def()],
        blocks[&yes_return],
        EdgeKind::Unconditional,
    )
    .unwrap();
    cfg.add_edge(
        blocks[&builder.values.get(no).unwrap().def()],
        blocks[&no_return],
        EdgeKind::Unconditional,
    )
    .unwrap();
    builder.insns[branch] = ManagedInsn::Branch {
        condition: Some(condition),
        then_edge,
        else_edge: Some(else_edge),
    };
    let ir = MethodIr::<SsaForm>::from_parts(cfg, builder.insns, builder.values);
    let planned = pure_expression::plan(&ir, None, &HashMap::new(), true, &PureResolver);
    assert!(planned.contains(&condition));
    let source = complete_with(
        &ir,
        signature(
            "booleanBranch",
            JavaType::Int,
            vec![JavaType::Boolean, JavaType::Boolean],
        ),
        ParameterBindings {
            parameters: vec![left, right],
            names: vec!["left".into(), "right".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(!source.contains("boolean v"), "{source}");
    java(
        &source,
        "if(booleanBranch(false,false)!=0 || booleanBranch(true,false)!=0 || booleanBranch(false,true)!=0 || booleanBranch(true,true)!=1)throw new AssertionError();",
    );
}

#[test]
fn ten_same_type_copy_assignments_become_one_return() {
    let mut builder = Builder::default();
    let input = builder.parameter(ManagedType::Int);
    let mut value = input;
    for _ in 0..10 {
        value = builder.value(ManagedType::Int, |result| ManagedInsn::Move {
            result,
            value,
        });
    }
    builder.push(ManagedInsn::Return { value: Some(value) });
    let ir = builder.finish();
    let source = complete_with(
        &ir,
        signature("copies", JavaType::Int, vec![JavaType::Int]),
        ParameterBindings {
            parameters: vec![input],
            names: vec!["x".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(source.contains("return x;"), "{source}");
    assert!(!source.contains("int v"), "{source}");
    java(
        &source,
        "for(int x:new int[]{0,-1,Integer.MAX_VALUE,Integer.MIN_VALUE})if(copies(x)!=x)throw new AssertionError();",
    );
}

#[test]
fn long_arithmetic_chain_keeps_bounded_java_parser_nesting() {
    let mut builder = Builder::default();
    let input = builder.parameter(ManagedType::Int);
    let mut value = input;
    for _ in 0..200 {
        let one = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(1),
        });
        value = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
            op: ArithOp::Add,
            result,
            lhs: value,
            rhs: one,
        });
    }
    builder.push(ManagedInsn::Return { value: Some(value) });
    let ir = builder.finish();
    let source = complete_with(
        &ir,
        signature("bounded", JavaType::Int, vec![JavaType::Int]),
        ParameterBindings {
            parameters: vec![input],
            names: vec!["x".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(source.contains("int v"), "{source}");
    java(
        &source,
        "for(int x:new int[]{0,-1,Integer.MAX_VALUE,Integer.MIN_VALUE})if(bounded(x)!=x+200)throw new AssertionError();",
    );
}

#[test]
fn balanced_expression_tree_has_bounded_rendered_operator_count() {
    let mut builder = Builder::default();
    let input = builder.parameter(ManagedType::Int);
    let mut level = vec![input; 256];
    while level.len() > 1 {
        level = level
            .chunks_exact(2)
            .map(|pair| {
                builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
                    op: ArithOp::Add,
                    result,
                    lhs: pair[0],
                    rhs: pair[1],
                })
            })
            .collect();
    }
    let result = level[0];
    builder.push(ManagedInsn::Return {
        value: Some(result),
    });
    let ir = builder.finish();
    let source = complete_with(
        &ir,
        signature("balanced", JavaType::Int, vec![JavaType::Int]),
        ParameterBindings {
            parameters: vec![input],
            names: vec!["x".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(source.contains("int v"), "{source}");
    for line in source.lines() {
        assert!(line.matches(" + ").count() <= 128, "{line}");
    }
    java(
        &source,
        "for(int x:new int[]{0,1,-1,Integer.MAX_VALUE,Integer.MIN_VALUE})if(balanced(x)!=x*256)throw new AssertionError();",
    );
}

#[test]
fn unary_and_comparison_results_inline_into_returns() {
    let mut builder = Builder::default();
    let input = builder.parameter(ManagedType::Int);
    let negated = builder.value(ManagedType::Int, |result| ManagedInsn::Neg {
        result,
        value: input,
    });
    let zero = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(0),
    });
    let positive = builder.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
        op: CompareOp::GreaterThan,
        result,
        lhs: negated,
        rhs: zero,
    });
    builder.push(ManagedInsn::Return {
        value: Some(positive),
    });
    let compare_ir = builder.finish();
    let compare_source = complete_with(
        &compare_ir,
        signature("negatedPositive", JavaType::Boolean, vec![JavaType::Int]),
        ParameterBindings {
            parameters: vec![input],
            names: vec!["input".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(
        compare_source.contains("return ((- input) > 0);"),
        "{compare_source}"
    );
    assert!(!compare_source.contains(" v0 = "), "{compare_source}");

    let mut builder = Builder::default();
    let left = builder.parameter(ManagedType::Boolean);
    let right = builder.parameter(ManagedType::Boolean);
    let conjunction = builder.value(ManagedType::Boolean, |result| ManagedInsn::Arithmetic {
        op: ArithOp::And,
        result,
        lhs: left,
        rhs: right,
    });
    let inverted = builder.value(ManagedType::Boolean, |result| ManagedInsn::Not {
        result,
        value: conjunction,
    });
    builder.push(ManagedInsn::Return {
        value: Some(inverted),
    });
    let boolean_ir = builder.finish();
    let boolean_source = complete_with(
        &boolean_ir,
        signature(
            "notConjunction",
            JavaType::Boolean,
            vec![JavaType::Boolean, JavaType::Boolean],
        ),
        ParameterBindings {
            parameters: vec![left, right],
            names: vec!["left".into(), "right".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(
        boolean_source.contains("return (!(left & right));"),
        "{boolean_source}"
    );
    assert!(!boolean_source.contains("boolean v"), "{boolean_source}");

    let mut builder = Builder::default();
    let negative_one = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(-1),
    });
    let positive_one = builder.value(ManagedType::Int, |result| ManagedInsn::Neg {
        result,
        value: negative_one,
    });
    builder.push(ManagedInsn::Return {
        value: Some(positive_one),
    });
    let literal_ir = builder.finish();
    let literal_source = complete_with(
        &literal_ir,
        signature("negateMinusOne", JavaType::Int, vec![]),
        ParameterBindings::default(),
        &PureResolver,
    );
    assert!(
        literal_source.contains("return (- -1);"),
        "{literal_source}"
    );

    java(
        &(compare_source + &boolean_source + &literal_source),
        "if(negatedPositive(1) || !negatedPositive(-1) || negatedPositive(Integer.MIN_VALUE)) throw new AssertionError(); if(!notConjunction(false, false) || !notConjunction(true, false) || !notConjunction(false, true) || notConjunction(true, true)) throw new AssertionError(); if(negateMinusOne() != 1) throw new AssertionError();",
    );
}

#[test]
fn nested_arithmetic_preserves_source_precedence() {
    let mut builder = Builder::default();
    let scale = builder.parameter(ManagedType::Int);
    let minuend = builder.parameter(ManagedType::Int);
    let subtrahend = builder.parameter(ManagedType::Int);
    let difference = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Sub,
        result,
        lhs: minuend,
        rhs: subtrahend,
    });
    let product = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Mul,
        result,
        lhs: scale,
        rhs: difference,
    });
    builder.push(ManagedInsn::Return {
        value: Some(product),
    });
    let ir = builder.finish();
    let source = complete_with(
        &ir,
        signature(
            "scaledDifference",
            JavaType::Int,
            vec![JavaType::Int, JavaType::Int, JavaType::Int],
        ),
        ParameterBindings {
            parameters: vec![scale, minuend, subtrahend],
            names: vec!["scale".into(), "minuend".into(), "subtrahend".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(
        source.contains("return (scale * (minuend - subtrahend));"),
        "{source}"
    );
    java(
        &source,
        "if(scaledDifference(3, 10, 4) != 18 || scaledDifference(2, Integer.MIN_VALUE, 1) != -2) throw new AssertionError();",
    );
}

#[test]
fn division_by_zero_remains_at_its_original_materialized_site() {
    let mut builder = Builder::default();
    let numerator = builder.parameter(ManagedType::Int);
    let denominator = builder.parameter(ManagedType::Int);
    let quotient = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Div,
        result,
        lhs: numerator,
        rhs: denominator,
    });
    let one = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let incremented = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: quotient,
        rhs: one,
    });
    builder.push(ManagedInsn::Return {
        value: Some(incremented),
    });
    let ir = builder.finish();
    let source = complete_with(
        &ir,
        signature(
            "divideThenIncrement",
            JavaType::Int,
            vec![JavaType::Int, JavaType::Int],
        ),
        ParameterBindings {
            parameters: vec![numerator, denominator],
            names: vec!["numerator".into(), "denominator".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(
        source.contains("int v0 = (numerator / denominator);"),
        "{source}"
    );
    assert!(source.contains("return (v0 + 1);"), "{source}");
    java(
        &source,
        "if(divideThenIncrement(8, 2) != 5) throw new AssertionError(); try { divideThenIncrement(8, 0); throw new AssertionError(); } catch(ArithmeticException expected) {}",
    );
}

#[test]
fn branch_phi_and_exception_guards_keep_values_materialized() {
    let mut builder = Builder::default();
    let input = builder.parameter(ManagedType::Int);
    let one = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let sum = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: input,
        rhs: one,
    });
    builder.push(ManagedInsn::Return { value: Some(sum) });
    let straight_line = builder.finish();
    let normal_plan =
        pure_expression::plan(&straight_line, None, &HashMap::new(), true, &PureResolver);
    assert!(normal_plan.contains(&sum));
    let scoped_plan =
        pure_expression::plan(&straight_line, None, &HashMap::new(), false, &PureResolver);
    assert!(scoped_plan.is_empty());

    let mut builder = Builder::default();
    let condition = builder.parameter(ManagedType::Boolean);
    let left = builder.parameter(ManagedType::Int);
    let right = builder.parameter(ManagedType::Int);
    let sum = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: left,
        rhs: right,
    });
    builder.push(ManagedInsn::Return { value: None });
    let branch_id = *builder.order.last().unwrap();
    builder.push(ManagedInsn::Return { value: Some(sum) });
    let true_return = *builder.order.last().unwrap();
    let (false_constant, false_return) = append_return(&mut builder, 0);
    let ids = builder.order.clone();
    let mut cfg = Cfg::new(ids[0]);
    let mut blocks = HashMap::from([(ids[0], cfg.entry())]);
    for id in ids.iter().skip(1) {
        blocks.insert(*id, cfg.add_block(*id));
    }
    let add_edge = |cfg: &mut Cfg<InsnId>, from, to, kind| {
        cfg.add_edge(blocks[&from], blocks[&to], kind).unwrap()
    };
    add_edge(&mut cfg, ids[0], branch_id, EdgeKind::Unconditional);
    let then_edge = add_edge(&mut cfg, branch_id, true_return, EdgeKind::BranchTrue);
    let else_edge = add_edge(&mut cfg, branch_id, false_constant, EdgeKind::BranchFalse);
    add_edge(
        &mut cfg,
        false_constant,
        false_return,
        EdgeKind::Unconditional,
    );
    builder.insns[branch_id] = ManagedInsn::Branch {
        condition: Some(condition),
        then_edge,
        else_edge: Some(else_edge),
    };
    let branch_ir = MethodIr::<SsaForm>::from_parts(cfg, builder.insns, builder.values);
    let branch_plan = pure_expression::plan(&branch_ir, None, &HashMap::new(), true, &PureResolver);
    assert!(!branch_plan.contains(&sum));
    let branch_source = complete_with(
        &branch_ir,
        signature(
            "branchKeepsArithmetic",
            JavaType::Int,
            vec![JavaType::Boolean, JavaType::Int, JavaType::Int],
        ),
        ParameterBindings {
            parameters: vec![condition, left, right],
            names: vec!["condition".into(), "left".into(), "right".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(
        branch_source.contains("v0 = (left + right);"),
        "{branch_source}"
    );
    assert!(
        branch_source.contains("condition ? (v0) : (0)"),
        "{branch_source}"
    );
    assert_eq!(
        branch_source.matches("(left + right)").count(),
        1,
        "{branch_source}"
    );
    java(
        &branch_source,
        "if(branchKeepsArithmetic(true, Integer.MAX_VALUE, 1) != Integer.MIN_VALUE || branchKeepsArithmetic(false, 4, 5) != 0) throw new AssertionError();",
    );

    let mut builder = Builder::default();
    let left = builder.parameter(ManagedType::Int);
    let right = builder.parameter(ManagedType::Int);
    let sum = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: left,
        rhs: right,
    });
    let phi = builder.value(ManagedType::Int, |result| ManagedInsn::Phi {
        result,
        inputs: Vec::new(),
    });
    builder.push(ManagedInsn::Return { value: Some(phi) });
    let ids = builder.order.clone();
    let mut cfg = Cfg::new(ids[0]);
    let mut blocks = HashMap::from([(ids[0], cfg.entry())]);
    for id in ids.iter().skip(1) {
        blocks.insert(*id, cfg.add_block(*id));
    }
    let edge = cfg
        .add_edge(blocks[&ids[0]], blocks[&ids[1]], EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(blocks[&ids[1]], blocks[&ids[2]], EdgeKind::Unconditional)
        .unwrap();
    let phi_id = builder.values.get(phi).unwrap().def();
    builder.insns[phi_id] = ManagedInsn::Phi {
        result: phi,
        inputs: vec![(edge, sum)],
    };
    let phi_ir = MethodIr::<SsaForm>::from_parts(cfg, builder.insns, builder.values);
    let phi_plan = pure_expression::plan(&phi_ir, None, &HashMap::new(), true, &PureResolver);
    assert!(!phi_plan.contains(&sum));
}

#[test]
fn phi_edge_read_keeps_branch_compare_materialized() {
    let mut builder = Builder::default();
    let left = builder.parameter(ManagedType::Int);
    let right = builder.parameter(ManagedType::Int);
    let condition = builder.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
        op: CompareOp::GreaterThan,
        result,
        lhs: left,
        rhs: right,
    });
    builder.push(ManagedInsn::Return { value: None });
    let branch = *builder.order.last().unwrap();
    let other = builder.value(ManagedType::Boolean, |result| ManagedInsn::Const {
        result,
        value: Constant::Boolean(false),
    });
    let false_block = builder.values.get(other).unwrap().def();
    let selected = builder.value(ManagedType::Boolean, |result| ManagedInsn::Phi {
        result,
        inputs: Vec::new(),
    });
    builder.push(ManagedInsn::Return {
        value: Some(selected),
    });
    let ir = branch_phi_ir(
        builder,
        branch,
        condition,
        false_block,
        selected,
        condition,
        other,
    );

    let planned = pure_expression::plan(&ir, None, &HashMap::new(), true, &PureResolver);
    assert!(!planned.contains(&condition));
    let source = complete_with(
        &ir,
        signature(
            "compareFeedsBranchAndPhi",
            JavaType::Boolean,
            vec![JavaType::Int, JavaType::Int],
        ),
        ParameterBindings {
            parameters: vec![left, right],
            names: vec!["left".into(), "right".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(source.contains("boolean v0;"), "{source}");
    assert!(source.contains("v0 = (left > right);"), "{source}");
    assert!(source.contains("if (v0)"), "{source}");
    assert!(source.contains(" = v0;"), "{source}");
    java(
        &source,
        "for(int left=-2;left<=2;left++)for(int right=-2;right<=2;right++)if(compareFeedsBranchAndPhi(left,right)!=(left>right))throw new AssertionError();",
    );
}

#[test]
fn phi_edge_read_prevents_inlining_a_single_use_copy_assignment() {
    let mut builder = Builder::default();
    let condition = builder.parameter(ManagedType::Boolean);
    let input = builder.parameter(ManagedType::Int);
    let alternate = builder.parameter(ManagedType::Int);
    let first_copy = builder.value(ManagedType::Int, |result| ManagedInsn::Move {
        result,
        value: input,
    });
    let second_copy = builder.value(ManagedType::Int, |result| ManagedInsn::Move {
        result,
        value: first_copy,
    });
    builder.push(ManagedInsn::Return { value: None });
    let branch = *builder.order.last().unwrap();
    let false_copy = builder.value(ManagedType::Int, |result| ManagedInsn::Move {
        result,
        value: alternate,
    });
    let false_block = builder.values.get(false_copy).unwrap().def();
    let selected = builder.value(ManagedType::Int, |result| ManagedInsn::Phi {
        result,
        inputs: Vec::new(),
    });
    let combined = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: second_copy,
        rhs: selected,
    });
    builder.push(ManagedInsn::Return {
        value: Some(combined),
    });
    let ir = branch_phi_ir(
        builder,
        branch,
        condition,
        false_block,
        selected,
        first_copy,
        false_copy,
    );
    let source = complete_with(
        &ir,
        signature(
            "copyFeedsPhi",
            JavaType::Int,
            vec![JavaType::Boolean, JavaType::Int, JavaType::Int],
        ),
        ParameterBindings {
            parameters: vec![condition, input, alternate],
            names: vec!["condition".into(), "input".into(), "alternate".into()],
            ..Default::default()
        },
        &PureResolver,
    );
    assert!(source.contains("v0 = input;"), "{source}");
    assert!(source.contains("v1 = v0;"), "{source}");
    assert!(source.contains(" = v0;"), "{source}");
    java(
        &source,
        "for(int input:new int[]{-7,0,11,Integer.MAX_VALUE})for(int alternate:new int[]{-3,4,Integer.MIN_VALUE}){if(copyFeedsPhi(true,input,alternate)!=input+input)throw new AssertionError();if(copyFeedsPhi(false,input,alternate)!=input+alternate)throw new AssertionError();}",
    );
}

#[test]
fn cyclic_cfg_keeps_expression_values_materialized_for_loop_hoisting() {
    let mut builder = Builder::default();
    let input = builder.parameter(ManagedType::Int);
    let one = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let sum = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: input,
        rhs: one,
    });
    builder.push(ManagedInsn::Return { value: Some(sum) });
    let ids = builder.order.clone();
    let mut cfg = Cfg::new(ids[0]);
    let mut blocks = HashMap::from([(ids[0], cfg.entry())]);
    for id in ids.iter().skip(1) {
        blocks.insert(*id, cfg.add_block(*id));
    }
    cfg.add_edge(blocks[&ids[0]], blocks[&ids[1]], EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(blocks[&ids[1]], blocks[&ids[2]], EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(blocks[&ids[2]], blocks[&ids[1]], EdgeKind::Unconditional)
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, builder.insns, builder.values);
    let planned = pure_expression::plan(&ir, None, &HashMap::new(), true, &PureResolver);
    assert!(planned.is_empty());
}

#[test]
fn multi_use_arithmetic_remains_materialized_instead_of_being_duplicated() {
    let mut builder = Builder::default();
    let left = builder.parameter(ManagedType::Int);
    let right = builder.parameter(ManagedType::Int);
    let sum = builder.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: left,
        rhs: right,
    });
    let same = builder.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
        op: CompareOp::Equal,
        result,
        lhs: sum,
        rhs: sum,
    });
    builder.push(ManagedInsn::Return { value: Some(same) });
    let ir = builder.finish();
    let SourceResult::Complete { source, .. } = emit_method(
        &ir,
        &signature(
            "sumUsedTwice",
            JavaType::Boolean,
            vec![JavaType::Int, JavaType::Int],
        ),
        &ParameterBindings {
            parameters: vec![left, right],
            names: vec!["left".into(), "right".into()],
            ..Default::default()
        },
        &PureResolver,
    ) else {
        panic!("multi-use fixture rejected")
    };
    assert!(source.contains("int v0 = (left + right);"), "{source}");
    assert!(source.contains("return (v0 == v0);"), "{source}");
    java(
        &source,
        "if(!sumUsedTwice(2, 3)) throw new AssertionError();",
    );
}

fn complete_with(
    ir: &MethodIr<SsaForm>,
    method: JavaMethod,
    bindings: ParameterBindings,
    resolver: &impl JavaResolver,
) -> String {
    match emit_method(ir, &method, &bindings, resolver) {
        SourceResult::Complete { source, .. } => source,
        other => panic!("{other:?}"),
    }
}
