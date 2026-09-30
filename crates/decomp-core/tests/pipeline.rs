//! Pass registry dependency scheduling tests.

use std::fmt::Debug;

use decomp_core::pipeline::{Pass, PipelineError, Registry, Scope};

fn must<T, E: Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("unexpected error: {error:?}"),
    }
}

struct TestPass {
    id: &'static str,
    requires: &'static [&'static str],
    provides: &'static [&'static str],
    invalidates: &'static [&'static str],
}

impl Pass for TestPass {
    fn id(&self) -> &'static str {
        self.id
    }

    fn scope(&self) -> Scope {
        Scope::Method
    }

    fn requires(&self) -> &[&'static str] {
        self.requires
    }

    fn provides(&self) -> &[&'static str] {
        self.provides
    }

    fn invalidates(&self) -> &[&'static str] {
        self.invalidates
    }
}

#[test]
fn schedule_orders_providers_before_consumers() {
    let mut registry = Registry::new();
    must(registry.register(Box::new(TestPass {
        id: "consume",
        requires: &["cfg"],
        provides: &["structured"],
        invalidates: &[],
    })));
    must(registry.register(Box::new(TestPass {
        id: "build",
        requires: &[],
        provides: &["cfg"],
        invalidates: &["structured"],
    })));

    let schedule = must(registry.schedule());
    let ids: Vec<_> = schedule.iter().map(|pass| pass.id()).collect();

    assert_eq!(ids, vec!["build", "consume"]);
}

#[test]
fn schedule_reports_a_missing_required_invariant() {
    let mut registry = Registry::new();
    must(registry.register(Box::new(TestPass {
        id: "consume",
        requires: &["cfg"],
        provides: &[],
        invalidates: &[],
    })));

    let error = match registry.schedule() {
        Ok(_) => panic!("cfg has no provider"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        PipelineError::MissingProvider {
            pass: "consume",
            invariant: "cfg"
        }
    ));
}

#[test]
fn schedule_reports_dependency_cycles() {
    let mut registry = Registry::new();
    must(registry.register(Box::new(TestPass {
        id: "a",
        requires: &["b-ready"],
        provides: &["a-ready"],
        invalidates: &[],
    })));
    must(registry.register(Box::new(TestPass {
        id: "b",
        requires: &["a-ready"],
        provides: &["b-ready"],
        invalidates: &[],
    })));

    assert!(matches!(registry.schedule(), Err(PipelineError::Cycle)));
}

#[test]
fn registry_rejects_duplicate_pass_ids() {
    let mut registry = Registry::new();
    for provides in [&[][..], &["cfg"][..]] {
        let result = registry.register(Box::new(TestPass {
            id: "same",
            requires: &[],
            provides,
            invalidates: &[],
        }));
        if provides.is_empty() {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(PipelineError::DuplicatePass("same"))));
        }
    }
}
