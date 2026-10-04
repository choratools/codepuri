//! Descriptor-preserving class-variable erasure stays separate from inference.
use super::*;
fn method(parameters: &[&str], return_type: &str) -> MethodKey {
    MethodKey {
        owner: "LBase;".into(),
        name: "accept".into(),
        parameters: parameters.iter().map(|x| (*x).into()).collect(),
        return_type: return_type.into(),
    }
}
#[test]
fn genuine_class_variable_descriptors_include_bounds_arrays_and_returns() {
    for (signature, parameters, ret) in [
        ("(TE;)Z", vec!["Ljava/lang/Object;"], "Z"),
        ("(ITE;)V", vec!["I", "Ljava/lang/Object;"], "V"),
        (
            "([TE;)TE;",
            vec!["[Ljava/lang/Number;"],
            "Ljava/lang/Number;",
        ),
        (
            "(Ljava/lang/String;TT;)TT;",
            vec!["Ljava/lang/String;", "Ljava/lang/CharSequence;"],
            "Ljava/lang/CharSequence;",
        ),
    ] {
        assert!(
            class_variable_signature(signature, &method(&parameters, ret)),
            "{signature}"
        );
    }
}
#[test]
fn generic_method_parameterized_members_and_descriptor_mismatches_stay_closed() {
    let target = method(&["Ljava/lang/Object;"], "Z");
    for signature in [
        "<T:Ljava/lang/Object;>(TT;)Z",
        "(Ljava/util/List<TE;>;)Z",
        "(TE;)I",
        "(TE;TE;)Z",
        "(I)Z",
        "(T;)Z",
        "(T9bad;)Z",
        "(TE;)Z^TE;",
        "(Ljava/lang/Object;)Z",
    ] {
        assert!(!class_variable_signature(signature, &target), "{signature}");
    }
    assert!(!class_variable_signature("(TE;)Z", &method(&["I"], "Z")));
    assert!(!class_variable_signature(
        "([TE;)Z",
        &method(&["Ljava/lang/Object;"], "Z")
    ));
    assert!(!class_variable_signature(
        "(TE;)Z",
        &method(&["[Ljava/lang/Object;"], "Z")
    ));
}

#[test]
fn checked_parameterized_signatures_match_descriptors_and_class_bounds() {
    let class = "<E:Ljava/lang/Object;>Ljava/util/AbstractCollection<TE;>;Ljava/util/List<TE;>;";
    for (sig, args, ret) in [
        (
            "(ILjava/util/Collection<+TE;>;)Z",
            vec!["I", "Ljava/util/Collection;"],
            "Z",
        ),
        (
            "(Ljava/util/Collection<+TE;>;)Z",
            vec!["Ljava/util/Collection;"],
            "Z",
        ),
        ("([Ljava/util/List<*>;)V", vec!["[Ljava/util/List;"], "V"),
    ] {
        assert_eq!(
            signature::checked_method(sig, &method(&args, ret), Some(class)),
            Some(false)
        );
    }
    assert_eq!(
        signature::checked_method(
            "(Ljava/util/List<Landroid/graphics/Rect;>;)V",
            &method(&["Ljava/util/List;"], "V"),
            None
        ),
        Some(false)
    );
    assert_eq!(
        signature::checked_method(
            "(TE;)TE;",
            &method(&["Ljava/lang/Object;"], "Ljava/lang/Object;"),
            Some(class)
        ),
        Some(true)
    );
    for bad in [
        "(TU;)Z",
        "<M:Ljava/lang/Object;>(TM;)Z",
        "(Ljava/util/List<+I>;)V",
        "(Ljava/util/List<>;)V",
        "(Ljava/util/List<TE;>;)V^TE;",
    ] {
        assert!(
            signature::checked_method(bad, &method(&["Ljava/util/List;"], "V"), Some(class))
                .is_none()
        );
    }
    assert!(
        signature::checked_method("(TE;)Z", &method(&["Ljava/lang/Number;"], "Z"), Some(class))
            .is_none()
    );
    for bad in [
        "<E:>Ljava/lang/Object;",
        "<E:TU;>Ljava/lang/Object;",
        "<E:Ljava/lang/Comparable<TU;>;>Ljava/lang/Object;",
        "<E:I>Ljava/lang/Object;",
    ] {
        assert!(signature::class(bad).is_none(), "{bad}");
    }
}

#[test]
fn nested_generic_member_erasure_preserves_parameters_returns_and_scope() {
    let class = "<E:Ljava/lang/Object;F:Ljava/lang/Number;>Ljava/lang/Object;";
    let descriptor = "Lpkg/Outer$Inner;";
    let target = method(&[descriptor], descriptor);
    assert_eq!(
        signature::checked_method(
            "(Lpkg/Outer<TE;>.Inner<TF;>;)Lpkg/Outer<TE;>.Inner<TF;>;",
            &target,
            Some(class)
        ),
        Some(false)
    );
    for malformed in [
        "(Lpkg/Outer<TE;>.Inner<TU;>;)Lpkg/Outer<TE;>.Inner<TF;>;",
        "(Lpkg/Outer<TE;>.Inner<TF;>;)Lpkg/Outer<TE;>.Inner<TU;>;",
        "(Lpkg/Outer<TE;>..Inner<TF;>;)Lpkg/Outer$Inner;",
        "(Lpkg/Outer<TE;>/Inner<TF;>;)Lpkg/Outer$Inner;",
        "(Lpkg/Outer<TE;>.Inner<TF;>;)Lpkg/Outer$Inner;garbage",
        "(Lpkg/Outer<TE;>.Inner<TF;>;)Lpkg/Outer<TE;>.;",
    ] {
        assert!(
            signature::checked_method(malformed, &target, Some(class)).is_none(),
            "{malformed}"
        );
    }
    assert!(
        signature::checked_method(
            "(Lpkg/Outer<TE;>.Inner<TF;>;)Lpkg/Outer<TE;>.Inner<TF;>;",
            &method(&["Lpkg/Outer;"], descriptor),
            Some(class)
        )
        .is_none()
    );
}
