//! Reference availability is independent of complete-body publication.
use crate::java_aliases::MethodKey;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Attested declaration availability, independent of caller semantics.
pub enum DeclarationStatus {
    /// Actual successful body publication.
    CompleteBody,
    /// Genuine abstract/native declaration; never counted as a complete body.
    BodylessDeclaration,
    /// Input declaration exists, but is outside the emitted source subset.
    SourceSubsetAbsent,
    /// Exact declaration is attested on the explicitly supplied classpath.
    ExternalClasspath,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(name: &str) -> MethodKey {
        MethodKey {
            owner: "Fixture".into(),
            name: name.into(),
            parameters: vec![],
            return_type: "V".into(),
        }
    }
    #[test]
    fn exact_body_identity_survives_unmet_declarations() {
        let caller = key("caller");
        let missing = key("missing");
        let native = key("nativeCall");
        let unknown = key("unknown");
        let mut publication = DependencyPublication::default();
        publication
            .declarations
            .insert(missing.clone(), DeclarationStatus::SourceSubsetAbsent);
        publication
            .declarations
            .insert(native.clone(), DeclarationStatus::BodylessDeclaration);
        let mut report = BodyPublicationReport::default();
        publication.record_emitted_body(
            &mut report,
            caller.clone(),
            [missing.clone(), native.clone(), unknown.clone()],
        );
        assert_eq!(report.complete_bodies, BTreeSet::from([caller.clone()]));
        assert_eq!(
            report.unresolved[&caller],
            vec![
                DependencyBlocker::SourceSubsetAbsent(missing.clone()),
                DependencyBlocker::UnknownDeclaration(unknown.clone())
            ]
        );
        assert!(!report.declaration_closed());
        assert_eq!(report.status_counts(), (1, 1, 2));
        assert_eq!(
            publication.references,
            BTreeSet::from([missing, native, unknown])
        );
        assert!(publication.body_closure.is_empty());
    }
    #[test]
    fn mutually_recursive_emitted_bodies_close_without_order_dependence() {
        let a = key("a");
        let b = key("b");
        let mut publication = DependencyPublication::default();
        for key in [&a, &b] {
            publication
                .declarations
                .insert(key.clone(), DeclarationStatus::CompleteBody);
        }
        let mut report = BodyPublicationReport::default();
        publication.record_emitted_body(&mut report, a.clone(), [b.clone()]);
        publication.record_emitted_body(&mut report, b.clone(), [a.clone()]);
        assert!(report.declaration_closed());
        assert_eq!(
            report.complete_bodies,
            BTreeSet::from([a.clone(), b.clone()])
        );
        assert_eq!(publication.body_closure, BTreeSet::from([a, b]));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Exact declaration required by a caller but unavailable in its output.
pub enum DependencyBlocker {
    /// No genuine declaration is known.
    UnknownDeclaration(MethodKey),
    /// Original declaration is known but not published.
    SourceSubsetAbsent(MethodKey),
}

#[derive(Default, Debug)]
/// Global publication evidence and retained reference edges.
pub struct DependencyPublication {
    /// Caller-attested declaration statuses.
    pub declarations: BTreeMap<MethodKey, DeclarationStatus>,
    /// Exact audited references.
    pub references: BTreeSet<MethodKey>,
    /// Referenced actual published bodies.
    pub body_closure: BTreeSet<MethodKey>,
    /// Exact per-caller edges retained for re-auditing after global publication.
    pub dependencies: BTreeMap<MethodKey, BTreeSet<MethodKey>>,
}
/// Body identity is recorded only after semantic emission succeeds. Dependency
/// failures are separate evidence and never erase that identity.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct BodyPublicationReport {
    /// Semantic body identities, preserved even when dependencies fail.
    pub complete_bodies: BTreeSet<MethodKey>,
    /// Direct unmet declarations for each semantic caller.
    pub unresolved: BTreeMap<MethodKey, Vec<DependencyBlocker>>,
}
impl BodyPublicationReport {
    /// (semantic bodies, callers with unresolved declarations, unmet edges).
    pub fn status_counts(&self) -> (usize, usize, usize) {
        (
            self.complete_bodies.len(),
            self.unresolved.values().filter(|v| !v.is_empty()).count(),
            self.unresolved.values().map(Vec::len).sum(),
        )
    }
    /// Whether every audited reference has a published declaration.
    pub fn declaration_closed(&self) -> bool {
        self.unresolved.values().all(Vec::is_empty)
    }
}
impl DependencyPublication {
    /// Record an actually emitted body, then independently audit its references.
    /// A closed subset is not evidence of whole-class completeness.
    pub fn record_emitted_body(
        &mut self,
        report: &mut BodyPublicationReport,
        caller: MethodKey,
        dependencies: impl IntoIterator<Item = MethodKey>,
    ) {
        report.complete_bodies.insert(caller.clone());
        let dependencies: BTreeSet<_> = dependencies.into_iter().collect();
        self.dependencies
            .insert(caller.clone(), dependencies.clone());
        let mut blockers = Vec::new();
        for dependency in dependencies {
            if let Err(blocker) = self.require_reference(&dependency)
                && !blockers.contains(&blocker)
            {
                blockers.push(blocker);
            }
        }
        report.unresolved.insert(caller, blockers);
    }
    /// Recompute evidence after declarations have been globally published. This
    /// removes order dependence without promoting any missing body to complete.
    pub fn refresh(&mut self, report: &mut BodyPublicationReport) {
        self.references.clear();
        self.body_closure.clear();
        report.unresolved.clear();
        for (caller, dependencies) in self.dependencies.clone() {
            if report.complete_bodies.contains(&caller) {
                self.record_emitted_body(report, caller, dependencies);
            }
        }
    }
    /// Caller supplies status only after actual publication/classpath validation.
    /// This API never creates stubs or promotes declarations to complete bodies.
    pub fn require_reference(&mut self, key: &MethodKey) -> Result<(), DependencyBlocker> {
        self.references.insert(key.clone());
        match self.declarations.get(key) {
            None => return Err(DependencyBlocker::UnknownDeclaration(key.clone())),
            Some(DeclarationStatus::SourceSubsetAbsent) => {
                return Err(DependencyBlocker::SourceSubsetAbsent(key.clone()));
            }
            Some(DeclarationStatus::CompleteBody) => {
                self.body_closure.insert(key.clone());
            }
            Some(DeclarationStatus::BodylessDeclaration | DeclarationStatus::ExternalClasspath) => {
            }
        }
        Ok(())
    }
}
