//! Dominator-tree renaming: operand renumbering and phi input completion.
//!
//! The walk is iterative (deep adversarial CFGs must not overflow the
//! stack). Register-variable stacks start empty, so a read with no
//! reaching definition falls back to a lazily created *entry initial*
//! definition hoisted to the entry chain — every reachable block is
//! dominated by entry, which keeps the use-dominates-def invariant even
//! for registers that are undefined on some paths. Ephemeral variables
//! (lifter-synthesized constants, predicates, initial reads, handler
//! inputs) are seeded with their single definition, which the placement
//! step locates at the entry chain head or the head of a using chain.

use std::collections::{HashMap, HashSet};

use decomp_core::analysis::dominators;
use decomp_core::cfg::Cfg;
use decomp_core::ids::BlockId;
use managed_ir::effects::EffectFlags;
use managed_ir::insn::ManagedInsn;
use managed_ir::method::{InsnArena, ValueArena};
use managed_ir::types::ManagedType;
use managed_ir::value::{Value, ValueId};

use super::chains::Chains;
use super::phis::Phis;
use super::uses::{for_each_definition, for_each_use_mut};
use super::vars::Variables;

/// Family tag for entry-initial definitions created by renaming.
pub(crate) const TAG_ENTRY_INITIAL: u16 = 0x8003;

/// Definitions created while renaming.
#[derive(Debug, Default)]
pub(crate) struct Renamed {
    /// One entry-initial value per variable that needed a fallback.
    pub entry_initials: HashMap<u32, ValueId>,
}

/// Rewrites every operand to its reaching definition and completes phi
/// inputs over the mirror graph.
pub(crate) fn rename(
    cfg: &Cfg<usize>,
    chains: &mut Chains,
    insns: &mut InsnArena,
    values: &mut ValueArena,
    vars: &Variables,
    phis: &mut Phis,
) -> Renamed {
    let doms = dominators(cfg);
    let mut children: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for block in cfg.block_ids() {
        if let Some(idom) = doms.immediate_dominator(block) {
            children.entry(idom).or_default().push(block);
        }
    }
    let mut stacks: Vec<Vec<ValueId>> = vec![Vec::new(); vars.count()];
    for (var, value) in vars.ephemeral_defs() {
        stacks[var as usize].push(value);
    }
    let mut renamed = Renamed::default();
    let mut visited: HashSet<BlockId> = HashSet::new();
    let mut pushed: Vec<Vec<u32>> = Vec::new();
    enum Step {
        Enter(BlockId),
        Exit,
    }
    let mut todo = vec![Step::Enter(cfg.entry())];
    while let Some(step) = todo.pop() {
        match step {
            Step::Exit => {
                for var in pushed.pop().unwrap_or_default() {
                    stacks[var as usize].pop();
                }
            }
            Step::Enter(block) => {
                if !visited.insert(block) {
                    continue;
                }
                let mut local: Vec<u32> = Vec::new();
                for phi in phis.at(block) {
                    stacks[phi.var as usize].push(phi.result);
                    local.push(phi.var);
                }
                let chain = cfg
                    .block(block)
                    .map(|record| *record.payload())
                    .unwrap_or(0);
                let payload = chains.payloads.get(chain).cloned().unwrap_or_default();
                for insn in payload {
                    let Some(original) = insns.get(insn) else {
                        continue;
                    };
                    let mut record = original.clone();
                    for_each_use_mut(&mut record, |operand| {
                        let var = vars.var_of(*operand);
                        *operand = match stacks[var as usize].last() {
                            Some(reaching) => *reaching,
                            None => ensure_initial(var, insns, values, &mut renamed.entry_initials),
                        };
                    });
                    for_each_definition(&record, |value| {
                        let var = vars.var_of(value);
                        stacks[var as usize].push(value);
                        local.push(var);
                    });
                    insns[insn] = record;
                }
                if let Ok(record) = cfg.block(block) {
                    for edge in record.outgoing() {
                        let Ok(target) = cfg.edge(*edge) else {
                            continue;
                        };
                        for phi in phis.at_mut(target.target()) {
                            let var = phi.var;
                            let value = match stacks[var as usize].last() {
                                Some(reaching) => *reaching,
                                None => {
                                    ensure_initial(var, insns, values, &mut renamed.entry_initials)
                                }
                            };
                            phi.fill(*edge, value);
                        }
                    }
                }
                pushed.push(local);
                todo.push(Step::Exit);
                for child in children.get(&block).into_iter().flatten().rev() {
                    todo.push(Step::Enter(*child));
                }
            }
        }
    }
    renamed
}

fn ensure_initial(
    var: u32,
    insns: &mut InsnArena,
    values: &mut ValueArena,
    initials: &mut HashMap<u32, ValueId>,
) -> ValueId {
    if let Some(value) = initials.get(&var) {
        return *value;
    }
    let insn = insns.insert(ManagedInsn::Return { value: None });
    let value = values.insert(Value::new(insn, ManagedType::Unknown));
    insns[insn] = ManagedInsn::Opaque {
        family_tag: TAG_ENTRY_INITIAL,
        reads: Vec::new(),
        writes: vec![value],
        effects: EffectFlags::NONE,
    };
    initials.insert(var, value);
    value
}
