use std::collections::{HashMap, HashSet};

use crate::cfg::Cfg;
use crate::ids::BlockId;

use super::traversal::successors;

struct Frame {
    block: BlockId,
    successors: Vec<BlockId>,
    next: usize,
    parent: Option<BlockId>,
}

/// Computes strongly connected components with iterative Tarjan traversal.
pub fn strongly_connected_components<T>(cfg: &Cfg<T>) -> Vec<Vec<BlockId>> {
    let mut next_index = 0usize;
    let mut indices = HashMap::new();
    let mut lowlinks = HashMap::new();
    let mut active = Vec::new();
    let mut on_stack = HashSet::new();
    let mut components = Vec::new();
    for root in cfg.block_ids() {
        if indices.contains_key(&root) {
            continue;
        }
        discover(root, &mut next_index, &mut indices, &mut lowlinks);
        active.push(root);
        on_stack.insert(root);
        let mut frames = vec![Frame {
            block: root,
            successors: successors(cfg, root),
            next: 0,
            parent: None,
        }];
        while !frames.is_empty() {
            let frame_index = frames.len() - 1;
            if frames[frame_index].next < frames[frame_index].successors.len() {
                let successor = frames[frame_index].successors[frames[frame_index].next];
                frames[frame_index].next += 1;
                if !indices.contains_key(&successor) {
                    discover(successor, &mut next_index, &mut indices, &mut lowlinks);
                    active.push(successor);
                    on_stack.insert(successor);
                    frames.push(Frame {
                        block: successor,
                        successors: successors(cfg, successor),
                        next: 0,
                        parent: Some(frames[frame_index].block),
                    });
                } else if on_stack.contains(&successor) {
                    let block = frames[frame_index].block;
                    if let Some(successor_index) = indices.get(&successor).copied() {
                        lowlinks
                            .entry(block)
                            .and_modify(|lowlink| *lowlink = (*lowlink).min(successor_index));
                    }
                }
                continue;
            }
            let Some(frame) = frames.pop() else {
                break;
            };
            if let Some(parent) = frame.parent
                && let Some(child_lowlink) = lowlinks.get(&frame.block).copied()
            {
                lowlinks
                    .entry(parent)
                    .and_modify(|lowlink| *lowlink = (*lowlink).min(child_lowlink));
            }
            if lowlinks.get(&frame.block) == indices.get(&frame.block) {
                components.push(pop_component(frame.block, &mut active, &mut on_stack));
            }
        }
    }
    components
}

fn discover(
    block: BlockId,
    next_index: &mut usize,
    indices: &mut HashMap<BlockId, usize>,
    lowlinks: &mut HashMap<BlockId, usize>,
) {
    indices.insert(block, *next_index);
    lowlinks.insert(block, *next_index);
    *next_index += 1;
}

fn pop_component(
    root: BlockId,
    active: &mut Vec<BlockId>,
    on_stack: &mut HashSet<BlockId>,
) -> Vec<BlockId> {
    let mut component = Vec::new();
    while let Some(block) = active.pop() {
        on_stack.remove(&block);
        component.push(block);
        if block == root {
            break;
        }
    }
    component
}
