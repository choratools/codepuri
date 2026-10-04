// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]

use super::*;

fn input(flags: u32, has_code: bool) -> MethodFlagInput {
    MethodFlagInput {
        access_flags: flags,
        owner: MethodOwner::Class { is_abstract: true },
        kind: MethodKind::Ordinary,
        has_code,
        last_parameter_is_array: true,
    }
}

#[test]
fn native_and_abstract_require_declarations_without_code() {
    for flags in [PUBLIC | NATIVE, PROTECTED | ABSTRACT] {
        assert_eq!(
            plan_method_declaration(input(flags, false)).unwrap().body,
            MethodBody::Forbidden
        );
        assert!(plan_method_declaration(input(flags, true)).is_err());
    }
    assert!(plan_method_declaration(input(PUBLIC, false)).is_err());
    assert_eq!(
        plan_method_declaration(input(PUBLIC, true)).unwrap().body,
        MethodBody::Required
    );
}

#[test]
fn synchronization_is_preserved_and_dex_bits_are_not_interchangeable() {
    for (flags, code, expected) in [
        (
            PUBLIC | NATIVE | SYNCHRONIZED,
            false,
            vec!["public", "synchronized", "native"],
        ),
        (
            PRIVATE | STATIC | DECLARED_SYNCHRONIZED,
            true,
            vec!["private", "static", "synchronized"],
        ),
    ] {
        let plan = plan_method_declaration(input(flags, code)).unwrap();
        assert_eq!(plan.modifiers, expected);
        assert!(plan.synchronized);
    }
    for flags in [
        SYNCHRONIZED,
        NATIVE | DECLARED_SYNCHRONIZED,
        NATIVE | SYNCHRONIZED | DECLARED_SYNCHRONIZED,
    ] {
        assert!(plan_method_declaration(input(flags, flags & NATIVE == 0)).is_err());
    }
}

#[test]
fn metadata_and_varargs_survive_without_fake_keywords() {
    let flags = PUBLIC | FINAL | BRIDGE | SYNTHETIC | VARARGS;
    let plan = plan_method_declaration(input(flags, true)).unwrap();
    assert_eq!(plan.modifiers, ["public", "final"]);
    assert!(plan.bridge && plan.synthetic && plan.varargs);
    let mut method = input(flags, true);
    method.last_parameter_is_array = false;
    assert!(plan_method_declaration(method).is_err());
    // Synthetic and bridge flags do not themselves imply a body or abstractness.
    assert!(plan_method_declaration(input(PUBLIC | ABSTRACT | BRIDGE | SYNTHETIC, false)).is_ok());
}

#[test]
fn contradictions_and_non_method_flags_are_rejected() {
    for extra in [
        PRIVATE,
        STATIC,
        FINAL,
        NATIVE,
        SYNCHRONIZED,
        STRICT,
        DECLARED_SYNCHRONIZED,
    ] {
        assert!(plan_method_declaration(input(ABSTRACT | extra, false)).is_err());
    }
    for flags in [
        PUBLIC | PRIVATE,
        PRIVATE | PROTECTED,
        PUBLIC | PROTECTED,
        NATIVE | STRICT,
        0x200,
        0x4000,
        0x8000,
        0x40000,
        CONSTRUCTOR,
    ] {
        assert!(plan_method_declaration(input(flags, true)).is_err());
    }
    let mut method = input(PUBLIC | ABSTRACT, false);
    method.owner = MethodOwner::Class { is_abstract: false };
    assert!(plan_method_declaration(method).is_err());
}

#[test]
fn interface_declarations_use_default_only_for_public_instance_bodies() {
    for (flags, code, expected) in [
        (PUBLIC | ABSTRACT, false, vec!["public", "abstract"]),
        (PUBLIC, true, vec!["public", "default"]),
        (PUBLIC | STATIC, true, vec!["public", "static"]),
        (PRIVATE, true, vec!["private"]),
        (PRIVATE | STATIC, true, vec!["private", "static"]),
    ] {
        let mut method = input(flags, code);
        method.owner = MethodOwner::Interface;
        assert_eq!(plan_method_declaration(method).unwrap().modifiers, expected);
    }
    for flags in [
        0,
        PROTECTED,
        PUBLIC | FINAL,
        PUBLIC | NATIVE,
        PUBLIC | DECLARED_SYNCHRONIZED,
        PRIVATE | ABSTRACT,
    ] {
        let mut method = input(flags, flags & (NATIVE | ABSTRACT) == 0);
        method.owner = MethodOwner::Interface;
        assert!(plan_method_declaration(method).is_err());
    }
}

#[test]
fn annotations_and_special_methods_are_separate() {
    let mut method = input(PUBLIC | ABSTRACT, false);
    method.owner = MethodOwner::Annotation;
    assert!(plan_method_declaration(method).is_ok());
    for flags in [
        PUBLIC,
        PUBLIC | STATIC,
        PUBLIC | ABSTRACT | VARARGS,
        PRIVATE | ABSTRACT,
    ] {
        method.access_flags = flags;
        method.has_code = flags & ABSTRACT == 0;
        assert!(plan_method_declaration(method).is_err());
    }
    for kind in [MethodKind::Constructor, MethodKind::ClassInitializer] {
        let mut special = input(PUBLIC | CONSTRUCTOR, true);
        special.kind = kind;
        assert_eq!(
            plan_method_declaration(special).unwrap_err().0,
            "constructor/class initializer requires separate lowering"
        );
    }
}
