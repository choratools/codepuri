//! JVM array clone intrinsic proof helpers.
//!
//! JLS 10.7 gives every array type a public `clone` operation with no checked
//! exceptions. Class files encode that invocation on the concrete array owner
//! with descriptor `()Ljava/lang/Object;`; source emission must keep the array
//! receiver operation instead of fabricating an ordinary class declaration.

use crate::java_aliases::MethodKey;
use backend_java::{JavaMethod, JavaType};

/// Returns true for the exact classfile/DEX array-clone descriptor shape.
pub(crate) fn is_array_clone_key(key: &MethodKey) -> bool {
    key.owner.starts_with('[')
        && key.name == "clone"
        && key.parameters.is_empty()
        && key.return_type == "Ljava/lang/Object;"
}

/// Build the source-level intrinsic method for a proven array receiver.
pub(crate) fn method_for_receiver(key: &MethodKey, receiver: &JavaType) -> Option<JavaMethod> {
    if !is_array_clone_key(key) || !matches!(receiver, JavaType::Array(_)) {
        return None;
    }
    Some(JavaMethod {
        // The backend uses this only as declaration metadata for the intrinsic;
        // the receiver expression itself is emitted directly, so no class owner
        // is fabricated for the array descriptor.
        owner: "java.lang.Object".into(),
        name: "clone".into(),
        is_static: false,
        return_type: JavaType::Class("java.lang.Object".into()),
        parameters: vec![],
        throws: vec![],
    })
}
