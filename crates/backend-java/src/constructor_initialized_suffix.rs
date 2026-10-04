//! Loop-aware constructor suffix lowering after the receiver initializer.
//!
//! This helper is intentionally narrower than ordinary method control
//! lowering. It starts only after the original `this`/`super` chain completed,
//! keeps original block and edge identities, and emits no default-initialized
//! locals. Unsupported shapes fail transactionally so the parent constructor
//! path can preserve its existing proof boundary.

#[path = "constructor_loop_tail.rs"]
mod loop_tail;

use super::*;
use decomp_core::ids::{BlockId, EdgeId};

pub(super) fn emit_initialized_suffix<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    m: &JavaMethod,
    entry: BlockId,
    incoming: Option<EdgeId>,
    seen: &mut HashSet<BlockId>,
) -> Result<(), Unsupported> {
    let checkpoint = Checkpoint {
        values: e.values.clone(),
        lines: e.lines.len(),
        next: e.next,
        reserved: e.reserved.clone(),
        current: e.current,
        hoisted: e.hoisted.clone(),
        seen: seen.clone(),
    };
    match emit_initialized_suffix_inner(e, m, entry, incoming, seen) {
        Ok(()) => Ok(()),
        Err(error) => {
            e.values = checkpoint.values;
            e.lines.truncate(checkpoint.lines);
            e.next = checkpoint.next;
            e.reserved = checkpoint.reserved;
            e.current = checkpoint.current;
            e.hoisted = checkpoint.hoisted;
            *seen = checkpoint.seen;
            Err(error)
        }
    }
}

/// Acyclic shared tails after the exact original initializer. Already-emitted
/// prefix blocks contribute only empty forward labels; their operations are
/// never repeated. The prefix must be a single path ending at this entry.
pub(super) fn emit_shared_initialized_suffix<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    m: &JavaMethod,
    entry: BlockId,
    incoming: Option<EdgeId>,
    seen: &mut HashSet<BlockId>,
) -> Result<(), Unsupported> {
    let checkpoint = Checkpoint {
        values: e.values.clone(),
        lines: e.lines.len(),
        next: e.next,
        reserved: e.reserved.clone(),
        current: e.current,
        hoisted: e.hoisted.clone(),
        seen: seen.clone(),
    };
    let result = emit_shared_initialized_suffix_inner(e, m, entry, incoming, seen);
    if result.is_err() {
        e.values = checkpoint.values;
        e.lines.truncate(checkpoint.lines);
        e.next = checkpoint.next;
        e.reserved = checkpoint.reserved;
        e.current = checkpoint.current;
        e.hoisted = checkpoint.hoisted;
        *seen = checkpoint.seen;
    }
    result
}

