//! Edge-selected, effect-free monitor release/rethrow suffixes shared by catches.
use super::*;

fn candidate(
    ir: &MethodIr<SsaForm>,
    plan: &Plan,
    entry: BlockId,
    capture: ValueId,
) -> Result<Option<Vec<BlockId>>, Error> {
    let Some(owner) = plan.monitors.owners[&entry] else {
        return Ok(None);
    };
    let mut selected: HashSet<_> = ir
        .cfg()
        .block(entry)
        .map_err(invalid)?
        .incoming()
        .iter()
        .copied()
        .filter(|e| !plan.monitors.impossible.contains(e))
        .collect();
    let mut aliases = HashSet::from([capture]);
    let mut seen = HashSet::new();
    let mut path = vec![];
    let mut block = entry;
    let mut released = false;
    loop {
        if !seen.insert(block) || path.len() >= 64 {
            return Ok(None);
        }
        if plan.monitors.owners[&block] != if released { None } else { Some(owner) } {
            return Ok(None);
        }
        path.push(block);
        let id = *ir.cfg().block(block).map_err(invalid)?.payload();
        let insn = ir.instruction(id).map_err(invalid)?;
        let state = plan
            .regions
            .graph
            .states
            .iter()
            .find(|s| s.block == block)
            .ok_or(Error::Unsupported("shared monitor cleanup state absent"))?;
        match insn {
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads,
                writes,
                ..
            } if reads.is_empty()
                && writes == &[capture]
                && plan.capture_blocks.contains(&block) => {}
            ManagedInsn::Phi { result, inputs } => {
                let active: Vec<_> = inputs
                    .iter()
                    .filter(|(edge, _)| selected.contains(edge))
                    .collect();
                if active.len() == 1 && aliases.contains(&active[0].1) {
                    aliases.insert(*result);
                } else {
                    aliases.remove(result);
                }
            }
            ManagedInsn::Move { result, value } => {
                if aliases.contains(value) {
                    aliases.insert(*result);
                } else {
                    aliases.remove(result);
                }
            }
            ManagedInsn::Branch {
                condition: None,
                else_edge: None,
                ..
            } => {}
            ManagedInsn::MonitorExit { .. }
                if !released && plan.monitors.releases.contains_key(&block) =>
            {
                released = true;
            }
            ManagedInsn::Throw { value }
                if released
                    && aliases.contains(value)
                    && state.normal.is_empty()
                    && state
                        .exceptional
                        .iter()
                        .all(|e| plan.monitors.impossible.contains(e)) =>
            {
                return Ok(Some(path));
            }
            _ => return Ok(None),
        }
        if state
            .exceptional
            .iter()
            .any(|e| !plan.monitors.impossible.contains(e))
        {
            return Ok(None);
        }
        let [edge] = state.normal.as_slice() else {
            return Ok(None);
        };
        if matches!(insn, ManagedInsn::MonitorExit { .. })
            && plan.monitors.releases.get(&block) != Some(edge)
        {
            return Ok(None);
        }
        selected.insert(*edge);
        block = ir.cfg().edge(*edge).map_err(invalid)?.target();
    }
}

pub(super) fn prove(
    ir: &MethodIr<SsaForm>,
    plan: &Plan,
) -> Result<HashMap<BlockId, Vec<BlockId>>, Error> {
    let mut candidates = HashMap::new();
    for (&entry, &capture) in &plan.captures {
        if let Some(path) = candidate(ir, plan, entry, capture)? {
            candidates.insert(entry, path);
        }
    }
    loop {
        let union: HashSet<_> = candidates.values().flatten().copied().collect();
        let mut bad = vec![];
        for (&entry, path) in &candidates {
            for block in path {
                for edge in ir.cfg().block(*block).map_err(invalid)?.incoming() {
                    if plan.monitors.impossible.contains(edge) {
                        continue;
                    }
                    let record = ir.cfg().edge(*edge).map_err(invalid)?;
                    if !(union.contains(&record.source())
                        || *block == entry && matches!(record.kind(), EdgeKind::Exception(_)))
                    {
                        bad.push(entry);
                        break;
                    }
                }
            }
        }
        if bad.is_empty() {
            return Ok(candidates);
        }
        for entry in bad {
            candidates.remove(&entry);
        }
    }
}

#[cfg(test)]
#[path = "exception_dispatch_shared_monitor_cleanup_tests.rs"]
mod tests;
