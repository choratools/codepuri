//! Fusion retains source edge identity order, priority, and parallel transfers.
#![allow(clippy::unwrap_used)]
use decomp_core::cfg::EdgeKind;
use dex_decompiler::{DexPasses, merge_method};
use dex_format::{
    RawSpan,
    code::{CodeItem, EncodedCatchHandler, TryItem, TypeAddrPair},
};
fn code(units: &[u16]) -> CodeItem {
    CodeItem {
        registers_size: 1,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|v| v.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: RawSpan { offset: 0, size: 0 },
    }
}
fn rebuilt(body: &CodeItem) -> (String, Vec<EdgeKind>) {
    let lifted = DexPasses::run(body).unwrap();
    let merged = merge_method(lifted).unwrap();
    merged.cfg().validate().unwrap();
    let edges: Vec<_> = merged
        .cfg()
        .edge_ids()
        .map(|id| merged.cfg().edge(id).unwrap().kind().clone())
        .collect();
    // Source-ready canonical graph text includes every block payload and
    // ordered successor. It catches state-number scheduling drift before emit.
    let text = merged
        .cfg()
        .block_ids()
        .map(|id| {
            let block = merged.cfg().block(id).unwrap();
            let out: Vec<_> = block
                .outgoing()
                .iter()
                .map(|e| {
                    let edge = merged.cfg().edge(*e).unwrap();
                    format!("{:?}:{:?}", edge.kind(), edge.target())
                })
                .collect();
            format!("{id:?} {:?} {out:?}\n", block.payload())
        })
        .collect();
    (text, edges)
}
#[test]
fn parallel_switch_source_order_survives_repeated_rebuilding() {
    let body = code(&[0x002b, 4, 0, 0x000e, 0x0100, 3, 10, 0, 3, 0, 3, 0, 3, 0]);
    let original = DexPasses::run(&body).unwrap();
    let expected: Vec<_> = original
        .ir()
        .cfg()
        .edge_ids()
        .map(|e| original.ir().cfg().edge(e).unwrap().kind().clone())
        .collect();
    let (text, edges) = rebuilt(&body);
    assert_eq!(edges, expected);
    assert_eq!(
        edges
            .iter()
            .filter(|e| matches!(e, EdgeKind::Switch(_)))
            .count(),
        3
    );
    for _ in 0..32 {
        assert_eq!(rebuilt(&body), (text.clone(), edges.clone()));
    }
}
#[test]
fn typed_handler_priority_and_catchall_survive_rebuilding() {
    let mut body = code(&[
        0x0071, 0, 0, 0x000e, 0x000d, 0x000e, 0x000d, 0x000e, 0x000d, 0x000e,
    ]);
    let span = RawSpan { offset: 0, size: 0 };
    body.tries.push(TryItem {
        start_addr: 0,
        insn_count: 3,
        handler_off: 1,
        span,
    });
    body.handlers.push(EncodedCatchHandler {
        relative_offset: 1,
        handlers: vec![
            TypeAddrPair {
                type_idx: 7,
                address: 4,
            },
            TypeAddrPair {
                type_idx: 3,
                address: 6,
            },
        ],
        catch_all_addr: Some(8),
        span,
    });
    let original = DexPasses::run(&body).unwrap();
    let expected: Vec<_> = original
        .ir()
        .cfg()
        .edge_ids()
        .map(|e| original.ir().cfg().edge(e).unwrap().kind().clone())
        .collect();
    let offsets: std::collections::HashMap<_, _> = original
        .ir()
        .instructions()
        .keys()
        .map(|id| (id, original.offset(id)))
        .collect();
    let exact = merge_method(original).unwrap();
    let destinations: Vec<_> = exact
        .cfg()
        .edge_ids()
        .filter_map(|id| {
            let edge = exact.cfg().edge(id).unwrap();
            if let EdgeKind::Exception(kind) = edge.kind() {
                let first = exact.cfg().block(edge.target()).unwrap().payload()[0];
                Some((kind.as_str(), offsets[&first]))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        destinations,
        vec![("7", Some(4)), ("3", Some(6)), ("", Some(8))]
    );
    let (text, edges) = rebuilt(&body);
    // Ordinary one-successor handler seams are fused; exceptional edge
    // identities/order must survive without sorting by type or target.
    assert_eq!(
        edges
            .iter()
            .filter(|e| matches!(e, EdgeKind::Exception(_)))
            .collect::<Vec<_>>(),
        expected
            .iter()
            .filter(|e| matches!(e, EdgeKind::Exception(_)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        edges
            .iter()
            .filter(|e| matches!(e, EdgeKind::Exception(_)))
            .count(),
        3
    );
    for _ in 0..32 {
        assert_eq!(rebuilt(&body), (text.clone(), edges.clone()));
    }
}