fn emit_shared_initialized_suffix_inner<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    m: &JavaMethod,
    entry: BlockId,
    incoming: Option<EdgeId>,
    seen: &mut HashSet<BlockId>,
) -> Result<(), Unsupported> {
    let fail =
        |s: &str| Unsupported::ControlFlow(format!("unproved shared constructor suffix: {s}"));
    let cfg = e.ir.cfg();
    let decomp_structure::extended::FlowStructuredCfg::Shared(plan) =
        decomp_structure::extended::structure_flow(cfg).map_err(|err| fail(&err.to_string()))?
    else {
        return Err(fail("not shared"));
    };
    let reachable = reachable_suffix(e.ir, entry)?;
    if reachable.iter().any(|b| seen.contains(b)) {
        return Err(fail("previously emitted block"));
    }
    let mut prefix = HashSet::new();
    let mut cursor = cfg.entry();
    while cursor != entry {
        if !prefix.insert(cursor) || !seen.contains(&cursor) {
            return Err(fail("prefix identity"));
        }
        let node = cfg.block(cursor).map_err(|_| fail("prefix block"))?;
        if node.outgoing().len() != 1 {
            return Err(fail("prefix not linear"));
        }
        let edge = cfg
            .edge(node.outgoing()[0])
            .map_err(|_| fail("prefix edge"))?;
        if edge.kind() != &EdgeKind::Unconditional {
            return Err(fail("prefix exceptional"));
        }
        if edge.target() == entry && incoming != Some(node.outgoing()[0]) {
            return Err(fail("entry edge identity"));
        }
        cursor = edge.target();
    }
    if prefix != *seen || prefix.len() + reachable.len() != plan.blocks.len() {
        return Err(fail("block coverage"));
    }
    let mut lowering = InitializedSuffixLowering::new(e, m, reachable.clone())?;
    let seed = lowering.seed();
    let mut exits = HashMap::<EdgeId, HashSet<ValueId>>::new();
    let selected = incoming.ok_or_else(|| fail("entry incoming"))?;
    let mut initial = seed.clone();
    <InitializedSuffixLowering<'_, '_, R> as crate::loop_control::Lowering>::edge(
        &mut lowering,
        selected,
        &mut initial,
    )?;
    exits.insert(selected, initial);
    for block in &plan.blocks {
        if !reachable.contains(&block.block) {
            continue;
        }
        let node = lowering
            .e
            .ir
            .cfg()
            .block(block.block)
            .map_err(|_| fail("block"))?;
        let mut incoming_envs = node
            .incoming()
            .iter()
            .map(|edge| {
                exits
                    .get(edge)
                    .cloned()
                    .ok_or_else(|| fail("unproved predecessor availability"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut env = incoming_envs
            .pop()
            .ok_or_else(|| fail("missing predecessor"))?;
        for other in incoming_envs {
            env.retain(|value| other.contains(value));
        }
        <InitializedSuffixLowering<'_, '_, R> as crate::loop_control::Lowering>::block(
            &mut lowering,
            block.block,
            &mut env,
        )?;
        for edge in &block.outgoing {
            let mut output = env.clone();
            <InitializedSuffixLowering<'_, '_, R> as crate::loop_control::Lowering>::edge(
                &mut lowering,
                *edge,
                &mut output,
            )?;
            exits.insert(*edge, output);
        }
    }
    for value in lowering.locals()? {
        if lowering.e.values.contains_key(&value) {
            continue;
        }
        let ty = lowering.e.ty(value)?;
        let name = fresh_local(lowering.e);
        lowering.e.lines.push(format!("{} {name};", ty.source()?));
        lowering.e.values.insert(value, (name, ty));
        lowering.e.hoisted.insert(value);
    }
    let mut emitter = InitializedForward {
        inner: ForwardJava {
            emitter: lowering.e,
            method: m,
            copies: &lowering.copies,
        },
        prefix: &prefix,
        incoming: selected,
    };
    let source = crate::shared_regions::render(
        emitter.inner.emitter.ir.cfg(),
        &plan,
        "constructorForward",
        &mut emitter,
    )
    .map_err(|err| fail(&err))?;
    emitter
        .inner
        .emitter
        .lines
        .extend(source.lines().map(str::to_string));
    seen.extend(reachable);
    Ok(())
}

struct InitializedForward<'a, 'ir, R> {
    inner: ForwardJava<'a, 'ir, R>,
    prefix: &'a HashSet<BlockId>,
    incoming: EdgeId,
}
impl<R: JavaResolver> crate::shared_regions::ForwardEmitter for InitializedForward<'_, '_, R> {
    fn instructions(&mut self, b: BlockId) -> Result<String, String> {
        if self.prefix.contains(&b) {
            Ok(String::new())
        } else {
            crate::shared_regions::ForwardEmitter::instructions(&mut self.inner, b)
        }
    }
    fn terminal(&mut self, b: BlockId) -> Result<String, String> {
        crate::shared_regions::ForwardEmitter::terminal(&mut self.inner, b)
    }
    fn condition(&mut self, b: BlockId) -> Result<String, String> {
        crate::shared_regions::ForwardEmitter::condition(&mut self.inner, b)
    }
    fn selector(&mut self, b: BlockId) -> Result<String, String> {
        crate::shared_regions::ForwardEmitter::selector(&mut self.inner, b)
    }
    fn copies(&mut self, edge: EdgeId) -> Result<String, String> {
        let source = self
            .inner
            .emitter
            .ir
            .cfg()
            .edge(edge)
            .map_err(|e| e.to_string())?
            .source();
        if self.prefix.contains(&source) && edge != self.incoming {
            Ok(String::new())
        } else {
            crate::shared_regions::ForwardEmitter::copies(&mut self.inner, edge)
        }
    }
}

struct Checkpoint {
    values: HashMap<ValueId, (String, JavaType)>,
    lines: usize,
    next: usize,
    reserved: HashSet<String>,
    current: Option<InsnId>,
    hoisted: HashSet<ValueId>,
    seen: HashSet<BlockId>,
}

fn emit_initialized_suffix_inner<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    m: &JavaMethod,
    entry: BlockId,
    incoming: Option<EdgeId>,
    seen: &mut HashSet<BlockId>,
) -> Result<(), Unsupported> {
    let fail = |msg: &str| {
        Unsupported::ControlFlow(format!("unproved initialized constructor suffix: {msg}"))
    };
    e.ir.cfg()
        .validate()
        .map_err(|err| Unsupported::ControlFlow(err.to_string()))?;
    let reachable = reachable_suffix(e.ir, entry)?;
    if reachable.is_empty() || reachable.iter().any(|block| seen.contains(block)) {
        return Err(fail("overlap"));
    }
    let structured = terminal_tail_control(e.ir, &reachable)?;
    let region = suffix_region(&structured.root, entry).ok_or_else(|| fail("entry region"))?;
    let mut lowering = InitializedSuffixLowering::new(e, m, reachable.clone())?;
    let mut env = lowering.seed();
    let mut steps = Vec::new();
    if matches!(lowering.insn(entry)?.1, ManagedInsn::Phi { .. }) {
        let selected = incoming.ok_or_else(|| fail("entry Phi incoming edge"))?;
        if lowering
            .e
            .ir
            .cfg()
            .edge(selected)
            .map_err(|_| fail("incoming edge"))?
            .target()
            != entry
        {
            return Err(fail("entry Phi incoming target"));
        }
        steps.extend(
            <InitializedSuffixLowering<'_, '_, R> as crate::loop_control::Lowering>::edge(
                &mut lowering,
                selected,
                &mut env,
            )?
            .into_iter()
            .map(crate::loop_control::ControlStep::Ordinary),
        );
    }
    let (body, next) = crate::loop_control::lower(&region, &mut lowering, &mut env)?;
    steps.extend(body);
    if next {
        return Err(fail("fallthrough"));
    }
    let locals = lowering.locals()?;
    for value in locals {
        if lowering.e.values.contains_key(&value) {
            continue;
        }
        let ty = lowering.e.ty(value)?;
        let name = fresh_local(lowering.e);
        lowering.e.lines.push(format!("{} {name};", ty.source()?));
        lowering.e.values.insert(value, (name, ty));
        lowering.e.hoisted.insert(value);
    }
    render_suffix_control(lowering.e, &steps, m, 0)?;
    seen.extend(reachable);
    Ok(())
}

// Terminal-tail expansion can place one original allocation on exclusive paths.
// Materialization is path-local even though SSA identity and planning are shared.
fn render_suffix_control<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    steps: &[crate::loop_control::ControlStep],
    m: &JavaMethod,
    depth: usize,
) -> Result<(), Unsupported> {
    use crate::loop_control::ControlStep;
    for step in steps {
        let indent = "    ".repeat(depth);
        match step {
            ControlStep::If {
                condition,
                then_steps,
                else_steps,
            } => {
                // The allocation argument may have atomically emitted this
                // exact original acyclic subtree already. Reuse the existing
                // instruction/edge-identity proof, never just its condition.
                if let (Some(then_steps), Some(else_steps)) = (
                    suffix_ordinary_steps(then_steps),
                    suffix_ordinary_steps(else_steps),
                ) && e.folded_argument_region_step(&branching::Step::If {
                    condition: *condition,
                    then_steps,
                    else_steps,
                })? {
                    continue;
                }
                let (name, ty) = e.get(*condition)?;
                compatible(&ty, &JavaType::Boolean)?;
                // Only terminal, mutually exclusive paths have no escaping
                // normal-success state to merge. Joined branches retain the
                // existing renderer's materialization behavior.
                let checkpoint = (suffix_steps_terminate(e.ir, then_steps)
                    && suffix_steps_terminate(e.ir, else_steps))
                .then(|| (e.values.clone(), e.hoisted.clone()));
                e.lines.push(format!("{indent}if ({name}) {{"));
                render_suffix_control(e, then_steps, m, depth + 1)?;
                if let Some((values, hoisted)) = &checkpoint {
                    e.values = values.clone();
                    e.hoisted = hoisted.clone();
                }
                e.lines.push(format!("{indent}}} else {{"));
                render_suffix_control(e, else_steps, m, depth + 1)?;
                if let Some((values, hoisted)) = checkpoint {
                    e.values = values;
                    e.hoisted = hoisted;
                }
                e.lines.push(format!("{indent}}}"));
            }
            ControlStep::Loop { header, body } => {
                let label: String = format!("{header:?}")
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .collect();
                e.lines
                    .push(format!("{indent}loop{label}: while (true) {{"));
                render_suffix_control(e, body, m, depth + 1)?;
                e.lines.push(format!("{indent}}}"));
            }
            _ => crate::render_control(e, std::slice::from_ref(step), m, depth)?,
        }
    }
    Ok(())
}

// Exact Step equality includes original instruction IDs and Phi-copy edges.
// Loops/transfers cannot be an acyclic argument-region subtree.
fn suffix_ordinary_steps(
    steps: &[crate::loop_control::ControlStep],
) -> Option<Vec<branching::Step>> {
    use crate::loop_control::ControlStep;
    steps
        .iter()
        .map(|step| match step {
            ControlStep::Ordinary(step) => Some(step.clone()),
            ControlStep::If {
                condition,
                then_steps,
                else_steps,
            } => Some(branching::Step::If {
                condition: *condition,
                then_steps: suffix_ordinary_steps(then_steps)?,
                else_steps: suffix_ordinary_steps(else_steps)?,
            }),
            _ => None,
        })
        .collect()
}

// A return/throw from original IR ends the method, unlike loop transfers.
// No inference from conditions, allocation state or syntactic loop shape.
fn suffix_steps_terminate(
    ir: &MethodIr<SsaForm>,
    steps: &[crate::loop_control::ControlStep],
) -> bool {
    use crate::loop_control::ControlStep;
    if !suffix_transfers_internal(steps, &HashSet::new()) {
        return false;
    }
    match steps.last() {
        Some(ControlStep::Ordinary(branching::Step::Instruction(id))) => {
            ir.instruction(*id).is_ok_and(|insn| {
                matches!(insn, ManagedInsn::Return { .. } | ManagedInsn::Throw { .. })
            })
        }
        Some(ControlStep::If {
            then_steps,
            else_steps,
            ..
        }) => suffix_steps_terminate(ir, then_steps) && suffix_steps_terminate(ir, else_steps),
        _ => false,
    }
}

// A prefix transfer can bypass a later return; only loops lexically owned by
// this exclusive arm may receive transfers before its method termination.
fn suffix_transfers_internal(
    steps: &[crate::loop_control::ControlStep],
    owned: &HashSet<BlockId>,
) -> bool {
    use crate::loop_control::ControlStep;
    use crate::loop_control_regions::TransferKind;
    steps.iter().all(|step| match step {
        ControlStep::Transfer { kind, .. } => {
            let (TransferKind::Break { header } | TransferKind::Continue { header }) = kind;
            owned.contains(header)
        }
        ControlStep::If {
            then_steps,
            else_steps,
            ..
        } => {
            suffix_transfers_internal(then_steps, owned)
                && suffix_transfers_internal(else_steps, owned)
        }
        ControlStep::Loop { header, body } => {
            let mut nested = owned.clone();
            nested.insert(*header);
            suffix_transfers_internal(body, &nested)
        }
        ControlStep::Ordinary(_) => true,
    })
}

// Duplicate only finite unconditional terminal suffixes in a temporary topology.
// Mapping the result back preserves every original instruction and edge identity;
// syntactic copies occur on mutually exclusive paths ending in return/throw.
fn terminal_tail_control(
    ir: &MethodIr<SsaForm>,
    reachable: &HashSet<BlockId>,
) -> Result<crate::loop_control_regions::ControlCfg, Unsupported> {
    use crate::loop_control_regions::{ControlRegion, TransferKind};
    let original = ir.cfg();
    if let Ok(plan) = crate::loop_control_regions::structure(original) {
        return Ok(plan);
    }
    let fail = || Unsupported::ControlFlow("unproved constructor terminal-tail expansion".into());
    if original.block_count() > 512 {
        return Err(fail());
    }
    let mut graph = decomp_core::cfg::Cfg::new(original.entry());
    let mut blocks = HashMap::from([(original.entry(), graph.entry())]);
    let mut origins = HashMap::from([(graph.entry(), original.entry())]);
    for block in original.block_ids().filter(|b| *b != original.entry()) {
        let copied = graph.add_block(block);
        blocks.insert(block, copied);
        origins.insert(copied, block);
    }
    let mut edges = HashMap::new();
    let mut edge_origins = HashMap::new();
    for edge in original.edge_ids() {
        let item = original.edge(edge).map_err(|_| fail())?;
        let copied = graph
            .add_edge(
                blocks[&item.source()],
                blocks[&item.target()],
                item.kind().clone(),
            )
            .map_err(|_| fail())?;
        edges.insert(edge, copied);
        edge_origins.insert(copied, edge);
    }
    let mut copies = 0;
    for head in original.block_ids().filter(|b| reachable.contains(b)) {
        let node = original.block(head).map_err(|_| fail())?;
        if node.incoming().len() < 2 {
            continue;
        }
        let mut chain = Vec::new();
        let mut cursor = head;
        let mut visited = HashSet::new();
        let terminal = loop {
            if !reachable.contains(&cursor) || !visited.insert(cursor) || chain.len() >= 64 {
                break false;
            }
            let block = original.block(cursor).map_err(|_| fail())?;
            let insn = ir.instruction(*block.payload()).map_err(|_| fail())?;
            chain.push(cursor);
            match block.outgoing() {
                [] => {
                    break matches!(
                        insn,
                        ManagedInsn::Return { value: None } | ManagedInsn::Throw { .. }
                    );
                }
                [edge]
                    if original.edge(*edge).map_err(|_| fail())?.kind()
                        == &EdgeKind::Unconditional =>
                {
                    cursor = original.edge(*edge).map_err(|_| fail())?.target();
                }
                _ => break false,
            }
        };
        if !terminal {
            continue;
        }
        // Internal merges require their own proof, never clone an entry bypass.
        if chain
            .iter()
            .skip(1)
            .any(|b| original.block(*b).is_ok_and(|n| n.incoming().len() != 1))
        {
            continue;
        }
        for incoming in node.incoming().iter().skip(1) {
            copies += chain.len();
            if copies > 512 {
                return Err(fail());
            }
            let clones: Vec<_> = chain
                .iter()
                .map(|b| {
                    let c = graph.add_block(*b);
                    origins.insert(c, *b);
                    c
                })
                .collect();
            graph
                .redirect_edge(edges[incoming], clones[0])
                .map_err(|_| fail())?;
            for (index, source) in chain.iter().enumerate().take(chain.len() - 1) {
                let edge = original.block(*source).map_err(|_| fail())?.outgoing()[0];
                let copied = graph
                    .add_edge(clones[index], clones[index + 1], EdgeKind::Unconditional)
                    .map_err(|_| fail())?;
                edge_origins.insert(copied, edge);
            }
        }
    }
    let mut plan = match crate::loop_control_regions::structure(&graph) {
        Ok(plan) => plan,
        Err(_) => return loop_tail::structure(original, reachable),
    };
    fn restore(
        r: &mut ControlRegion,
        bs: &HashMap<BlockId, BlockId>,
        es: &HashMap<EdgeId, EdgeId>,
    ) {
        match r {
            ControlRegion::Block(b) => *b = bs[b],
            ControlRegion::Sequence(items) => {
                for item in items {
                    restore(item, bs, es);
                }
            }
            ControlRegion::If {
                header,
                then_edge,
                then_region,
                else_edge,
                else_region,
            } => {
                *header = bs[header];
                *then_edge = es[then_edge];
                *else_edge = es[else_edge];
                restore(then_region, bs, es);
                restore(else_region, bs, es);
            }
            ControlRegion::Loop { header, body, exit } => {
                *header = bs[header];
                *exit = exit.map(|b| bs[&b]);
                restore(body, bs, es);
            }
            ControlRegion::Transfer { edge, kind } => {
                *edge = es[edge];
                match kind {
                    TransferKind::Break { header } | TransferKind::Continue { header } => {
                        *header = bs[header]
                    }
                }
            }
        }
    }
    restore(&mut plan.root, &origins, &edge_origins);
    plan.entry = original.entry();
    plan.revision = original.revision();
    plan.edges = original
        .edge_ids()
        .map(|id| {
            let edge = original.edge(id).map_err(|_| fail())?;
            Ok(decomp_structure::EdgeProvenance {
                id,
                source: edge.source(),
                target: edge.target(),
                kind: edge.kind().clone(),
            })
        })
        .collect::<Result<_, Unsupported>>()?;
    Ok(plan)
}

fn reachable_suffix(
    ir: &MethodIr<SsaForm>,
    entry: BlockId,
) -> Result<HashSet<BlockId>, Unsupported> {
    let fail =
        || Unsupported::ControlFlow("unproved initialized constructor suffix reachability".into());
    let mut reachable = HashSet::new();
    let mut pending = vec![entry];
    while let Some(block) = pending.pop() {
        if !reachable.insert(block) {
            continue;
        }
        let node = ir.cfg().block(block).map_err(|_| fail())?;
        for edge in node.outgoing() {
            let edge = ir.cfg().edge(*edge).map_err(|_| fail())?;
            if matches!(edge.kind(), EdgeKind::Exception(_) | EdgeKind::Switch(_)) {
                return Err(fail());
            }
            pending.push(edge.target());
        }
    }
    Ok(reachable)
}

fn suffix_region(
    region: &crate::loop_control_regions::ControlRegion,
    entry: BlockId,
) -> Option<crate::loop_control_regions::ControlRegion> {
    use crate::loop_control_regions::ControlRegion;
    match region {
        ControlRegion::Block(block) => (*block == entry).then_some(region.clone()),
        ControlRegion::Loop { header, .. } if *header == entry => Some(region.clone()),
        ControlRegion::If { header, .. } if *header == entry => Some(region.clone()),
        ControlRegion::Sequence(items) => {
            for (index, item) in items.iter().enumerate() {
                if starts_at(item, entry) {
                    return Some(ControlRegion::Sequence(items[index..].to_vec()));
                }
                if let Some(found) = suffix_region(item, entry) {
                    return Some(found);
                }
            }
            None
        }
        ControlRegion::If {
            then_region,
            else_region,
            ..
        } => suffix_region(then_region, entry).or_else(|| suffix_region(else_region, entry)),
        ControlRegion::Loop { body, .. } => suffix_region(body, entry),
        ControlRegion::Transfer { .. } => None,
    }
}

fn starts_at(region: &crate::loop_control_regions::ControlRegion, entry: BlockId) -> bool {
    use crate::loop_control_regions::ControlRegion;
    match region {
        ControlRegion::Block(block) => *block == entry,
        ControlRegion::If { header, .. } | ControlRegion::Loop { header, .. } => *header == entry,
        ControlRegion::Sequence(items) => items.first().is_some_and(|item| starts_at(item, entry)),
        ControlRegion::Transfer { .. } => false,
    }
}

struct InitializedSuffixLowering<'a, 'ir, R> {
    e: &'a mut Emitter<'ir, R>,
    method: &'a JavaMethod,
    reachable: HashSet<BlockId>,
    loop_blocks: HashSet<BlockId>,
    copies: HashMap<EdgeId, Vec<branching::Copy>>,
    definitions: HashSet<ValueId>,
}

impl<'a, 'ir, R: JavaResolver> InitializedSuffixLowering<'a, 'ir, R> {
    fn new(
        e: &'a mut Emitter<'ir, R>,
        method: &'a JavaMethod,
        reachable: HashSet<BlockId>,
    ) -> Result<Self, Unsupported> {
        let fail = || Unsupported::ControlFlow("unproved initialized constructor suffix".into());
        let loop_blocks = loop_blocks(e.ir, &reachable);
        let mut this = Self {
            e,
            method,
            loop_blocks,
            reachable,
            copies: HashMap::new(),
            definitions: HashSet::new(),
        };
        this.validate_blocks()?;
        this.collect_phis()?;
        if this
            .definitions
            .iter()
            .any(|value| this.e.values.contains_key(value))
        {
            return Err(fail());
        }
        Ok(this)
    }

    fn seed(&self) -> HashSet<ValueId> {
        self.e.values.keys().copied().collect()
    }

    fn locals(&self) -> Result<Vec<ValueId>, Unsupported> {
        let mut locals = Vec::new();
        for (value_id, value) in self.e.ir.values() {
            let id = value.def();
            let block = self.e.ir.cfg().block_ids().find(|block| {
                self.reachable.contains(block)
                    && self
                        .e
                        .ir
                        .cfg()
                        .block(*block)
                        .is_ok_and(|node| *node.payload() == id)
            });
            if block.is_some() && self.definitions.contains(&value_id) {
                locals.push(value_id);
            }
        }
        Ok(locals)
    }

    fn insn(&self, block: BlockId) -> Result<(InsnId, &ManagedInsn), Unsupported> {
        if !self.reachable.contains(&block) {
            return Err(Unsupported::ControlFlow(
                "initialized constructor suffix escaped reachable set".into(),
            ));
        }
        let id = *self
            .e
            .ir
            .cfg()
            .block(block)
            .map_err(|err| Unsupported::ControlFlow(err.to_string()))?
            .payload();
        Ok((
            id,
            self.e
                .ir
                .instruction(id)
                .map_err(|err| Unsupported::Instruction(err.to_string()))?,
        ))
    }

    fn validate_blocks(&mut self) -> Result<(), Unsupported> {
        let mut placed = HashSet::new();
        for block in self.reachable.clone() {
            let (id, insn) = {
                let (id, insn) = self.insn(block)?;
                (id, insn.clone())
            };
            if !placed.insert(id) {
                return Err(Unsupported::ControlFlow(
                    "duplicate initialized suffix instruction placement".into(),
                ));
            }
            for value in defined(&insn) {
                if !self.definitions.insert(value)
                    || self
                        .e
                        .ir
                        .value(value)
                        .map_err(|err| Unsupported::Type(err.to_string()))?
                        .def()
                        != id
                {
                    return Err(Unsupported::Type(
                        "invalid initialized suffix definition identity".into(),
                    ));
                }
            }
            self.validate_block_shape(block, &insn)?;
            self.validate_instruction(block, &insn)?;
        }
        Ok(())
    }

    fn validate_block_shape(&self, block: BlockId, insn: &ManagedInsn) -> Result<(), Unsupported> {
        let fail = || Unsupported::ControlFlow("initialized constructor suffix edge shape".into());
        let node = self.e.ir.cfg().block(block).map_err(|_| fail())?;
        let outgoing = node.outgoing();
        match insn {
            ManagedInsn::Branch {
                condition: Some(condition),
                then_edge,
                else_edge: Some(else_edge),
            } => {
                if self.e.ir.value(*condition).map_err(|_| fail())?.ty() != &ManagedType::Boolean
                    || outgoing.len() != 2
                    || !outgoing.contains(then_edge)
                    || !outgoing.contains(else_edge)
                    || then_edge == else_edge
                    || self.e.ir.cfg().edge(*then_edge).map_err(|_| fail())?.kind()
                        != &EdgeKind::BranchTrue
                    || self.e.ir.cfg().edge(*else_edge).map_err(|_| fail())?.kind()
                        != &EdgeKind::BranchFalse
                {
                    return Err(fail());
                }
            }
            ManagedInsn::Branch {
                condition: None,
                then_edge,
                else_edge: None,
            } => {
                if outgoing != [*then_edge]
                    || self.e.ir.cfg().edge(*then_edge).map_err(|_| fail())?.kind()
                        != &EdgeKind::Unconditional
                {
                    return Err(fail());
                }
            }
            ManagedInsn::Return { .. } | ManagedInsn::Throw { .. } => {
                if !outgoing.is_empty() {
                    return Err(fail());
                }
            }
            ManagedInsn::Switch { .. } => return Err(fail()),
            _ => {
                if outgoing.len() != 1
                    || self
                        .e
                        .ir
                        .cfg()
                        .edge(outgoing[0])
                        .map_err(|_| fail())?
                        .kind()
                        != &EdgeKind::Unconditional
                {
                    return Err(fail());
                }
            }
        }
        for edge in outgoing {
            let edge = self.e.ir.cfg().edge(*edge).map_err(|_| fail())?;
            if matches!(edge.kind(), EdgeKind::Exception(_) | EdgeKind::Switch(_)) {
                return Err(fail());
            }
        }
        Ok(())
    }

    fn validate_instruction(&self, block: BlockId, insn: &ManagedInsn) -> Result<(), Unsupported> {
        match insn {
            ManagedInsn::MonitorEnter { .. } | ManagedInsn::MonitorExit { .. } => Err(
                Unsupported::ControlFlow("initialized constructor suffix monitor".into()),
            ),
            ManagedInsn::Invoke {
                kind: InvokeKind::Super,
                ..
            } => Err(Unsupported::ControlFlow(
                "second initialized constructor super call".into(),
            )),
            ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                args,
                method,
                ..
            } => {
                let declaration = self.e.resolver.method(*method).ok_or_else(|| {
                    Unsupported::Metadata("unresolved suffix special target".into())
                })?;
                // DEX invoke-direct also denotes ordinary private methods.
                // Their usual access/dispatch proof remains in the emitter;
                // only another <init> on this violates the initialized chain.
                if declaration.name == "<init>"
                    && args
                        .first()
                        .and_then(|receiver| self.e.values.get(receiver))
                        .is_some_and(|(name, ty)| {
                            name == "this" && *ty == JavaType::Class(self.method.owner.clone())
                        })
                {
                    return Err(Unsupported::ControlFlow(
                        "second initialized constructor this call".into(),
                    ));
                }
                Ok(())
            }
            ManagedInsn::FieldPut {
                object: Some(object),
                field,
                ..
            } => {
                let f = self
                    .e
                    .resolver
                    .field(*field)
                    .ok_or_else(|| Unsupported::Metadata("unresolved field".into()))?;
                let own_final = if f.owner == self.method.owner {
                    self.e.resolver.field_access(*field).ok_or_else(|| {
                        Unsupported::Metadata("unresolved constructor field access flags".into())
                    })? & 0x0010
                        != 0
                } else {
                    false
                };
                if own_final && self.loop_blocks.contains(&block) {
                    return Err(Unsupported::ControlFlow(
                        "initialized constructor final store in loop".into(),
                    ));
                }
                if own_final
                    && !self.e.values.get(object).is_some_and(|(name, ty)| {
                        name == "this" && *ty == JavaType::Class(self.method.owner.clone())
                    })
                {
                    return Err(Unsupported::ControlFlow(
                        "initialized constructor final store receiver".into(),
                    ));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn collect_phis(&mut self) -> Result<(), Unsupported> {
        let fail = || Unsupported::ControlFlow("initialized constructor suffix Phi".into());
        let cfg = self.e.ir.cfg();
        let mut processed = HashSet::new();
        for block in self.reachable.clone() {
            if processed.contains(&block) || !matches!(self.insn(block)?.1, ManagedInsn::Phi { .. })
            {
                continue;
            }
            let mut head = block;
            let mut walked = HashSet::new();
            loop {
                if !walked.insert(head) {
                    return Err(fail());
                }
                let incoming = cfg.block(head).map_err(|_| fail())?.incoming();
                if incoming.len() != 1 {
                    break;
                }
                let edge = cfg.edge(incoming[0]).map_err(|_| fail())?;
                if !self.reachable.contains(&edge.source()) {
                    break;
                }
                let source = cfg.block(edge.source()).map_err(|_| fail())?;
                if edge.kind() != &EdgeKind::Unconditional
                    || source.outgoing().len() != 1
                    || !matches!(self.insn(edge.source())?.1, ManagedInsn::Phi { .. })
                {
                    break;
                }
                head = edge.source();
            }
            let incoming = cfg.block(head).map_err(|_| fail())?.incoming();
            if incoming.is_empty()
                || incoming.iter().copied().collect::<HashSet<_>>().len() != incoming.len()
                || incoming.iter().any(|edge| match cfg.edge(*edge) {
                    Ok(edge) => matches!(edge.kind(), EdgeKind::Exception(_) | EdgeKind::Switch(_)),
                    Err(_) => true,
                })
            {
                return Err(fail());
            }
            let incoming_set: HashSet<_> = incoming.iter().copied().collect();
            let mut member = head;
            while let ManagedInsn::Phi { result, inputs } = self.insn(member)?.1.clone() {
                if !processed.insert(member) {
                    return Err(fail());
                }
                if inputs.len() != incoming.len()
                    || inputs.iter().map(|(edge, _)| *edge).collect::<HashSet<_>>() != incoming_set
                {
                    return Err(fail());
                }
                let result_ty = self.e.ty(result)?;
                let mut selected = HashSet::new();
                for (edge, source) in &inputs {
                    if !selected.insert(*edge)
                        || !self.phi_source_compatible(*source, &result_ty)?
                    {
                        return Err(fail());
                    }
                    self.copies.entry(*edge).or_default().push(branching::Copy {
                        destination: result,
                        source: *source,
                    });
                }
                let outgoing = cfg.block(member).map_err(|_| fail())?.outgoing();
                if outgoing.len() != 1 {
                    break;
                }
                let edge = cfg.edge(outgoing[0]).map_err(|_| fail())?;
                let target = cfg.block(edge.target()).map_err(|_| fail())?;
                if edge.kind() != &EdgeKind::Unconditional
                    || target.incoming().len() != 1
                    || !self.reachable.contains(&edge.target())
                    || !matches!(self.insn(edge.target())?.1, ManagedInsn::Phi { .. })
                {
                    break;
                }
                member = edge.target();
            }
        }
        Ok(())
    }

    fn phi_source_compatible(
        &self,
        source: ValueId,
        destination: &JavaType,
    ) -> Result<bool, Unsupported> {
        let source_ty = self.e.ty(source)?;
        if self.e.resolver.assignable(&source_ty, destination) {
            return Ok(true);
        }
        let managed = self
            .e
            .ir
            .value(source)
            .map_err(|err| Unsupported::Type(err.to_string()))?
            .ty();
        Ok(
            matches!(managed, ManagedType::Null) && destination.reference()
                || matches!(managed, ManagedType::Zero)
                    && matches!(
                        destination,
                        JavaType::Boolean
                            | JavaType::Byte
                            | JavaType::Short
                            | JavaType::Char
                            | JavaType::Int
                            | JavaType::Long
                            | JavaType::Float
                            | JavaType::Double
                    ),
        )
    }
}

fn loop_blocks(ir: &MethodIr<SsaForm>, reachable: &HashSet<BlockId>) -> HashSet<BlockId> {
    let dominators = decomp_core::analysis::dominators(ir.cfg());
    decomp_core::analysis::natural_loops(ir.cfg(), &dominators)
        .into_iter()
        .flat_map(|lp| lp.blocks().to_vec())
        .filter(|block| reachable.contains(block))
        .collect()
}

impl<R: JavaResolver> crate::loop_control::Lowering for InitializedSuffixLowering<'_, '_, R> {
    type Error = Unsupported;

    fn block(
        &mut self,
        block: BlockId,
        env: &mut HashSet<ValueId>,
    ) -> Result<(Vec<branching::Step>, bool), Unsupported> {
        let (id, insn) = self.insn(block)?;
        let mut steps = Vec::new();
        match insn {
            ManagedInsn::Phi { result, .. } => require(env, *result)?,
            ManagedInsn::Branch {
                condition: None, ..
            } => {}
            _ => {
                for value in reads(insn) {
                    require(env, value)?;
                }
                steps.push(branching::Step::Instruction(id));
                env.extend(defined(insn));
            }
        }
        Ok((
            steps,
            !matches!(insn, ManagedInsn::Return { .. } | ManagedInsn::Throw { .. }),
        ))
    }

    fn condition(
        &mut self,
        block: BlockId,
        env: &HashSet<ValueId>,
    ) -> Result<ValueId, Unsupported> {
        let (_, insn) = self.insn(block)?;
        let ManagedInsn::Branch {
            condition: Some(value),
            ..
        } = insn
        else {
            return Err(Unsupported::ControlFlow(
                "initialized constructor suffix condition".into(),
            ));
        };
        require(env, *value)?;
        Ok(*value)
    }

    fn edge(
        &mut self,
        edge: EdgeId,
        env: &mut HashSet<ValueId>,
    ) -> Result<Vec<branching::Step>, Unsupported> {
        let mut steps = Vec::new();
        if let Some(copies) = self.copies.get(&edge) {
            for copy in copies {
                require(env, copy.source)?;
            }
            for copy in copies {
                env.insert(copy.destination);
            }
            steps.push(branching::Step::Copies {
                edge,
                copies: copies.clone(),
            });
        }
        Ok(steps)
    }

    fn outgoing(&mut self, block: BlockId) -> Result<Option<EdgeId>, Unsupported> {
        let outgoing = self
            .e
            .ir
            .cfg()
            .block(block)
            .map_err(|err| Unsupported::ControlFlow(err.to_string()))?
            .outgoing();
        if outgoing.len() > 1 {
            return Err(Unsupported::ControlFlow(
                "initialized constructor suffix outgoing".into(),
            ));
        }
        Ok(outgoing.first().copied())
    }
}

fn require(env: &HashSet<ValueId>, value: ValueId) -> Result<(), Unsupported> {
    if env.contains(&value) {
        Ok(())
    } else {
        Err(Unsupported::Type(format!(
            "initialized constructor suffix unavailable {value:?}"
        )))
    }
}

fn fresh_local<R: JavaResolver>(e: &mut Emitter<'_, R>) -> String {
    loop {
        let name = format!("v{}", e.next);
        e.next += 1;
        if e.reserved.insert(name.clone()) {
            return name;
        }
    }
}

fn defined(insn: &ManagedInsn) -> Vec<ValueId> {
    match insn {
        ManagedInsn::Const { result, .. }
        | ManagedInsn::Move { result, .. }
        | ManagedInsn::Phi { result, .. }
        | ManagedInsn::Arithmetic { result, .. }
        | ManagedInsn::Compare { result, .. }
        | ManagedInsn::OrderingCompare { result, .. }
        | ManagedInsn::Neg { result, .. }
        | ManagedInsn::Not { result, .. }
        | ManagedInsn::FieldGet { result, .. }
        | ManagedInsn::ArrayGet { result, .. }
        | ManagedInsn::ArrayLength { result, .. }
        | ManagedInsn::NewInstance { result, .. }
        | ManagedInsn::NewArray { result, .. }
        | ManagedInsn::Cast { result, .. }
        | ManagedInsn::InstanceOf { result, .. } => vec![*result],
        ManagedInsn::Invoke { result, .. } | ManagedInsn::NewFilledArray { result, .. } => {
            result.iter().copied().collect()
        }
        ManagedInsn::Opaque { writes, .. } => writes.clone(),
        _ => Vec::new(),
    }
}

fn reads(insn: &ManagedInsn) -> Vec<ValueId> {
    match insn {
        ManagedInsn::Move { value, .. }
        | ManagedInsn::Neg { value, .. }
        | ManagedInsn::Not { value, .. }
        | ManagedInsn::Cast { value, .. }
        | ManagedInsn::InstanceOf { value, .. }
        | ManagedInsn::Throw { value } => vec![*value],
        ManagedInsn::Arithmetic { lhs, rhs, .. }
        | ManagedInsn::Compare { lhs, rhs, .. }
        | ManagedInsn::OrderingCompare { lhs, rhs, .. } => vec![*lhs, *rhs],
        ManagedInsn::FieldGet { object, .. } => object.iter().copied().collect(),
        ManagedInsn::FieldPut { object, value, .. } => {
            object.iter().copied().chain([*value]).collect()
        }
        ManagedInsn::ArrayGet { array, index, .. } => vec![*array, *index],
        ManagedInsn::ArrayPut {
            array,
            index,
            value,
        } => vec![*array, *index, *value],
        ManagedInsn::ArrayLength { array, .. } => vec![*array],
        ManagedInsn::NewArray { length, .. } => vec![*length],
        ManagedInsn::NewFilledArray { elements, .. } => elements.clone(),
        ManagedInsn::FillArray { array, values } => std::iter::once(*array)
            .chain(values.iter().copied())
            .collect(),
        ManagedInsn::Invoke { args, .. } => args.clone(),
        ManagedInsn::MonitorEnter { object } | ManagedInsn::MonitorExit { object } => vec![*object],
        ManagedInsn::Branch { condition, .. } => condition.iter().copied().collect(),
        ManagedInsn::Switch { key, .. } => vec![*key],
        ManagedInsn::Return { value } => value.iter().copied().collect(),
        ManagedInsn::Opaque { reads, .. } => reads.clone(),
        ManagedInsn::Const { .. } | ManagedInsn::Phi { .. } | ManagedInsn::NewInstance { .. } => {
            Vec::new()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    include!("constructor_mutable_suffix_tests.rs");
    include!("constructor_exclusive_allocation_tests.rs");
    use super::*;
    use decomp_core::cfg::Cfg;
    use managed_ir::{
        effects::EffectFlags,
        insn::Constant,
        method::{InsnArena, ValueArena},
        ops::{ArithOp, CompareOp},
        refs::{ClassRef, FieldRef, StringRef},
        types::ManagedType,
        value::Value,
    };

    struct R;

    impl JavaResolver for R {
        fn field_access(&self, _: FieldRef) -> Option<u32> {
            Some(0x0011)
        }
        fn class(&self, c: ClassRef) -> Option<String> {
            Some(
                match c.index() {
                    0 => "Fixture",
                    _ => "java.lang.Object",
                }
                .into(),
            )
        }

        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }

        fn field(&self, _: FieldRef) -> Option<JavaField> {
            Some(JavaField {
                owner: "Fixture".into(),
                name: "value".into(),
                ty: JavaType::Int,
                is_static: false,
            })
        }

        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            Some(JavaMethod {
                owner: "Fixture".into(),
                name: "<init>".into(),
                parameters: vec![],
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            })
        }
    }

    struct Fixture {
        ir: MethodIr<SsaForm>,
        this: ValueId,
        start: ValueId,
        limit: ValueId,
        entry: BlockId,
        incoming: EdgeId,
    }

    fn def(
        insns: &mut InsnArena,
        values: &mut ValueArena,
        ty: ManagedType,
        f: impl FnOnce(ValueId) -> ManagedInsn,
    ) -> (InsnId, ValueId) {
        let mut value = None;
        let id = insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ty));
            value = Some(v);
            f(v)
        });
        (id, value.unwrap())
    }

    fn input(insns: &mut InsnArena, values: &mut ValueArena, ty: ManagedType) -> ValueId {
        def(insns, values, ty, |v| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        })
        .1
    }

    fn fixture(mode: u8) -> Fixture {
        let mut insns = InsnArena::default();
        let mut values = ValueArena::default();
        let this = input(
            &mut insns,
            &mut values,
            ManagedType::Reference(ClassRef::new(0)),
        );
        let start = input(&mut insns, &mut values, ManagedType::Int);
        let limit = input(&mut insns, &mut values, ManagedType::Int);
        let chain = insns.insert(ManagedInsn::Return { value: None });
        let (phi, index) = def(&mut insns, &mut values, ManagedType::Int, |result| {
            ManagedInsn::Phi {
                result,
                inputs: vec![],
            }
        });
        let (cmp, keep_going) = def(&mut insns, &mut values, ManagedType::Boolean, |result| {
            ManagedInsn::Compare {
                op: CompareOp::LessThan,
                result,
                lhs: index,
                rhs: limit,
            }
        });
        let branch = insns.insert(ManagedInsn::Return { value: None });
        let (one, one_value) = def(&mut insns, &mut values, ManagedType::Int, |result| {
            ManagedInsn::Const {
                result,
                value: Constant::Int(1),
            }
        });
        let loop_store = insns.insert(ManagedInsn::FieldPut {
            object: Some(this),
            field: FieldRef::new(u32::from(mode == 5)),
            value: index,
        });
        let (add, next) = def(&mut insns, &mut values, ManagedType::Int, |result| {
            ManagedInsn::Arithmetic {
                op: ArithOp::Add,
                result,
                lhs: index,
                rhs: one_value,
            }
        });
        let second_chain = insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![this],
        });
        let jump = insns.insert(ManagedInsn::Return { value: None });
        let store = insns.insert(ManagedInsn::FieldPut {
            object: Some(this),
            field: FieldRef::new(0),
            value: index,
        });
        let ret = insns.insert(ManagedInsn::Return { value: None });
        let mut cfg = Cfg::new(chain);
        let chain_block = cfg.entry();
        let phi_block = cfg.add_block(phi);
        let cmp_block = cfg.add_block(cmp);
        let branch_block = cfg.add_block(branch);
        let one_block = cfg.add_block(one);
        let add_block = cfg.add_block(add);
        let jump_block = cfg.add_block(jump);
        let store_block = cfg.add_block(store);
        let ret_block = cfg.add_block(ret);
        let incoming = cfg
            .add_edge(chain_block, phi_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(phi_block, cmp_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(cmp_block, branch_block, EdgeKind::Unconditional)
            .unwrap();
        let body_edge = cfg
            .add_edge(branch_block, one_block, EdgeKind::BranchTrue)
            .unwrap();
        let exit_edge = cfg
            .add_edge(branch_block, store_block, EdgeKind::BranchFalse)
            .unwrap();
        let after_one = if matches!(mode, 4 | 5) {
            let loop_store_block = cfg.add_block(loop_store);
            cfg.add_edge(one_block, loop_store_block, EdgeKind::Unconditional)
                .unwrap();
            cfg.add_edge(loop_store_block, add_block, EdgeKind::Unconditional)
                .unwrap();
            loop_store_block
        } else {
            cfg.add_edge(one_block, add_block, EdgeKind::Unconditional)
                .unwrap();
            one_block
        };
        let after_add = if mode == 3 {
            let second_chain_block = cfg.add_block(second_chain);
            cfg.add_edge(add_block, second_chain_block, EdgeKind::Unconditional)
                .unwrap();
            cfg.add_edge(second_chain_block, jump_block, EdgeKind::Unconditional)
                .unwrap();
            second_chain_block
        } else {
            cfg.add_edge(add_block, jump_block, EdgeKind::Unconditional)
                .unwrap();
            add_block
        };
        let back = cfg
            .add_edge(jump_block, phi_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(store_block, ret_block, EdgeKind::Unconditional)
            .unwrap();
        if mode == 2 {
            cfg.add_edge(
                after_one,
                ret_block,
                EdgeKind::Exception("java.lang.Exception".into()),
            )
            .unwrap();
        }
        let _ = after_add;
        *insns.get_mut(chain).unwrap() = ManagedInsn::Branch {
            condition: None,
            then_edge: incoming,
            else_edge: None,
        };
        *insns.get_mut(phi).unwrap() = ManagedInsn::Phi {
            result: index,
            inputs: vec![(incoming, start), (back, next)],
        };
        *insns.get_mut(branch).unwrap() = ManagedInsn::Branch {
            condition: Some(keep_going),
            then_edge: body_edge,
            else_edge: Some(exit_edge),
        };
        *insns.get_mut(jump).unwrap() = ManagedInsn::Branch {
            condition: None,
            then_edge: back,
            else_edge: None,
        };
        Fixture {
            ir: MethodIr::<SsaForm>::from_parts(cfg, insns, values),
            this,
            start,
            limit,
            entry: phi_block,
            incoming,
        }
    }

    fn emitter<'a, Q: JavaResolver>(f: &'a Fixture, resolver: &'a Q) -> Emitter<'a, Q> {
        let mut values = HashMap::new();
        values.insert(f.this, ("this".into(), JavaType::Class("Fixture".into())));
        values.insert(f.start, ("p0".into(), JavaType::Int));
        values.insert(f.limit, ("p1".into(), JavaType::Int));
        Emitter {
            allocation_readable: None,
            object_planning: Default::default(),
            ir: &f.ir,
            resolver,
            values,
            lines: Vec::new(),
            next: 0,
            parameters: HashSet::from([f.this, f.start, f.limit]),
            reserved: HashSet::from([
                "java".to_string(),
                "this".to_string(),
                "p0".to_string(),
                "p1".to_string(),
            ]),
            current: None,
            hoisted: HashSet::new(),
            declarations: HashMap::new(),
        }
    }

    fn method() -> JavaMethod {
        JavaMethod {
            owner: "Fixture".into(),
            name: "<init>".into(),
            parameters: vec![JavaType::Int, JavaType::Int],
            return_type: JavaType::Void,
            is_static: false,
            throws: vec![],
        }
    }

    #[test]
    fn initialized_entry_phi_loop_suffix_emits_once_and_keeps_final_store_after_loop() {
        let f = fixture(0);
        let resolver = R;
        let mut e = emitter(&f, &resolver);
        let mut seen = HashSet::from([f.ir.cfg().entry()]);
        emit_initialized_suffix(&mut e, &method(), f.entry, Some(f.incoming), &mut seen).unwrap();
        let source = e.lines.join("\n");
        assert!(source.contains("while (true)"));
        assert_eq!(source.matches("this.value =").count(), 1);
        assert!(source.find("while (true)").unwrap() < source.find("this.value =").unwrap());
        assert!(seen.contains(&f.entry));
    }

    #[test]
    fn entry_phi_requires_selected_original_incoming_edge() {
        let f = fixture(0);
        let resolver = R;
        let mut e = emitter(&f, &resolver);
        let before = e.values.clone();
        let mut seen = HashSet::from([f.ir.cfg().entry()]);
        assert!(emit_initialized_suffix(&mut e, &method(), f.entry, None, &mut seen).is_err());
        assert_eq!(e.values, before);
        assert_eq!(e.lines.len(), 0);
    }

    #[test]
    fn exception_second_chain_and_loop_final_store_reject() {
        for mode in [2, 3, 4] {
            let f = fixture(mode);
            let resolver = R;
            let mut e = emitter(&f, &resolver);
            let mut seen = HashSet::from([f.ir.cfg().entry()]);
            assert!(
                emit_initialized_suffix(&mut e, &method(), f.entry, Some(f.incoming), &mut seen)
                    .is_err(),
                "accepted mode {mode}"
            );
        }
    }
    fn shared_fixture() -> Fixture {
        let mut insns = InsnArena::default();
        let mut values = ValueArena::default();
        let this = input(
            &mut insns,
            &mut values,
            ManagedType::Reference(ClassRef::new(0)),
        );
        let start = input(&mut insns, &mut values, ManagedType::Int);
        let limit = input(&mut insns, &mut values, ManagedType::Int);
        let chain = insns.insert(ManagedInsn::Return { value: None });
        let (cmp, condition) = def(&mut insns, &mut values, ManagedType::Boolean, |result| {
            ManagedInsn::Compare {
                op: CompareOp::LessThan,
                result,
                lhs: start,
                rhs: limit,
            }
        });
        let branch = insns.insert(ManagedInsn::Return { value: None });
        let (zero, zero_value) = def(&mut insns, &mut values, ManagedType::Int, |result| {
            ManagedInsn::Const {
                result,
                value: Constant::Int(0),
            }
        });
        let (cmp2, condition2) = def(&mut insns, &mut values, ManagedType::Boolean, |result| {
            ManagedInsn::Compare {
                op: CompareOp::Equal,
                result,
                lhs: start,
                rhs: zero_value,
            }
        });
        let second = insns.insert(ManagedInsn::Return { value: None });
        let store = insns.insert(ManagedInsn::FieldPut {
            object: Some(this),
            field: FieldRef::new(0),
            value: start,
        });
        let ret = insns.insert(ManagedInsn::Return { value: None });
        let mut cfg = Cfg::new(chain);
        let entry = cfg.add_block(cmp);
        let b = cfg.add_block(branch);
        let z = cfg.add_block(zero);
        let c2 = cfg.add_block(cmp2);
        let c = cfg.add_block(second);
        let d = cfg.add_block(store);
        let end = cfg.add_block(ret);
        let incoming = cfg
            .add_edge(cfg.entry(), entry, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(entry, b, EdgeKind::Unconditional).unwrap();
        let yes = cfg.add_edge(b, z, EdgeKind::BranchTrue).unwrap();
        cfg.add_edge(z, c2, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(c2, c, EdgeKind::Unconditional).unwrap();
        let no = cfg.add_edge(b, d, EdgeKind::BranchFalse).unwrap();
        let yes2 = cfg.add_edge(c, d, EdgeKind::BranchTrue).unwrap();
        let no2 = cfg.add_edge(c, end, EdgeKind::BranchFalse).unwrap();
        cfg.add_edge(d, end, EdgeKind::Unconditional).unwrap();
        *insns.get_mut(chain).unwrap() = ManagedInsn::Branch {
            condition: None,
            then_edge: incoming,
            else_edge: None,
        };
        *insns.get_mut(branch).unwrap() = ManagedInsn::Branch {
            condition: Some(condition),
            then_edge: yes,
            else_edge: Some(no),
        };
        *insns.get_mut(second).unwrap() = ManagedInsn::Branch {
            condition: Some(condition2),
            then_edge: yes2,
            else_edge: Some(no2),
        };
        Fixture {
            ir: MethodIr::<SsaForm>::from_parts(cfg, insns, values),
            this,
            start,
            limit,
            entry,
            incoming,
        }
    }

    #[test]
    fn shared_suffix_preserves_single_store_and_transactional_prefix_gate() {
        let f = shared_fixture();
        let r = R;
        let mut e = emitter(&f, &r);
        let mut seen = HashSet::from([f.ir.cfg().entry()]);
        emit_shared_initialized_suffix(&mut e, &method(), f.entry, Some(f.incoming), &mut seen)
            .unwrap();
        let source = e.lines.join("\n");
        assert_eq!(source.matches("this.value =").count(), 1);
        assert!(source.contains("break constructorForward_"));
        assert_eq!(seen.len(), f.ir.cfg().block_ids().count());
        let mut bad = emitter(&f, &r);
        let values = bad.values.clone();
        let mut absent = HashSet::new();
        assert!(
            emit_shared_initialized_suffix(
                &mut bad,
                &method(),
                f.entry,
                Some(f.incoming),
                &mut absent
            )
            .is_err()
        );
        assert_eq!(bad.values, values);
        assert!(bad.lines.is_empty());
        assert!(absent.is_empty());
    }

    #[test]
    fn shared_constructor_suffix_jdk_execution_preserves_early_return() {
        let f = shared_fixture();
        let r = R;
        let mut e = emitter(&f, &r);
        let mut seen = HashSet::from([f.ir.cfg().entry()]);
        emit_shared_initialized_suffix(&mut e, &method(), f.entry, Some(f.incoming), &mut seen)
            .unwrap();
        let body = e.lines.join("\n");
        let dir = std::env::temp_dir().join(format!(
            "decomphify-shared-constructor-wave17-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = format!(
            "public class Fixture {{ int value; Fixture(int p0,int p1){{ super(); {body} }} public static void main(String[]args){{ for(int a=-3;a<=3;a++)for(int b=-3;b<=3;b++){{ int expected=(a<b && a!=0)?0:a; if(new Fixture(a,b).value!=expected)throw new AssertionError(a+\":\"+b); }} }} }}"
        );
        std::fs::write(dir.join("Fixture.java"), source).unwrap();
        let java_home = std::env::var_os("JAVA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11"));
        let compile = std::process::Command::new(java_home.join("bin/javac"))
            .arg("--release")
            .arg("17")
            .arg(dir.join("Fixture.java"))
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let runtime = std::process::Command::new(java_home.join("bin/java"))
            .arg("-cp")
            .arg(&dir)
            .arg("Fixture")
            .output()
            .unwrap();
        assert!(
            runtime.status.success(),
            "{}",
            String::from_utf8_lossy(&runtime.stderr)
        );
    }
    struct PrivateR;
    impl JavaResolver for PrivateR {
        fn field_access(&self, f: FieldRef) -> Option<u32> {
            R.field_access(f)
        }
        fn class(&self, c: ClassRef) -> Option<String> {
            R.class(c)
        }
        fn string(&self, s: StringRef) -> Option<Vec<u16>> {
            R.string(s)
        }
        fn field(&self, f: FieldRef) -> Option<JavaField> {
            R.field(f)
        }
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            Some(JavaMethod {
                owner: "Fixture".into(),
                name: "touch".into(),
                parameters: vec![],
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            })
        }
        fn method_access(&self, _: MethodRef) -> Option<u32> {
            Some(2)
        }
    }
    #[test]
    fn initialized_private_direct_call_is_not_second_constructor() {
        let f = fixture(3);
        let base = R;
        let seed = emitter(&f, &base);
        let resolver = PrivateR;
        let mut e = Emitter {
            allocation_readable: None,
            object_planning: Default::default(),
            ir: &f.ir,
            resolver: &resolver,
            values: seed.values,
            lines: seed.lines,
            next: seed.next,
            parameters: seed.parameters,
            reserved: seed.reserved,
            current: seed.current,
            hoisted: seed.hoisted,
            declarations: seed.declarations,
        };
        let mut seen = HashSet::from([f.ir.cfg().entry()]);
        emit_initialized_suffix(&mut e, &method(), f.entry, Some(f.incoming), &mut seen).unwrap();
        assert_eq!(e.lines.join("\n").matches("this.touch(").count(), 1);
    }
    #[test]
    fn terminal_clone_control_retains_original_edges_and_lazy_guards() {
        let mut insns = InsnArena::default();
        let first = insns.insert(ManagedInsn::Return { value: None });
        let mut cfg = Cfg::new(first);
        let entry = cfg.entry();
        let mut node = || insns.insert(ManagedInsn::Return { value: None });
        let second = cfg.add_block(node());
        let header = cfg.add_block(node());
        let body = cfg.add_block(node());
        let tail = cfg.add_block(node());
        let end = cfg.add_block(node());
        let tail_return = cfg.add_block(node());
        cfg.add_edge(entry, tail, EdgeKind::BranchTrue).unwrap();
        cfg.add_edge(entry, second, EdgeKind::BranchFalse).unwrap();
        cfg.add_edge(second, tail, EdgeKind::BranchTrue).unwrap();
        cfg.add_edge(second, header, EdgeKind::BranchFalse).unwrap();
        cfg.add_edge(header, body, EdgeKind::BranchTrue).unwrap();
        cfg.add_edge(header, end, EdgeKind::BranchFalse).unwrap();
        cfg.add_edge(body, header, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(tail, tail_return, EdgeKind::Unconditional)
            .unwrap();
        let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, ValueArena::default());
        let reachable = reachable_suffix(&ir, entry).unwrap();
        assert!(crate::loop_control_regions::structure(ir.cfg()).is_err());
        let plan = terminal_tail_control(&ir, &reachable).unwrap();
        assert_eq!(plan.edges.len(), ir.cfg().edge_count());
        for item in &plan.edges {
            let original = ir.cfg().edge(item.id).unwrap();
            assert_eq!(
                (item.source, item.target, &item.kind),
                (original.source(), original.target(), original.kind())
            );
        }
        fn count(region: &crate::loop_control_regions::ControlRegion, block: BlockId) -> usize {
            use crate::loop_control_regions::ControlRegion;
            match region {
                ControlRegion::Block(b) => usize::from(*b == block),
                ControlRegion::Sequence(items) => items.iter().map(|i| count(i, block)).sum(),
                ControlRegion::If {
                    then_region,
                    else_region,
                    ..
                } => count(then_region, block) + count(else_region, block),
                ControlRegion::Loop { body, .. } => count(body, block),
                ControlRegion::Transfer { .. } => 0,
            }
        }
        assert_eq!(count(&plan.root, tail), 2);
        assert_eq!(count(&plan.root, tail_return), 2);
    }
}
