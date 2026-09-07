//! The layout tree. Every visible rectangle is a Slot; Splits carry an axis and a ratio.
//! Nodes live in an arena so a slot keeps its NodeId while its neighbours split or compact.
//! Nothing here collapses a node automatically; only `compact` removes empty slots.

use std::collections::HashMap;

use crate::geometry::{Edge, Rect, SplitAxis};

/// A top-level window handle, stored as an integer so this module stays Win32-free.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub isize);

/// Stable, lowercased application identifier. This is a process-image path on Windows, a
/// desktop-file/resource class on Linux, and pure data to the layout engine.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AppIdentity(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

pub const MIN_RATIO: f32 = 0.05;
pub const MAX_RATIO: f32 = 0.95;

/// Everything a slot holds; used to move contents between slots or trees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotContents {
    pub windows: Vec<WindowId>,
    pub active: usize,
    pub vacated_seq: Option<u64>,
    pub remembered: Option<AppIdentity>,
}

#[derive(Clone, Debug, PartialEq)]
enum NodeKind {
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: NodeId,
        second: NodeId,
    },
    /// `windows` is a stack sharing one rect; `active` indexes the raised one.
    /// `vacated_seq` is stamped when the last window leaves so new windows can
    /// prefer the most recently emptied slot. `remembered` is the identity of the last
    /// window to leave an EMPTY slot, so a reopened app can come back to it. Cleared
    /// whenever a window is assigned.
    Slot {
        windows: Vec<WindowId>,
        active: usize,
        vacated_seq: Option<u64>,
        remembered: Option<AppIdentity>,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct NodeData {
    kind: NodeKind,
    parent: Option<NodeId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    nodes: Vec<NodeData>,
    root: NodeId,
    next_seq: u64,
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

fn empty_slot() -> NodeKind {
    NodeKind::Slot { windows: Vec::new(), active: 0, vacated_seq: None, remembered: None }
}

impl Tree {
    pub fn snapshot(&self) -> crate::snapshots::Node {
        fn visit(t: &Tree, id: NodeId) -> crate::snapshots::Node {
            use crate::snapshots::Node;
            match &t.nodes[id.0].kind {
                NodeKind::Slot { windows, active, .. } => {
                    Node::Leaf { windows: windows.iter().map(|w| w.0.to_string()).collect(), active: *active }
                }
                NodeKind::Split { axis, ratio, first, second } => Node::Split {
                    axis: if *axis == SplitAxis::X { "x" } else { "y" }.into(),
                    ratio: *ratio as f64,
                    first: Box::new(visit(t, *first)),
                    second: Box::new(visit(t, *second)),
                },
            }
        }
        visit(self, self.root)
    }
    pub fn from_snapshot(node: &crate::snapshots::Node, mapping: &HashMap<String, WindowId>) -> Self {
        fn visit(
            t: &mut Tree,
            n: &crate::snapshots::Node,
            parent: Option<NodeId>,
            mapping: &HashMap<String, WindowId>,
        ) -> NodeId {
            let id = t.alloc(empty_slot(), parent);
            t.nodes[id.0].kind = match n {
                crate::snapshots::Node::Leaf { windows, active } => {
                    let windows: Vec<_> = windows.iter().filter_map(|w| mapping.get(w).copied()).collect();
                    let active = (*active).min(windows.len().saturating_sub(1));
                    NodeKind::Slot { windows, active, vacated_seq: None, remembered: None }
                }
                crate::snapshots::Node::Split { axis, ratio, first, second } => NodeKind::Split {
                    axis: if axis == "x" { SplitAxis::X } else { SplitAxis::Y },
                    ratio: *ratio as f32,
                    first: visit(t, first, Some(id), mapping),
                    second: visit(t, second, Some(id), mapping),
                },
            };
            id
        }
        let mut t = Self { nodes: Vec::new(), root: NodeId(0), next_seq: 1 };
        t.root = visit(&mut t, node, None, mapping);
        t
    }
    pub fn new() -> Self {
        Tree { nodes: vec![NodeData { kind: empty_slot(), parent: None }], root: NodeId(0), next_seq: 1 }
    }

    fn alloc(&mut self, kind: NodeKind, parent: Option<NodeId>) -> NodeId {
        self.nodes.push(NodeData { kind, parent });
        NodeId(self.nodes.len() - 1)
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0].parent
    }

    pub fn is_slot(&self, id: NodeId) -> bool {
        matches!(self.nodes[id.0].kind, NodeKind::Slot { .. })
    }

    pub fn slot_windows(&self, id: NodeId) -> &[WindowId] {
        match &self.nodes[id.0].kind {
            NodeKind::Slot { windows, .. } => windows,
            NodeKind::Split { .. } => &[],
        }
    }

    pub fn is_empty_slot(&self, id: NodeId) -> bool {
        self.is_slot(id) && self.slot_windows(id).is_empty()
    }

    pub fn active_window(&self, id: NodeId) -> Option<WindowId> {
        match &self.nodes[id.0].kind {
            NodeKind::Slot { windows, active, .. } => windows.get(*active).copied(),
            NodeKind::Split { .. } => None,
        }
    }

    pub fn split_info(&self, id: NodeId) -> Option<(SplitAxis, f32, NodeId, NodeId)> {
        match &self.nodes[id.0].kind {
            NodeKind::Split { axis, ratio, first, second } => Some((*axis, *ratio, *first, *second)),
            NodeKind::Slot { .. } => None,
        }
    }

    fn vacated_seq(&self, id: NodeId) -> Option<u64> {
        match &self.nodes[id.0].kind {
            NodeKind::Slot { vacated_seq, .. } => *vacated_seq,
            NodeKind::Split { .. } => None,
        }
    }

    pub fn remembered(&self, slot: NodeId) -> Option<&AppIdentity> {
        match &self.nodes[slot.0].kind {
            NodeKind::Slot { remembered, .. } => remembered.as_ref(),
            NodeKind::Split { .. } => None,
        }
    }

    pub fn set_remembered(&mut self, slot: NodeId, identity: Option<AppIdentity>) {
        if let NodeKind::Slot { remembered, .. } = &mut self.nodes[slot.0].kind {
            *remembered = identity;
        }
    }

    /// All slots reachable from the root, in preorder (left/top before right/bottom).
    pub fn slots(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_slots(self.root, &mut out);
        out
    }

    fn collect_slots(&self, id: NodeId, out: &mut Vec<NodeId>) {
        match &self.nodes[id.0].kind {
            NodeKind::Slot { .. } => out.push(id),
            NodeKind::Split { first, second, .. } => {
                self.collect_slots(*first, out);
                self.collect_slots(*second, out);
            }
        }
    }

    pub fn windows(&self) -> Vec<WindowId> {
        self.slots().iter().flat_map(|s| self.slot_windows(*s).iter().copied()).collect()
    }

    pub fn slot_of(&self, w: WindowId) -> Option<NodeId> {
        self.slots().into_iter().find(|s| self.slot_windows(*s).contains(&w))
    }

    /// Push `w` into `slot` and make it the active window. Panics on a split node.
    /// Clears the remembered tag: the occupant is the memory now.
    pub fn assign(&mut self, slot: NodeId, w: WindowId) {
        match &mut self.nodes[slot.0].kind {
            NodeKind::Slot { windows, active, vacated_seq, remembered } => {
                windows.push(w);
                *active = windows.len() - 1;
                *vacated_seq = None;
                *remembered = None;
            }
            NodeKind::Split { .. } => panic!("assign: {slot:?} is a split, not a slot"),
        }
    }

    /// Replace `target` with a split of `target` and a new slot holding `w`.
    /// `target` keeps its NodeId; the new slot's id is returned.
    pub fn split_slot(&mut self, target: NodeId, axis: SplitAxis, new_first: bool, w: WindowId) -> NodeId {
        assert!(self.is_slot(target), "split_slot: {target:?} is not a slot");
        let grand = self.parent(target);
        let new_slot =
            self.alloc(NodeKind::Slot { windows: vec![w], active: 0, vacated_seq: None, remembered: None }, None);
        let (first, second) = if new_first { (new_slot, target) } else { (target, new_slot) };
        let split = self.alloc(NodeKind::Split { axis, ratio: 0.5, first, second }, grand);
        self.nodes[target.0].parent = Some(split);
        self.nodes[new_slot.0].parent = Some(split);
        match grand {
            None => self.root = split,
            Some(g) => self.replace_child(g, target, split),
        }
        new_slot
    }

    fn replace_child(&mut self, parent: NodeId, old: NodeId, new: NodeId) {
        match &mut self.nodes[parent.0].kind {
            NodeKind::Split { first, second, .. } => {
                if *first == old {
                    *first = new;
                } else if *second == old {
                    *second = new;
                } else {
                    panic!("replace_child: {old:?} is not a child of {parent:?}");
                }
            }
            NodeKind::Slot { .. } => panic!("replace_child: {parent:?} is a slot"),
        }
    }

    /// Remove `w` from its slot. The slot stays; if it is now empty it is stamped as vacated.
    pub fn remove_window(&mut self, w: WindowId) -> Option<NodeId> {
        let slot = self.slot_of(w)?;
        let seq = self.next_seq;
        let mut vacated = false;
        if let NodeKind::Slot { windows, active, vacated_seq, .. } = &mut self.nodes[slot.0].kind {
            let idx = windows.iter().position(|x| *x == w)?;
            windows.remove(idx);
            if windows.is_empty() {
                *active = 0;
                *vacated_seq = Some(seq);
                vacated = true;
            } else {
                if idx < *active {
                    *active -= 1;
                }
                if *active >= windows.len() {
                    *active = windows.len() - 1;
                }
            }
        }
        if vacated {
            self.next_seq += 1;
        }
        Some(slot)
    }

    /// Remove `w`; if its slot becomes empty, stamp `vacated_seq` and record `identity` as the
    /// slot's memory (`None` clears any older tag so an unknown app never inherits one).
    pub fn remove_window_remembering(&mut self, w: WindowId, identity: Option<AppIdentity>) -> Option<NodeId> {
        let slot = self.remove_window(w)?;
        if self.is_empty_slot(slot) {
            self.set_remembered(slot, identity);
        }
        Some(slot)
    }

    /// The empty slot remembering `identity`, most recently vacated first.
    pub fn empty_slot_remembering(&self, identity: &AppIdentity) -> Option<NodeId> {
        self.slots()
            .into_iter()
            .rev()
            .filter(|s| self.is_empty_slot(*s) && self.remembered(*s) == Some(identity))
            .max_by_key(|s| self.vacated_seq(*s))
    }

    /// Empty `slot` and return what it held. Does not stamp a vacated sequence.
    pub fn take_contents(&mut self, slot: NodeId) -> SlotContents {
        match &mut self.nodes[slot.0].kind {
            NodeKind::Slot { windows, active, vacated_seq, remembered } => SlotContents {
                windows: std::mem::take(windows),
                active: std::mem::take(active),
                vacated_seq: std::mem::take(vacated_seq),
                remembered: std::mem::take(remembered),
            },
            NodeKind::Split { .. } => panic!("take_contents: {slot:?} is a split"),
        }
    }

    pub fn put_contents(&mut self, slot: NodeId, c: SlotContents) {
        match &mut self.nodes[slot.0].kind {
            NodeKind::Slot { windows, active, vacated_seq, remembered } => {
                *windows = c.windows;
                *active = c.active.min(windows.len().saturating_sub(1));
                *vacated_seq = c.vacated_seq;
                *remembered = c.remembered;
            }
            NodeKind::Split { .. } => panic!("put_contents: {slot:?} is a split"),
        }
    }

    pub fn swap_slots(&mut self, a: NodeId, b: NodeId) {
        if a == b {
            return;
        }
        let ca = self.take_contents(a);
        let cb = self.take_contents(b);
        self.put_contents(a, cb);
        self.put_contents(b, ca);
    }

    /// The empty slot that most recently lost its last window. Empty slots that were
    /// never occupied (a fresh root) qualify too, with the lowest priority.
    pub fn most_recently_vacated_empty(&self) -> Option<NodeId> {
        self.slots()
            .into_iter()
            .rev() // among equal keys max_by_key keeps the last, so reverse to prefer preorder-first
            .filter(|s| self.is_empty_slot(*s))
            .max_by_key(|s| self.vacated_seq(*s))
    }

    pub fn ratio(&self, split: NodeId) -> Option<f32> {
        self.split_info(split).map(|(_, r, _, _)| r)
    }

    pub fn set_ratio(&mut self, split: NodeId, ratio: f32) {
        if let NodeKind::Split { ratio: r, .. } = &mut self.nodes[split.0].kind {
            *r = ratio.clamp(MIN_RATIO, MAX_RATIO);
        }
    }

    /// Remove every empty slot. A split with one empty child is replaced by the other
    /// child, which takes the whole space. Orphaned arena entries are simply unreachable.
    pub fn compact(&mut self) {
        match self.compact_node(self.root) {
            Some(n) => {
                self.root = n;
                self.nodes[n.0].parent = None;
            }
            None => {
                if !self.is_empty_slot(self.root) {
                    let n = self.alloc(empty_slot(), None);
                    self.root = n;
                }
            }
        }
    }

    fn compact_node(&mut self, id: NodeId) -> Option<NodeId> {
        match self.split_info(id) {
            None => {
                if self.is_empty_slot(id) {
                    None
                } else {
                    Some(id)
                }
            }
            Some((_, _, first, second)) => {
                let a = self.compact_node(first);
                let b = self.compact_node(second);
                match (a, b) {
                    (Some(a), Some(b)) => {
                        if let NodeKind::Split { first, second, .. } = &mut self.nodes[id.0].kind {
                            *first = a;
                            *second = b;
                        }
                        self.nodes[a.0].parent = Some(id);
                        self.nodes[b.0].parent = Some(id);
                        Some(id)
                    }
                    (Some(only), None) | (None, Some(only)) => Some(only),
                    (None, None) => None,
                }
            }
        }
    }

    /// The nearest ancestor split that owns the given edge of `slot`, or `None` when that
    /// edge is the outer edge of the whole area. This is the walk half of `drag_edge`,
    /// separated so a caller can rank several edges by their owner before moving any.
    pub fn edge_owner(&self, slot: NodeId, edge: Edge) -> Option<NodeId> {
        let want = edge.axis();
        let mut child = slot;
        let mut cur = self.parent(slot);
        while let Some(p) = cur {
            if let Some((axis, _, first, _)) = self.split_info(p) {
                if axis == want {
                    let child_is_first = first == child;
                    // Left/Top edges are owned by a split where we are the SECOND child;
                    // Right/Bottom edges by one where we are the FIRST child.
                    let owns = match edge {
                        Edge::Left | Edge::Top => !child_is_first,
                        Edge::Right | Edge::Bottom => child_is_first,
                    };
                    if owns {
                        return Some(p);
                    }
                }
            }
            child = p;
            cur = self.parent(p);
        }
        None
    }

    /// Parent hops from `id` up to the root; the root itself is 0.
    pub fn depth(&self, id: NodeId) -> usize {
        let mut n = 0;
        let mut cur = self.parent(id);
        while let Some(p) = cur {
            n += 1;
            cur = self.parent(p);
        }
        n
    }

    /// Move the given edge of `slot` to absolute coordinate `pos` by adjusting the nearest
    /// ancestor split that owns that boundary. `rects` must come from `compute_rects` with
    /// the same `gap`. Returns `None` when the edge is the outer edge of the whole area.
    pub fn drag_edge(
        &mut self,
        slot: NodeId,
        edge: Edge,
        pos: i32,
        rects: &HashMap<NodeId, Rect>,
        gap: i32,
    ) -> Option<NodeId> {
        let p = self.edge_owner(slot, edge)?;
        let r = rects[&p];
        let (start, len) = match edge.axis() {
            SplitAxis::X => (r.x, r.w),
            SplitAxis::Y => (r.y, r.h),
        };
        let usable = (len - gap).max(1);
        let first_len = match edge {
            Edge::Left | Edge::Top => pos - gap - start, // pos is where `second` begins
            Edge::Right | Edge::Bottom => pos - start,   // pos is where `first` ends
        };
        self.set_ratio(p, first_len as f32 / usable as f32);
        Some(p)
    }

    /// Make `w` the raised window of its stack. Returns false if `w` is not in `slot`.
    pub fn set_active(&mut self, slot: NodeId, w: WindowId) -> bool {
        if let NodeKind::Slot { windows, active, .. } = &mut self.nodes[slot.0].kind {
            if let Some(i) = windows.iter().position(|x| *x == w) {
                *active = i;
                return true;
            }
        }
        false
    }

    /// Step the active index by `delta` (wrapping) and return the new active window.
    pub fn cycle_active(&mut self, slot: NodeId, delta: isize) -> Option<WindowId> {
        if let NodeKind::Slot { windows, active, .. } = &mut self.nodes[slot.0].kind {
            let n = windows.len() as isize;
            if n == 0 {
                return None;
            }
            *active = (*active as isize + delta).rem_euclid(n) as usize;
            return Some(windows[*active]);
        }
        None
    }

    /// Rect of every node (splits included) when the root fills `area`.
    /// `gap` pixels separate sibling children; the outer margin is the caller's business.
    pub fn compute_rects(&self, area: Rect, gap: i32) -> HashMap<NodeId, Rect> {
        let mut out = HashMap::new();
        self.layout(self.root, area, gap, &mut out);
        out
    }

    fn layout(&self, id: NodeId, r: Rect, gap: i32, out: &mut HashMap<NodeId, Rect>) {
        out.insert(id, r);
        if let Some((axis, ratio, first, second)) = self.split_info(id) {
            match axis {
                SplitAxis::X => {
                    let usable = (r.w - gap).max(0);
                    let a = ((usable as f32) * ratio).round() as i32;
                    let a = a.clamp(0, usable);
                    self.layout(first, Rect::new(r.x, r.y, a, r.h), gap, out);
                    self.layout(second, Rect::new(r.x + a + gap, r.y, usable - a, r.h), gap, out);
                }
                SplitAxis::Y => {
                    let usable = (r.h - gap).max(0);
                    let a = ((usable as f32) * ratio).round() as i32;
                    let a = a.clamp(0, usable);
                    self.layout(first, Rect::new(r.x, r.y, r.w, a), gap, out);
                    self.layout(second, Rect::new(r.x, r.y + a + gap, r.w, usable - a), gap, out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::SplitAxis::{X, Y};

    const AREA: Rect = Rect::new(0, 0, 1000, 500);
    const A: WindowId = WindowId(1);
    const B: WindowId = WindowId(2);
    const C: WindowId = WindowId(3);
    const D: WindowId = WindowId(4);

    fn rect(t: &Tree, n: NodeId) -> Rect {
        t.compute_rects(AREA, 0)[&n]
    }

    fn id(s: &str) -> AppIdentity {
        AppIdentity(s.to_string())
    }

    #[test]
    fn vacating_a_slot_remembers_the_app_and_assign_forgets_it() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        assert_eq!(t.remembered(a), None);
        assert_eq!(t.remove_window_remembering(A, Some(id("notepad"))), Some(a));
        assert_eq!(t.remembered(a), Some(&id("notepad")));
        t.assign(a, B);
        assert_eq!(t.remembered(a), None, "an occupant is the memory now");
    }

    #[test]
    fn removing_one_of_a_stack_keeps_the_slot_untagged() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        t.assign(a, B);
        t.remove_window_remembering(A, Some(id("notepad")));
        assert_eq!(t.remembered(a), None, "slot still occupied by B");
        t.remove_window_remembering(B, Some(id("code")));
        assert_eq!(t.remembered(a), Some(&id("code")));
    }

    #[test]
    fn remove_without_identity_clears_any_older_tag() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        t.set_remembered(a, Some(id("stale")));
        t.remove_window_remembering(A, None);
        assert_eq!(t.remembered(a), None, "unknown identity must not leave a stale tag");
    }

    #[test]
    fn empty_slot_remembering_prefers_the_most_recently_vacated_match() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let c = t.split_slot(b, Y, false, C);
        t.remove_window_remembering(A, Some(id("code")));
        t.remove_window_remembering(C, Some(id("code")));
        t.remove_window_remembering(B, Some(id("notepad")));
        assert_eq!(t.empty_slot_remembering(&id("code")), Some(c), "c was vacated after a");
        assert_eq!(t.empty_slot_remembering(&id("notepad")), Some(b));
        assert_eq!(t.empty_slot_remembering(&id("chrome")), None);
        t.assign(c, D);
        assert_eq!(t.empty_slot_remembering(&id("code")), Some(a), "occupied slots do not count");
    }

    #[test]
    fn remembered_tag_travels_with_slot_contents() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        t.remove_window_remembering(B, Some(id("notepad")));
        t.swap_slots(a, b);
        assert_eq!(t.remembered(a), Some(&id("notepad")));
        assert_eq!(t.remembered(b), None);
        let c = t.take_contents(a);
        assert_eq!(c.remembered, Some(id("notepad")));
        assert_eq!(t.remembered(a), None);
        t.put_contents(a, c);
        assert_eq!(t.remembered(a), Some(&id("notepad")));
    }

    #[test]
    fn compact_removes_remembered_empty_slots_too() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        t.remove_window_remembering(B, Some(id("notepad")));
        t.compact();
        assert_eq!(t.slots(), vec![a]);
        let _ = b;
    }

    #[test]
    fn new_tree_is_one_empty_slot_filling_the_area() {
        let t = Tree::new();
        assert!(t.is_empty_slot(t.root()));
        assert_eq!(t.slots(), vec![t.root()]);
        assert_eq!(rect(&t, t.root()), AREA);
        assert_eq!(t.parent(t.root()), None);
    }

    #[test]
    fn assign_fills_a_slot() {
        let mut t = Tree::new();
        t.assign(t.root(), A);
        assert_eq!(t.slot_of(A), Some(t.root()));
        assert_eq!(t.slot_windows(t.root()), &[A]);
        assert_eq!(t.active_window(t.root()), Some(A));
        assert_eq!(t.windows(), vec![A]);
    }

    #[test]
    fn stacked_windows_share_the_slot_and_the_last_assigned_is_active() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        t.assign(a, B);
        assert_eq!(t.slot_windows(a), &[A, B]);
        assert_eq!(t.active_window(a), Some(B));
        assert_eq!(t.slot_of(A), Some(a));
        assert_eq!(t.slots().len(), 1);
    }

    #[test]
    fn set_active_and_cycle() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        t.assign(a, B);
        t.assign(a, C);
        assert!(t.set_active(a, A));
        assert_eq!(t.active_window(a), Some(A));
        assert!(!t.set_active(a, D));
        assert_eq!(t.cycle_active(a, 1), Some(B));
        assert_eq!(t.cycle_active(a, 1), Some(C));
        assert_eq!(t.cycle_active(a, 1), Some(A), "wraps");
        assert_eq!(t.cycle_active(a, -1), Some(C), "negative wraps");
        assert_eq!(t.cycle_active(t.root(), 1), Some(A), "single-window and empty slots are harmless");
    }

    #[test]
    fn removing_the_active_window_keeps_a_valid_active_index() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        t.assign(a, B);
        t.remove_window(B);
        assert_eq!(t.active_window(a), Some(A));
        assert_eq!(t.cycle_active(a, 1), Some(A));
        t.remove_window(A);
        assert_eq!(t.cycle_active(a, 1), None);
    }

    #[test]
    fn split_keeps_the_target_id_and_inserts_a_split_above_it() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let split = t.root();
        assert_ne!(split, a);
        assert_eq!(t.split_info(split), Some((X, 0.5, a, b)));
        assert_eq!(t.parent(a), Some(split));
        assert_eq!(t.parent(b), Some(split));
        assert_eq!(t.slots(), vec![a, b]);
        assert_eq!(rect(&t, a), Rect::new(0, 0, 500, 500));
        assert_eq!(rect(&t, b), Rect::new(500, 0, 500, 500));
    }

    #[test]
    fn split_new_first_puts_the_new_window_left_or_top() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, Y, true, B);
        assert_eq!(rect(&t, b), Rect::new(0, 0, 1000, 250));
        assert_eq!(rect(&t, a), Rect::new(0, 250, 1000, 250));
    }

    #[test]
    fn gap_is_taken_between_children_only() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let rects = t.compute_rects(AREA, 10);
        assert_eq!(rects[&a], Rect::new(0, 0, 495, 500));
        assert_eq!(rects[&b], Rect::new(505, 0, 495, 500));
    }

    #[test]
    fn spec_example_browser_vscode_terminal_logs() {
        // Browser | VS Code, then Terminal under Browser, then Logs right of Terminal.
        let mut t = Tree::new();
        let browser = t.root();
        t.assign(browser, A);
        let vscode = t.split_slot(browser, X, false, B);
        let terminal = t.split_slot(browser, Y, false, C);
        let logs = t.split_slot(terminal, X, false, D);
        assert_eq!(rect(&t, browser), Rect::new(0, 0, 500, 250));
        assert_eq!(rect(&t, terminal), Rect::new(0, 250, 250, 250));
        assert_eq!(rect(&t, logs), Rect::new(250, 250, 250, 250));
        assert_eq!(rect(&t, vscode), Rect::new(500, 0, 500, 500));
        assert_eq!(t.slots(), vec![browser, terminal, logs, vscode]);
    }

    #[test]
    fn removing_a_window_leaves_its_slot_in_place() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let before = t.compute_rects(AREA, 0);
        assert_eq!(t.remove_window(B), Some(b));
        assert!(t.is_empty_slot(b));
        assert_eq!(t.slots(), vec![a, b]);
        assert_eq!(t.compute_rects(AREA, 0), before, "spatial memory: nothing moves");
        assert_eq!(t.vacated_seq(b), Some(1));
        assert_eq!(t.vacated_seq(a), None);
        assert_eq!(t.remove_window(A), Some(a));
        assert_eq!(t.vacated_seq(a), Some(2), "the sequence advances");
        assert_eq!(t.remove_window(B), None, "already gone");
    }

    #[test]
    fn removing_a_window_before_the_active_one_keeps_the_same_active_window() {
        let mut t = Tree::new();
        let root = t.root();
        t.put_contents(root, SlotContents { windows: vec![A, B, C], active: 1, vacated_seq: None, remembered: None });
        t.remove_window(A);
        assert_eq!(t.active_window(root), Some(B));
        t.remove_window(C);
        assert_eq!(t.active_window(root), Some(B));
    }

    #[test]
    fn swap_exchanges_contents_and_keeps_parents() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let (pa, pb) = (t.parent(a), t.parent(b));
        t.swap_slots(a, b);
        assert_eq!(t.slot_windows(a), &[B]);
        assert_eq!(t.slot_windows(b), &[A]);
        assert_eq!((t.parent(a), t.parent(b)), (pa, pb));
        assert_eq!(rect(&t, t.slot_of(A).unwrap()), Rect::new(500, 0, 500, 500));
    }

    #[test]
    fn swap_with_an_empty_slot_is_a_move() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        t.remove_window(B);
        t.swap_slots(a, b);
        assert!(t.is_empty_slot(a));
        assert_eq!(t.slot_windows(b), &[A]);
    }

    #[test]
    fn take_and_put_contents_round_trip() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        t.assign(a, B);
        let c = t.take_contents(a);
        assert_eq!(c.windows, vec![A, B]);
        assert_eq!(c.active, 1);
        assert!(t.is_empty_slot(a));
        t.put_contents(a, c);
        assert_eq!(t.slot_windows(a), &[A, B]);
        assert_eq!(t.active_window(a), Some(B));
    }

    #[test]
    #[should_panic]
    fn assign_to_a_split_panics() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        t.split_slot(a, X, false, B);
        let split = t.root();
        t.assign(split, C);
    }

    #[test]
    fn fresh_tree_offers_its_root_as_the_empty_slot() {
        let t = Tree::new();
        assert_eq!(t.most_recently_vacated_empty(), Some(t.root()));
    }

    #[test]
    fn most_recently_vacated_wins() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        t.remove_window(A);
        t.remove_window(B);
        assert_eq!(t.most_recently_vacated_empty(), Some(b));
        t.assign(b, C);
        assert_eq!(t.most_recently_vacated_empty(), Some(a));
        t.assign(a, D);
        assert_eq!(t.most_recently_vacated_empty(), None);
    }

    #[test]
    fn compact_removes_an_empty_sibling_and_gives_its_space_to_the_survivor() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        t.remove_window(B);
        t.compact();
        assert_eq!(t.root(), a, "the survivor becomes the root");
        assert_eq!(t.parent(a), None);
        assert_eq!(t.slots(), vec![a]);
        assert_eq!(rect(&t, a), AREA);
        let _ = b; // unreachable now
    }

    #[test]
    fn compact_is_recursive_and_keeps_surviving_ids_and_ratios() {
        // A | (Empty / C)  ->  A | C, and C takes the whole right half.
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let c = t.split_slot(b, Y, false, C);
        t.set_ratio(t.root(), 0.3);
        t.remove_window(B);
        t.compact();
        assert_eq!(t.slots(), vec![a, c]);
        assert_eq!(t.parent(c), Some(t.root()));
        assert_eq!(rect(&t, a), Rect::new(0, 0, 300, 500));
        assert_eq!(rect(&t, c), Rect::new(300, 0, 700, 500));
    }

    #[test]
    fn compact_of_an_all_empty_tree_leaves_one_empty_root() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        t.split_slot(a, X, false, B);
        t.remove_window(A);
        t.remove_window(B);
        t.compact();
        assert_eq!(t.slots().len(), 1);
        assert!(t.is_empty_slot(t.root()));
        assert_eq!(t.parent(t.root()), None);
    }

    #[test]
    fn compact_on_an_all_empty_tree_is_idempotent_and_keeps_the_root_id() {
        let mut t = Tree::new();
        let r = t.root();
        t.compact();
        assert_eq!(t.root(), r);
        t.compact();
        assert_eq!(t.root(), r);
        assert_eq!(t.slots(), vec![r]);

        let mut t2 = Tree::new();
        let a = t2.root();
        t2.assign(a, A);
        t2.split_slot(a, X, false, B);
        t2.remove_window(A);
        t2.remove_window(B);
        t2.compact();
        let r2 = t2.root();
        t2.compact();
        assert_eq!(t2.root(), r2);
        assert_eq!(t2.slots(), vec![r2]);
    }

    #[test]
    fn compact_with_nothing_empty_is_a_no_op() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        t.split_slot(a, X, false, B);
        let before = t.clone();
        t.compact();
        assert_eq!(t, before);
    }

    #[test]
    fn set_ratio_clamps() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        t.split_slot(a, X, false, B);
        let s = t.root();
        t.set_ratio(s, 0.0);
        assert_eq!(t.ratio(s), Some(MIN_RATIO));
        t.set_ratio(s, 2.0);
        assert_eq!(t.ratio(s), Some(MAX_RATIO));
        t.set_ratio(s, 0.3);
        assert_eq!(t.ratio(s), Some(0.3));
        assert_eq!(t.ratio(a), None);
    }

    #[test]
    fn dragging_the_boundary_between_two_slots_changes_one_ratio() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let split = t.root();
        let rects = t.compute_rects(AREA, 0);
        assert_eq!(t.drag_edge(b, Edge::Left, 300, &rects, 0), Some(split));
        assert!((t.ratio(split).unwrap() - 0.3).abs() < 1e-4);
        assert_eq!(rect(&t, a), Rect::new(0, 0, 300, 500));
        assert_eq!(rect(&t, b), Rect::new(300, 0, 700, 500));

        let rects = t.compute_rects(AREA, 0);
        assert_eq!(t.drag_edge(a, Edge::Right, 700, &rects, 0), Some(split));
        assert_eq!(rect(&t, a), Rect::new(0, 0, 700, 500));
    }

    #[test]
    fn monitor_edges_belong_to_no_split() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let rects = t.compute_rects(AREA, 0);
        assert_eq!(t.drag_edge(a, Edge::Left, 10, &rects, 0), None);
        assert_eq!(t.drag_edge(a, Edge::Top, 10, &rects, 0), None);
        assert_eq!(t.drag_edge(b, Edge::Right, 990, &rects, 0), None);
    }

    #[test]
    fn nested_slot_edge_walks_up_to_the_split_that_owns_it() {
        // A | (B / C): dragging C's LEFT edge adjusts the outer X split, not the inner Y split.
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let c = t.split_slot(b, Y, false, C);
        let outer = t.root();
        let inner = t.parent(c).unwrap();
        let rects = t.compute_rects(AREA, 0);
        assert_eq!(t.drag_edge(c, Edge::Left, 250, &rects, 0), Some(outer));
        assert_eq!(rect(&t, a), Rect::new(0, 0, 250, 500));
        assert_eq!(rect(&t, b), Rect::new(250, 0, 750, 250));
        assert_eq!(rect(&t, c), Rect::new(250, 250, 750, 250));
        // Dragging C's TOP edge adjusts the inner Y split.
        let rects = t.compute_rects(AREA, 0);
        assert_eq!(t.drag_edge(c, Edge::Top, 100, &rects, 0), Some(inner));
        assert_eq!(rect(&t, b), Rect::new(250, 0, 750, 100));
        assert_eq!(rect(&t, c), Rect::new(250, 100, 750, 400));
    }

    /// `edge_owner` is the walk `drag_edge` used to do inline, so it has to name the same
    /// splits that test names, and `depth` has to rank them outermost-first.
    #[test]
    fn edge_owner_and_depth_agree_with_drag_edge() {
        // A | (B / C), the same fixture as the test above.
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let c = t.split_slot(b, Y, false, C);
        let outer = t.root();
        let inner = t.parent(c).unwrap();
        assert_eq!(t.edge_owner(c, Edge::Left), Some(outer));
        assert_eq!(t.edge_owner(c, Edge::Top), Some(inner));
        assert_eq!(t.edge_owner(a, Edge::Left), None, "the outer edge of the area has no owner");
        assert_eq!(t.depth(outer), 0);
        assert_eq!(t.depth(inner), 1);
        assert_eq!(t.depth(c), 2);
    }

    #[test]
    fn drag_edge_accounts_for_the_gap() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let rects = t.compute_rects(AREA, 10);
        // B's visible left edge should land at x=310, so A ends at 300.
        t.drag_edge(b, Edge::Left, 310, &rects, 10);
        let rects = t.compute_rects(AREA, 10);
        assert_eq!(rects[&a], Rect::new(0, 0, 300, 500));
        assert_eq!(rects[&b], Rect::new(310, 0, 690, 500));
    }

    #[test]
    fn drag_edge_clamps_to_ratio_bounds() {
        let mut t = Tree::new();
        let a = t.root();
        t.assign(a, A);
        let b = t.split_slot(a, X, false, B);
        let rects = t.compute_rects(AREA, 0);
        t.drag_edge(b, Edge::Left, 5, &rects, 0);
        assert_eq!(t.ratio(t.root()), Some(MIN_RATIO));
    }
}
