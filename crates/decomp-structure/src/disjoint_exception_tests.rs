#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod disjoint_structure_tests {
    use super::super::*;
    #[test]
    fn disjoint_singleplacement_and_shared_handler_rejection() {
        let mut cfg = Cfg::new(());
        let first = cfg.entry();
        let second = cfg.add_block(());
        let end = cfg.add_block(());
        let catch1 = cfg.add_block(());
        let catch2 = cfg.add_block(());
        cfg.add_edge(first, second, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(second, end, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(first, catch1, EdgeKind::Exception("E".into()))
            .unwrap();
        cfg.add_edge(second, catch2, EdgeKind::Exception("E".into()))
            .unwrap();
        cfg.add_edge(catch1, second, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(catch2, end, EdgeKind::Unconditional).unwrap();
        let mut scopes = vec![
            RegionScope {
                protected: vec![first],
                catches: vec![Catch {
                    key: "E".into(),
                    entry: catch1,
                }],
                continuation: Some(second),
            },
            RegionScope {
                protected: vec![second],
                catches: vec![Catch {
                    key: "E".into(),
                    entry: catch2,
                }],
                continuation: Some(end),
            },
        ];
        let result = structure_disjoint(&cfg, &scopes).unwrap();
        assert_eq!(result.body.len(), 3);
        assert_eq!(result.scopes.len(), 2);
        scopes[1].catches[0].entry = catch1;
        assert!(structure_disjoint(&cfg, &scopes).is_err());
    }
}
