//! Focused descriptor-based incoming layout regression tests.
// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]

#[path = "../src/java_incoming_registers.rs"]
// This standalone descriptor suite does not exercise the DEX-pool adapter.
#[allow(dead_code)]
mod java_incoming_registers;

use java_incoming_registers::{IncomingLayout, IncomingLayoutError, IncomingSlot};

fn slot(register: u16, high_register: Option<u16>) -> IncomingSlot {
    IncomingSlot {
        register,
        high_register,
    }
}

#[test]
fn static_descriptor_order_and_wide_semantic_parameters() {
    let layout = IncomingLayout::from_descriptors(
        &["I", "J", "D", "[J", "[[D", "Ljava/lang/String;"],
        true,
        12,
        8,
    )
    .unwrap();
    assert_eq!(layout.incoming_start, 4);
    assert_eq!(layout.receiver, None);
    assert_eq!(
        layout.parameters,
        vec![
            slot(4, None),
            slot(5, Some(6)),
            slot(7, Some(8)),
            slot(9, None),
            slot(10, None),
            slot(11, None),
        ]
    );
}

#[test]
fn instance_receiver_precedes_wide_and_narrow_parameters() {
    let layout = IncomingLayout::from_descriptors(&["J", "I", "D"], false, 9, 6).unwrap();
    assert_eq!(layout.receiver, Some(slot(3, None)));
    assert_eq!(
        layout.parameters,
        vec![slot(4, Some(5)), slot(6, None), slot(7, Some(8))]
    );
}

#[test]
fn empty_static_and_receiver_only_instance() {
    let layout = IncomingLayout::from_descriptors(&[], true, 7, 0).unwrap();
    assert_eq!(layout.incoming_start, 7);
    assert!(layout.parameters.is_empty());
    assert_eq!(layout.receiver, None);
    let layout = IncomingLayout::from_descriptors(&[], false, 1, 1).unwrap();
    assert_eq!(layout.receiver, Some(slot(0, None)));
    assert!(layout.parameters.is_empty());
}

#[test]
fn mismatched_words_and_too_small_frame_are_rejected() {
    assert_eq!(
        IncomingLayout::from_descriptors(&["J"], false, 4, 2),
        Err(IncomingLayoutError::WordCountMismatch {
            expected: 3,
            actual: 2
        })
    );
    assert_eq!(
        IncomingLayout::from_descriptors(&["D"], true, 1, 2),
        Err(IncomingLayoutError::IncomingExceedsFrame {
            registers_size: 1,
            ins_size: 2
        })
    );
}

#[test]
fn maximum_frame_boundary_and_word_sum_overflow() {
    let layout = IncomingLayout::from_descriptors(&["J"], true, u16::MAX, 2).unwrap();
    assert_eq!(layout.parameters, vec![slot(65533, Some(65534))]);
    assert_eq!(
        IncomingLayout::from_descriptors(&vec!["J"; 32768], true, u16::MAX, 0),
        Err(IncomingLayoutError::WordCountOverflow)
    );
    assert_eq!(
        IncomingLayout::from_descriptors(&vec!["I"; 65535], false, u16::MAX, 0),
        Err(IncomingLayoutError::WordCountOverflow)
    );
}

#[test]
fn invalid_parameter_descriptors_do_not_silently_consume_words() {
    for descriptor in [
        "V",
        "[V",
        "",
        "L;",
        "Ljava.lang.String;",
        "L/foo;",
        "[",
        "II",
        "Jjunk",
    ] {
        assert!(matches!(
            IncomingLayout::from_descriptors(&[descriptor], true, 1, 1),
            Err(IncomingLayoutError::InvalidParameterDescriptor { index: 0, .. })
        ));
    }
    let too_deep = format!("{}I", "[".repeat(256));
    assert!(IncomingLayout::from_descriptors(&[&too_deep], true, 1, 1).is_err());
}
