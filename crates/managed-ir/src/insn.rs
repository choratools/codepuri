//! Register-form managed semantic instructions.

use decomp_core::ids::EdgeId;

use crate::effects::EffectFlags;
use crate::refs::{ClassRef, FieldRef, MethodRef, TypeRef};
use crate::value::ValueId;

pub use crate::ops::{ArithOp, CompareOp, Constant, InvokeKind};

/// A family-neutral managed semantic instruction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedInsn {
    /// Defines a constant value.
    Const {
        /// Defined value.
        result: ValueId,
        /// Constant payload.
        value: Constant,
    },
    /// Copies one semantic value.
    Move {
        /// Defined value.
        result: ValueId,
        /// Value being copied.
        value: ValueId,
    },
    /// Merges values selected by predecessor edge identity. Consecutive
    /// phis connected by unique unconditional edges form a simultaneous
    /// cluster: every member selects over the incoming edges of its head.
    /// Additional entries or branching terminate a cluster.
    Phi {
        /// Defined merged value.
        result: ValueId,
        /// One incoming value per predecessor edge.
        inputs: Vec<(EdgeId, ValueId)>,
    },
    /// Applies a binary arithmetic or bitwise operation.
    Arithmetic {
        /// Operation to apply.
        op: ArithOp,
        /// Defined value.
        result: ValueId,
        /// Left operand.
        lhs: ValueId,
        /// Right operand.
        rhs: ValueId,
    },
    /// Compares two values and defines a Boolean result.
    Compare {
        /// Comparison to apply.
        op: CompareOp,
        /// Defined Boolean value.
        result: ValueId,
        /// Left operand.
        lhs: ValueId,
        /// Right operand.
        rhs: ValueId,
    },
    /// Applies arithmetic negation.
    Neg {
        /// Defined value.
        result: ValueId,
        /// Operand being negated.
        value: ValueId,
    },
    /// Applies bitwise or logical negation.
    Not {
        /// Defined value.
        result: ValueId,
        /// Operand being negated.
        value: ValueId,
    },
    /// Reads an instance or static field.
    FieldGet {
        /// Defined field value.
        result: ValueId,
        /// Instance receiver, or `None` for a static field.
        object: Option<ValueId>,
        /// Unresolved field reference.
        field: FieldRef,
    },
    /// Writes an instance or static field.
    FieldPut {
        /// Instance receiver, or `None` for a static field.
        object: Option<ValueId>,
        /// Unresolved field reference.
        field: FieldRef,
        /// Value being stored.
        value: ValueId,
    },
    /// Reads an array element.
    ArrayGet {
        /// Defined element value.
        result: ValueId,
        /// Array reference.
        array: ValueId,
        /// Element index.
        index: ValueId,
    },
    /// Writes an array element.
    ArrayPut {
        /// Array reference.
        array: ValueId,
        /// Element index.
        index: ValueId,
        /// Value being stored.
        value: ValueId,
    },
    /// Reads an array length.
    ArrayLength {
        /// Defined integral length.
        result: ValueId,
        /// Array reference.
        array: ValueId,
    },
    /// Allocates an uninitialized class instance.
    NewInstance {
        /// Defined reference.
        result: ValueId,
        /// Unresolved allocated class.
        class: ClassRef,
    },
    /// Allocates an array.
    NewArray {
        /// Defined array reference.
        result: ValueId,
        /// Unresolved element type.
        element: TypeRef,
        /// Requested array length.
        length: ValueId,
    },
    /// Stores a sequence of values into an existing array.
    FillArray {
        /// Destination array reference.
        array: ValueId,
        /// Values in element order.
        values: Vec<ValueId>,
    },
    /// Invokes a managed method.
    Invoke {
        /// Dispatch semantics.
        kind: InvokeKind,
        /// Optional defined return value.
        result: Option<ValueId>,
        /// Unresolved target method.
        method: MethodRef,
        /// Arguments in call order.
        args: Vec<ValueId>,
    },
    /// Converts or checks a value against a target type.
    Cast {
        /// Defined converted value.
        result: ValueId,
        /// Source value.
        value: ValueId,
        /// Unresolved target type.
        target: TypeRef,
    },
    /// Tests whether a value is an instance of a target type.
    InstanceOf {
        /// Defined Boolean value.
        result: ValueId,
        /// Value being tested.
        value: ValueId,
        /// Unresolved target type.
        target: TypeRef,
    },
    /// Acquires an object's monitor.
    MonitorEnter {
        /// Object whose monitor is acquired.
        object: ValueId,
    },
    /// Releases an object's monitor.
    MonitorExit {
        /// Object whose monitor is released.
        object: ValueId,
    },
    /// Transfers control conditionally or unconditionally.
    Branch {
        /// Branch predicate, or `None` for an unconditional transfer.
        condition: Option<ValueId>,
        /// Taken or sole successor edge.
        then_edge: EdgeId,
        /// Fallthrough edge for a conditional transfer.
        else_edge: Option<EdgeId>,
    },
    /// Selects a successor edge from an integral key.
    Switch {
        /// Integral selector.
        key: ValueId,
        /// Edge used when no case matches.
        default_edge: EdgeId,
        /// Signed case value and successor-edge pairs.
        cases: Vec<(i64, EdgeId)>,
    },
    /// Returns from the current method.
    Return {
        /// Optional returned value.
        value: Option<ValueId>,
    },
    /// Throws a managed exception value.
    Throw {
        /// Exception value being thrown.
        value: ValueId,
    },
    /// Preserves an unmodeled family instruction and its dataflow contract.
    Opaque {
        /// Frontend-owned instruction category.
        family_tag: u16,
        /// Values consumed by the instruction.
        reads: Vec<ValueId>,
        /// Values defined by the instruction.
        writes: Vec<ValueId>,
        /// Conservatively declared semantic effects.
        effects: EffectFlags,
    },
}
