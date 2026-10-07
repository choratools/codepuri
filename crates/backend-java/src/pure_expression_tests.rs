use super::*;
use decomp_core::{cfg::EdgeKind, ids::BlockId};
use std::collections::HashMap;

struct PureResolver;

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
    assert_eq!(source.lines().count(), 15, "{source}");

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
fn multi_use_arithmetic_remains_materialized_instead_of_being_duplicated() {
    let mut builder = Builder::default();
    let sentinel = builder.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    });
    let left = builder.parameter(ManagedType::Int);
    let right = builder.parameter(ManagedType::Int);
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
            vec![
                JavaType::Array(Box::new(JavaType::Int)),
                JavaType::Int,
                JavaType::Int,
            ],
        ),
        &ParameterBindings {
            parameters: vec![sentinel, left, right],
            names: vec!["sentinel".into(), "left".into(), "right".into()],
            ..Default::default()
        },
        &PureResolver,
    ) else {
        panic!("multi-use fixture rejected")
    };
    assert!(source.contains("int v1 = (left + right);"), "{source}");
    assert!(source.contains("return (v1 == v1);"), "{source}");
    java(
        &source,
        "if(!sumUsedTwice(new int[1], 2, 3)) throw new AssertionError();",
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
