//! The desktop: one layout tree per monitor, the focused window, floating windows, and
//! the policies that turn window events into tree edits. Pure; produces `Placement`s.

use std::collections::{HashMap, HashSet};

use crate::geometry::{changed_edges, drop_zone, longest_axis, DropZone, Edge, Point, Rect, SplitAxis};
use crate::tree::{NodeId, Tree, WindowId};

pub use crate::tree::AppIdentity;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MonitorId(pub isize);

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorState {
    pub id: MonitorId,
    pub work_area: Rect,
    pub tree: Tree,
}

/// Where one window should be. Stack members share a rect; `active` marks the raised one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub window: WindowId,
    pub rect: Rect,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropEffect {
    Swapped,
    Moved,
    Split,
    Stacked,
    Ignored,
}

pub const RESIZE_TOLERANCE_PX: i32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Desktop {
    monitors: Vec<MonitorState>,
    gap: i32,
    focused: Option<WindowId>,
    floating: HashSet<WindowId>,
    /// Identity of every tracked or floating window, so vacating can tag the slot.
    identities: HashMap<WindowId, AppIdentity>,
}

impl Desktop {
    pub fn snapshot(&self, windows: Vec<crate::snapshots::AppWindow>) -> crate::snapshots::Snapshot {
        crate::snapshots::Snapshot {
            schema: 1,
            name: String::new(),
            gap: self.gap,
            windows,
            monitors: self
                .monitors
                .iter()
                .map(|m| crate::snapshots::Monitor {
                    name: m.id.0.to_string(),
                    area: m.work_area,
                    root: m.tree.snapshot(),
                })
                .collect(),
        }
    }
    pub fn restore_snapshot(
        &mut self,
        s: &crate::snapshots::Snapshot,
        live: &[crate::snapshots::AppWindow],
    ) -> Result<usize, String> {
        crate::snapshots::validate(s)?;
        let matching = crate::snapshots::matching(&s.windows, live);
        let mapping: HashMap<_, _> = matching
            .iter()
            .filter_map(|(old, new)| new.parse::<isize>().ok().map(|id| (old.clone(), WindowId(id))))
            .collect();
        for (i, m) in self.monitors.iter_mut().enumerate() {
            let saved = s
                .monitors
                .iter()
                .find(|n| n.name == m.id.0.to_string())
                .or_else(|| s.monitors.iter().find(|n| n.area == m.work_area))
                .or_else(|| s.monitors.get(i));
            m.tree = saved.map(|n| Tree::from_snapshot(&n.root, &mapping)).unwrap_or_default();
        }
        self.gap = s.gap;
        self.floating.clear();
        self.identities.clear();
        for w in live {
            if let Ok(id) = w.token.parse::<isize>() {
                let id = WindowId(id);
                self.identities.insert(id, AppIdentity(w.app.clone()));
                if self.locate(id).is_none() {
                    self.floating.insert(id);
                }
            }
        }
        Ok(matching.len())
    }
    pub fn new(gap: i32) -> Self {
        Desktop { monitors: Vec::new(), gap, focused: None, floating: HashSet::new(), identities: HashMap::new() }
    }

    /// Record what app a window belongs to. Called for every window we start tracking, so a
    /// later vacate can leave the slot tagged even when the window arrived before we knew.
    pub fn note_identity(&mut self, w: WindowId, identity: AppIdentity) {
        self.identities.insert(w, identity);
    }

    pub fn identity_of(&self, w: WindowId) -> Option<&AppIdentity> {
        self.identities.get(&w)
    }

    fn forget_identity(&mut self, w: WindowId) {
        self.identities.remove(&w);
    }

    pub fn gap(&self) -> i32 {
        self.gap
    }
    pub fn set_gap(&mut self, gap: i32) {
        self.gap = gap.max(0);
    }

    pub fn monitors(&self) -> &[MonitorState] {
        &self.monitors
    }

    pub fn focused(&self) -> Option<WindowId> {
        self.focused
    }

    pub fn is_floating(&self, w: WindowId) -> bool {
        self.floating.contains(&w)
    }

    pub fn contains(&self, w: WindowId) -> bool {
        self.locate(w).is_some()
    }

    /// True when `w` shares its slot with at least one other window, i.e. raising it changes
    /// what is visible. A window alone in its slot — or one we do not track — is false.
    pub fn is_stacked(&self, w: WindowId) -> bool {
        match self.locate(w) {
            Some((mi, slot)) => self.monitors[mi].tree.slot_windows(slot).len() > 1,
            None => false,
        }
    }

    pub fn unstack(&mut self, w: WindowId) -> bool {
        if !self.is_stacked(w) {
            return false;
        }
        let Some(on) = self.monitor_of(w) else { return false };
        let identity = self.identities.get(&w).cloned();
        self.detach(w);
        self.window_appeared(w, on, identity)
    }

    pub fn monitor_of(&self, w: WindowId) -> Option<MonitorId> {
        self.locate(w).map(|(mi, _)| self.monitors[mi].id)
    }

    fn tiling_area(&self, m: &MonitorState) -> Rect {
        m.work_area.inset(self.gap)
    }

    fn monitor_index(&self, id: MonitorId) -> Option<usize> {
        self.monitors.iter().position(|m| m.id == id)
    }

    /// (monitor index, slot) of a tiled window.
    fn locate(&self, w: WindowId) -> Option<(usize, NodeId)> {
        self.monitors.iter().enumerate().find_map(|(i, m)| m.tree.slot_of(w).map(|s| (i, s)))
    }

    fn rects_for(&self, mi: usize) -> HashMap<NodeId, Rect> {
        let m = &self.monitors[mi];
        m.tree.compute_rects(self.tiling_area(m), self.gap)
    }

    /// Add new monitors with a fresh tree, update work areas of known ones, and re-home
    /// the windows of monitors that disappeared onto the first remaining monitor.
    pub fn sync_monitors(&mut self, current: &[(MonitorId, Rect)]) {
        // An empty enumeration is a transient glitch (driver reset, RDP disconnect); keep the
        // last known monitors rather than destroying the layout.
        if current.is_empty() {
            return;
        }
        for (id, area) in current {
            match self.monitor_index(*id) {
                Some(i) => self.monitors[i].work_area = *area,
                None => self.monitors.push(MonitorState { id: *id, work_area: *area, tree: Tree::new() }),
            }
        }
        let keep: HashSet<MonitorId> = current.iter().map(|(id, _)| *id).collect();
        let mut gone = Vec::new();
        self.monitors.retain(|m| {
            if keep.contains(&m.id) {
                true
            } else {
                gone.push(m.tree.windows());
                false
            }
        });
        if let Some((first, _)) = current.first() {
            for w in gone.into_iter().flatten() {
                let identity = self.identities.get(&w).cloned();
                self.window_appeared(w, *first, identity);
            }
        }
        // Keep the invariant local: never leave `focused` pointing at a window we no longer
        // track. Re-homing puts every window back today, so this cannot fire.
        if let Some(f) = self.focused {
            if !self.contains(f) && !self.floating.contains(&f) {
                self.focused = None;
            }
        }
    }

    /// Place a newly visible window, in precedence order: (1) an empty slot on that monitor
    /// that remembers `identity` — a reopened app goes back where it was, (2) the most
    /// recently vacated empty slot, (3) split the focused slot (if it is on that monitor) or
    /// the largest slot, along the slot's longest axis, with the new window on the
    /// right/bottom.
    pub fn window_appeared(&mut self, w: WindowId, on: MonitorId, identity: Option<AppIdentity>) -> bool {
        if self.contains(w) || self.floating.contains(&w) {
            return false;
        }
        let Some(mi) = self.monitor_index(on) else { return false };
        if let Some(id) = identity {
            self.identities.insert(w, id.clone());
            if let Some(slot) = self.monitors[mi].tree.empty_slot_remembering(&id) {
                self.monitors[mi].tree.assign(slot, w);
                return true;
            }
        }
        if let Some(empty) = self.monitors[mi].tree.most_recently_vacated_empty() {
            self.monitors[mi].tree.assign(empty, w);
            return true;
        }
        let rects = self.rects_for(mi);
        let focused_here = self.focused.and_then(|f| self.monitors[mi].tree.slot_of(f));
        let target = focused_here.unwrap_or_else(|| largest_slot(&self.monitors[mi].tree, &rects));
        let axis = longest_axis(rects[&target]);
        self.monitors[mi].tree.split_slot(target, axis, false, w);
        true
    }

    /// The window is gone: its slot stays empty, tagged with the app that just left it.
    /// Returns false if it was not tracked.
    pub fn window_vanished(&mut self, w: WindowId) -> bool {
        let was_tracked = if self.floating.remove(&w) {
            true
        } else if self.contains(w) {
            self.detach(w);
            true
        } else {
            false
        };
        if was_tracked {
            self.forget_identity(w);
            if self.focused == Some(w) {
                self.focused = None;
            }
        }
        was_tracked
    }

    /// Foreground changed. Untracked windows never steal the focused slot.
    pub fn focus_changed(&mut self, w: WindowId) {
        if let Some((mi, slot)) = self.locate(w) {
            self.focused = Some(w);
            self.monitors[mi].tree.set_active(slot, w);
        } else if self.floating.contains(&w) {
            self.focused = Some(w);
        }
    }

    pub fn rect_of(&self, w: WindowId) -> Option<Rect> {
        let (mi, slot) = self.locate(w)?;
        Some(self.rects_for(mi)[&slot])
    }

    /// Every tiled window's target rect. Within a stack the active window comes last so an
    /// applier that raises in order leaves it on top.
    ///
    /// `active` is ONLY set for the raised member of a real stack. Raising is a global
    /// Z-order edit, so marking the sole occupant of every slot active would have the
    /// applier rewrite the whole Z-order on every apply — burying anything the user had
    /// deliberately put on top (a floating window, a dialog). A window alone in its slot is
    /// already above nothing, so it never needs raising.
    pub fn placements(&self) -> Vec<Placement> {
        let mut out = Vec::new();
        for (mi, m) in self.monitors.iter().enumerate() {
            let rects = self.rects_for(mi);
            for slot in m.tree.slots() {
                let active = m.tree.active_window(slot);
                let rect = rects[&slot];
                let windows = m.tree.slot_windows(slot);
                let stacked = windows.len() > 1;
                let mut members: Vec<Placement> = windows
                    .iter()
                    .map(|w| Placement { window: *w, rect, active: stacked && Some(*w) == active })
                    .collect();
                members.sort_by_key(|p| p.active); // false before true
                out.extend(members);
            }
        }
        out
    }

    /// The slot under `p`. Points in a gap or margin snap to the nearest slot of that monitor.
    pub fn slot_at(&self, p: Point) -> Option<(MonitorId, NodeId)> {
        let (mi, m) = self.monitors.iter().enumerate().find(|(_, m)| m.work_area.contains(p))?;
        let rects = self.rects_for(mi);
        let slots = m.tree.slots();
        if let Some(s) = slots.iter().find(|s| rects[s].contains(p)) {
            return Some((m.id, *s));
        }
        slots.into_iter().min_by_key(|s| distance_sq(rects[s], p)).map(|s| (m.id, s))
    }

    /// Apply a drop of `dragged` at cursor position `at`. `stack` is the Ctrl modifier.
    pub fn drop_window(&mut self, dragged: WindowId, at: Point, stack: bool) -> DropEffect {
        // Floating means LEFT ALONE: a floating window's drags do nothing at all, so it stays
        // wherever the user puts it. Refused first, before the slot is even looked up, so that
        // `preview_rect` (which routes through here) yields None and no preview is drawn either.
        // `toggle_float` is the only way back into the tree.
        if self.floating.contains(&dragged) {
            return DropEffect::Ignored;
        }
        let Some((target_mon, target)) = self.slot_at(at) else { return DropEffect::Ignored };
        let tmi = self.monitor_index(target_mon).expect("slot_at only returns known monitors");
        let target_rect = self.rects_for(tmi)[&target];
        let zone = drop_zone(target_rect, at);
        let source = self.locate(dragged);

        if source == Some((tmi, target)) {
            // Dropped on its own slot: only meaningful when tearing a window out of a stack.
            if zone == DropZone::Center || self.monitors[tmi].tree.slot_windows(target).len() < 2 {
                return DropEffect::Ignored;
            }
            self.monitors[tmi].tree.remove_window(dragged);
            let (axis, new_first) = zone_split(zone);
            self.monitors[tmi].tree.split_slot(target, axis, new_first, dragged);
            return DropEffect::Split;
        }

        if self.monitors[tmi].tree.is_empty_slot(target) {
            self.detach(dragged);
            let mut selected = target;
            self.monitors[tmi].tree.assign(selected, dragged);
            for (axis, first) in crate::geometry::empty_splits(target_rect, at) {
                self.monitors[tmi].tree.remove_window(dragged);
                selected = self.monitors[tmi].tree.split_slot(selected, axis, first, dragged);
            }
            return DropEffect::Moved;
        }

        match zone {
            DropZone::Center if stack => {
                self.detach(dragged);
                self.monitors[tmi].tree.assign(target, dragged);
                DropEffect::Stacked
            }
            DropZone::Center => match source {
                Some((smi, src)) => {
                    self.swap(smi, src, tmi, target);
                    DropEffect::Swapped
                }
                None => {
                    // Nothing to swap with: tile it beside the target.
                    let axis = longest_axis(target_rect);
                    self.monitors[tmi].tree.split_slot(target, axis, false, dragged);
                    DropEffect::Split
                }
            },
            edge => {
                self.detach(dragged);
                let (axis, new_first) = zone_split(edge);
                self.monitors[tmi].tree.split_slot(target, axis, new_first, dragged);
                DropEffect::Split
            }
        }
    }

    /// Where `dragged` would land if dropped now; `None` when the drop would do nothing.
    pub fn preview_rect(&self, dragged: WindowId, at: Point, stack: bool) -> Option<Rect> {
        let mut sim = self.clone();
        if sim.drop_window(dragged, at, stack) == DropEffect::Ignored {
            return None;
        }
        sim.rect_of(dragged)
    }

    /// The user resized a tiled window natively. Each edge that moved drags the split that
    /// owns it. Returns true if any ratio changed.
    pub fn window_resized(&mut self, w: WindowId, before: Rect, after: Rect) -> bool {
        let Some((mi, slot)) = self.locate(w) else { return false };
        let gap = self.gap;
        let area = self.tiling_area(&self.monitors[mi]);
        // Two edges on the same axis are owned by two different splits, one nested inside
        // the other. Moving the inner one first is wasted: the outer move then rescales the
        // column the inner ratio was just measured against, and the window lands short of
        // where it was dragged. Outermost (shallowest) first, so every inner adjustment is
        // measured against a column that is already final.
        let mut edges: Vec<(Edge, NodeId)> = changed_edges(before, after, RESIZE_TOLERANCE_PX)
            .into_iter()
            .filter_map(|e| self.monitors[mi].tree.edge_owner(slot, e).map(|owner| (e, owner)))
            .collect();
        edges.sort_by_key(|(_, owner)| self.monitors[mi].tree.depth(*owner));
        let mut changed = false;
        for (edge, _) in edges {
            let rects = self.monitors[mi].tree.compute_rects(area, gap);
            let pos = match edge {
                Edge::Left => after.x,
                Edge::Right => after.right(),
                Edge::Top => after.y,
                Edge::Bottom => after.bottom(),
            };
            if self.monitors[mi].tree.drag_edge(slot, edge, pos, &rects, gap).is_some() {
                changed = true;
            }
        }
        changed
    }

    pub fn compact(&mut self, monitor: MonitorId) -> bool {
        match self.monitor_index(monitor) {
            Some(mi) => {
                self.monitors[mi].tree.compact();
                true
            }
            None => false,
        }
    }

    pub fn compact_all(&mut self) {
        for m in &mut self.monitors {
            m.tree.compact();
        }
    }

    /// Tiled → floating (slot left empty), floating → tiled (normal placement policy).
    /// Returns true when the window is now floating.
    pub fn toggle_float(&mut self, w: WindowId, on: MonitorId) -> bool {
        if self.floating.remove(&w) {
            let identity = self.identities.get(&w).cloned();
            self.window_appeared(w, on, identity);
            return false;
        }
        self.detach(w);
        self.floating.insert(w);
        true
    }

    /// Rotate the stack holding the focused window; the result becomes focused.
    pub fn cycle_stack(&mut self, delta: isize) -> Option<WindowId> {
        let (mi, slot) = self.locate(self.focused?)?;
        let next = self.monitors[mi].tree.cycle_active(slot, delta)?;
        self.focused = Some(next);
        Some(next)
    }

    /// Remove a tiled window from its slot (the slot stays, vacated and tagged with the
    /// window's identity). Every vacate path routes through here, so a slot always remembers
    /// the last app to leave it however the window left.
    fn detach(&mut self, w: WindowId) {
        let identity = self.identities.get(&w).cloned();
        if let Some((mi, _)) = self.locate(w) {
            self.monitors[mi].tree.remove_window_remembering(w, identity);
        }
    }

    fn swap(&mut self, a_mi: usize, a: NodeId, b_mi: usize, b: NodeId) {
        if a_mi == b_mi {
            self.monitors[a_mi].tree.swap_slots(a, b);
            return;
        }
        let ca = self.monitors[a_mi].tree.take_contents(a);
        let cb = self.monitors[b_mi].tree.take_contents(b);
        self.monitors[a_mi].tree.put_contents(a, cb);
        self.monitors[b_mi].tree.put_contents(b, ca);
    }
}

fn largest_slot(tree: &Tree, rects: &HashMap<NodeId, Rect>) -> NodeId {
    tree.slots().into_iter().max_by_key(|s| rects[s].area()).expect("a tree always has at least one slot")
}

fn zone_split(zone: DropZone) -> (SplitAxis, bool) {
    match zone {
        DropZone::Left => (SplitAxis::X, true),
        DropZone::Right => (SplitAxis::X, false),
        DropZone::Top => (SplitAxis::Y, true),
        DropZone::Bottom => (SplitAxis::Y, false),
        DropZone::Center => unreachable!("centre drops never split by zone"),
    }
}

fn distance_sq(r: Rect, p: Point) -> i64 {
    let dx = if p.x < r.x {
        r.x - p.x
    } else if p.x >= r.right() {
        p.x - r.right() + 1
    } else {
        0
    };
    let dy = if p.y < r.y {
        r.y - p.y
    } else if p.y >= r.bottom() {
        p.y - r.bottom() + 1
    } else {
        0
    };
    dx as i64 * dx as i64 + dy as i64 * dy as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const M1: MonitorId = MonitorId(1);
    pub(super) const M2: MonitorId = MonitorId(2);
    pub(super) const M1_AREA: Rect = Rect::new(0, 0, 1000, 500);
    pub(super) const M2_AREA: Rect = Rect::new(1000, 0, 600, 500);
    pub(super) const A: WindowId = WindowId(1);
    pub(super) const B: WindowId = WindowId(2);
    pub(super) const C: WindowId = WindowId(3);
    pub(super) const D: WindowId = WindowId(4);

    pub(super) fn desk() -> Desktop {
        let mut d = Desktop::new(0);
        d.sync_monitors(&[(M1, M1_AREA)]);
        d
    }

    pub(super) fn rect(d: &Desktop, w: WindowId) -> Rect {
        d.rect_of(w).expect("window is tiled")
    }

    fn two_up() -> Desktop {
        // A | B, each 500x500.
        let mut d = desk();
        d.window_appeared(A, M1, None);
        d.focus_changed(A);
        d.window_appeared(B, M1, None);
        d
    }

    #[test]
    fn first_window_fills_the_monitor() {
        let mut d = desk();
        assert!(d.window_appeared(A, M1, None));
        assert_eq!(d.placements(), vec![Placement { window: A, rect: M1_AREA, active: false }]);
        assert!(!d.window_appeared(A, M1, None), "already tracked");
        assert!(!d.window_appeared(B, MonitorId(99), None), "unknown monitor");
    }

    #[test]
    fn focus_only_lands_after_the_window_is_tracked() {
        // The applier depends on this: a FOREGROUND event for an untracked window must track
        // first and focus second, or focus is silently dropped and the next new window splits
        // the previously focused slot.
        let mut d = desk();
        d.focus_changed(A);
        d.window_appeared(A, M1, None);
        assert_eq!(d.focused(), None, "focus before tracking is ignored");
        d.focus_changed(A);
        assert_eq!(d.focused(), Some(A));
    }

    #[test]
    fn gap_insets_the_work_area() {
        let mut d = Desktop::new(10);
        d.sync_monitors(&[(M1, M1_AREA)]);
        d.window_appeared(A, M1, None);
        assert_eq!(rect(&d, A), Rect::new(10, 10, 980, 480));
    }

    #[test]
    fn new_window_splits_the_focused_slot_along_its_longest_axis() {
        let mut d = desk();
        d.window_appeared(A, M1, None);
        d.focus_changed(A);
        d.window_appeared(B, M1, None);
        assert_eq!(rect(&d, A), Rect::new(0, 0, 500, 500));
        assert_eq!(rect(&d, B), Rect::new(500, 0, 500, 500));
        d.focus_changed(B);
        d.window_appeared(C, M1, None); // B is 500x500: tie -> X
        assert_eq!(rect(&d, B), Rect::new(500, 0, 250, 500));
        assert_eq!(rect(&d, C), Rect::new(750, 0, 250, 500));
        d.focus_changed(C);
        d.window_appeared(D, M1, None); // C is 250x500: tall -> Y
        assert_eq!(rect(&d, C), Rect::new(750, 0, 250, 250));
        assert_eq!(rect(&d, D), Rect::new(750, 250, 250, 250));
    }

    #[test]
    fn without_focus_the_largest_slot_is_split() {
        let mut d = desk();
        d.window_appeared(A, M1, None);
        d.window_appeared(B, M1, None); // no focus: A is the only slot
        d.focus_changed(A);
        d.window_appeared(C, M1, None); // A (500 wide) splits -> A 250, C 250
        d.window_vanished(A);
        d.focus_changed(B); // B is 500 wide: the largest
        assert_eq!(d.focused(), Some(B));
        let mut d2 = d.clone();
        d2.window_appeared(D, M1, None); // vacated slot wins over focus
        assert_eq!(rect(&d2, D), Rect::new(0, 0, 250, 500));
    }

    /// Reaches the `largest_slot` fallback with THREE occupied slots of unequal area, which
    /// the other tests never do -- they all fall back on a one-slot tree, where any choice
    /// rule looks correct. Sabotage: `max_by_key` -> `min_by_key` in `largest_slot` must
    /// fail this test.
    #[test]
    fn without_focus_the_widest_slot_wins() {
        let mut d = desk();
        d.window_appeared(A, M1, None);
        d.focus_changed(A);
        d.window_appeared(B, M1, None); // A (0,0,500,500) | B (500,0,500,500)
        d.focus_changed(B);
        d.window_appeared(C, M1, None); // B (500,0,250,500) | C (750,0,250,500)

        // Vanishing the FOCUSED window is what clears focus, so B (not C) has to go: that
        // leaves focus None and B's slot vacated.
        assert!(d.window_vanished(B));
        assert_eq!(d.focused(), None);
        d.window_appeared(D, M1, None); // takes B's vacated slot; does not set focus
        assert_eq!(rect(&d, D), Rect::new(500, 0, 250, 500));
        assert_eq!(d.focused(), None, "appearing never steals focus");
        // Three occupied slots, no vacancy, no focus: widths 500 / 250 / 250.
        let e = WindowId(5);
        d.window_appeared(e, M1, None);
        assert_eq!(rect(&d, A), Rect::new(0, 0, 250, 500), "the 500-wide slot split, not a 250");
        assert_eq!(rect(&d, e), Rect::new(250, 0, 250, 500));
    }

    #[test]
    fn closing_leaves_a_hole_and_the_next_window_takes_it() {
        let mut d = desk();
        d.window_appeared(A, M1, None);
        d.focus_changed(A);
        d.window_appeared(B, M1, None);
        d.focus_changed(B);
        d.window_appeared(C, M1, None);
        let b_rect = rect(&d, B);
        let a_rect = rect(&d, A);
        let c_rect = rect(&d, C);
        assert!(d.window_vanished(B));
        assert!(!d.contains(B));
        assert_eq!(rect(&d, A), a_rect, "nothing moved");
        assert_eq!(rect(&d, C), c_rect, "nothing moved");
        assert_eq!(d.focused(), None, "the focused window vanished");
        d.window_appeared(D, M1, None);
        assert_eq!(rect(&d, D), b_rect, "new window fills the hole");
        assert!(!d.window_vanished(B), "already gone");
    }

    #[test]
    fn focus_on_an_untracked_window_does_not_change_the_focused_slot() {
        let mut d = desk();
        d.window_appeared(A, M1, None);
        d.focus_changed(A);
        d.focus_changed(WindowId(999));
        assert_eq!(d.focused(), Some(A));
    }

    /// Holds BOTH halves of the `active` flag: only a real stack's raised member is marked
    /// (a sole occupant is already above nothing, and raising it would rewrite the global
    /// Z-order on every apply), and within a stack the raised member is LAST so an applier
    /// that raises in order leaves it on top.
    ///
    /// The stack is built by a Ctrl drop, then focus is moved to B — the member that was
    /// there FIRST — so the active one is not the last-assigned. Sabotage: delete the
    /// `sort_by_key` line in `placements` and the "raised member last" assertion fails,
    /// because `slot_windows` yields B then A in assignment order.
    #[test]
    fn placements_list_stack_members_with_the_active_one_last() {
        let mut d = two_up(); // A (0,0,500,500) | B (500,0,500,500)
        assert_eq!(d.drop_window(A, Point { x: 750, y: 250 }, true), DropEffect::Stacked);
        d.window_appeared(C, M1, None); // fills A's vacated slot: a single-occupant slot to compare against
        d.focus_changed(B);
        let stack_rect = Rect::new(500, 0, 500, 500);
        let p = d.placements();
        assert_eq!(p.len(), 3);
        let stacked: Vec<_> = p.iter().filter(|p| p.rect == stack_rect).collect();
        assert_eq!(stacked.len(), 2, "B and A share one slot");
        assert_eq!(
            *stacked.last().unwrap(),
            &Placement { window: B, rect: stack_rect, active: true },
            "the focused stack member is raised, and comes last"
        );
        assert_eq!(
            *stacked.first().unwrap(),
            &Placement { window: A, rect: stack_rect, active: false },
            "the other stack member is not raised"
        );
        let alone = p.iter().find(|p| p.window == C).expect("C is placed");
        assert!(!alone.active, "a sole occupant is never raised");
        assert_eq!(d.monitor_of(A), Some(M1));
        assert_eq!(d.monitor_of(WindowId(9)), None);
    }

    #[test]
    fn is_stacked_is_true_only_for_a_shared_slot() {
        let mut d = two_up();
        assert!(!d.is_stacked(A), "alone in its slot");
        assert!(!d.is_stacked(C), "untracked");
        assert_eq!(d.drop_window(A, Point { x: 750, y: 250 }, true), DropEffect::Stacked);
        assert!(d.is_stacked(A));
        assert!(d.is_stacked(B), "both members of the stack");
        d.window_vanished(A);
        assert!(!d.is_stacked(B), "the last window in a slot is not stacked");
    }

    #[test]
    fn monitors_are_independent_trees() {
        let mut d = desk();
        d.sync_monitors(&[(M1, M1_AREA), (M2, M2_AREA)]);
        d.window_appeared(A, M1, None);
        d.window_appeared(B, M2, None);
        assert_eq!(rect(&d, A), M1_AREA);
        assert_eq!(rect(&d, B), M2_AREA);
        assert_eq!(d.monitors().len(), 2);
    }

    #[test]
    fn removing_a_monitor_rehomes_its_windows() {
        let mut d = desk();
        d.sync_monitors(&[(M1, M1_AREA), (M2, M2_AREA)]);
        d.window_appeared(A, M1, None);
        d.window_appeared(B, M2, None);
        d.focus_changed(B);
        d.sync_monitors(&[(M1, M1_AREA)]);
        assert_eq!(d.monitors().len(), 1);
        assert_eq!(d.monitor_of(B), Some(M1));
        assert!(d.contains(A) && d.contains(B));
        assert_eq!(d.focused(), Some(B), "the focused window survived re-homing");
        assert!(d.contains(B), "focused() never names an untracked window");
    }

    #[test]
    fn sync_with_no_monitors_keeps_the_layout() {
        let mut d = desk();
        d.window_appeared(A, M1, None);
        d.window_appeared(B, M1, None);
        d.sync_monitors(&[]);
        assert_eq!(d.monitors().len(), 1, "a transient empty enumeration is ignored");
        assert!(d.contains(A) && d.contains(B));
    }

    #[test]
    fn sync_updates_work_area_in_place() {
        let mut d = desk();
        d.window_appeared(A, M1, None);
        d.sync_monitors(&[(M1, Rect::new(0, 0, 2000, 1000))]);
        assert_eq!(rect(&d, A), Rect::new(0, 0, 2000, 1000));
    }

    #[test]
    fn slot_at_finds_the_slot_under_a_point_and_snaps_gaps_to_the_nearest() {
        let mut d = Desktop::new(10);
        d.sync_monitors(&[(M1, M1_AREA)]);
        d.window_appeared(A, M1, None);
        d.focus_changed(A);
        d.window_appeared(B, M1, None);
        let (m, a) = d.slot_at(Point { x: 100, y: 100 }).unwrap();
        assert_eq!(m, M1);
        assert_eq!(a, d.monitors()[0].tree.slot_of(A).unwrap());
        let (_, in_gap) = d.slot_at(Point { x: 5, y: 250 }).unwrap(); // outer margin
        assert_eq!(in_gap, a);
        assert_eq!(d.slot_at(Point { x: 5000, y: 5 }), None);
    }

    #[test]
    fn centre_drop_swaps() {
        let mut d = two_up();
        assert_eq!(d.drop_window(A, Point { x: 750, y: 250 }, false), DropEffect::Swapped);
        assert_eq!(rect(&d, A), Rect::new(500, 0, 500, 500));
        assert_eq!(rect(&d, B), Rect::new(0, 0, 500, 500));
    }

    #[test]
    fn edge_drop_of_an_untracked_window_splits_the_target() {
        let mut d = two_up();
        // Bottom of A.
        assert_eq!(d.drop_window(C, Point { x: 250, y: 480 }, false), DropEffect::Split);
        assert_eq!(rect(&d, A), Rect::new(0, 0, 500, 250));
        assert_eq!(rect(&d, C), Rect::new(0, 250, 500, 250));
        assert_eq!(rect(&d, B), Rect::new(500, 0, 500, 500));
        // Right of C.
        assert_eq!(d.drop_window(D, Point { x: 480, y: 375 }, false), DropEffect::Split);
        assert_eq!(rect(&d, C), Rect::new(0, 250, 250, 250));
        assert_eq!(rect(&d, D), Rect::new(250, 250, 250, 250));
        // Left and top zones put the new window first.
        let mut d = two_up();
        d.drop_window(C, Point { x: 20, y: 250 }, false);
        assert_eq!(rect(&d, C), Rect::new(0, 0, 250, 500));
        let mut d = two_up();
        d.drop_window(C, Point { x: 250, y: 20 }, false);
        assert_eq!(rect(&d, C), Rect::new(0, 0, 500, 250));
    }

    #[test]
    fn edge_drop_of_a_tiled_window_leaves_its_old_slot_empty() {
        let mut d = two_up();
        assert_eq!(d.drop_window(B, Point { x: 250, y: 480 }, false), DropEffect::Split);
        assert_eq!(rect(&d, A), Rect::new(0, 0, 500, 250));
        assert_eq!(rect(&d, B), Rect::new(0, 250, 500, 250));
        let tree = &d.monitors()[0].tree;
        assert_eq!(tree.slots().len(), 3);
        let empty = tree.most_recently_vacated_empty().expect("B's old slot");
        assert_eq!(tree.compute_rects(M1_AREA, 0)[&empty], Rect::new(500, 0, 500, 500));
    }

    #[test]
    fn center_drop_on_an_empty_slot_fills_it() {
        let mut d = two_up();
        d.window_vanished(B);
        assert_eq!(d.drop_window(C, Point { x: 750, y: 250 }, false), DropEffect::Moved);
        assert_eq!(rect(&d, C), Rect::new(500, 0, 500, 500));
        assert_eq!(d.drop_window(A, Point { x: 750, y: 250 }, false), DropEffect::Swapped);
        assert_eq!(rect(&d, A), Rect::new(500, 0, 500, 500));
        assert_eq!(rect(&d, C), Rect::new(0, 0, 500, 500));
    }

    #[test]
    fn empty_space_halves_and_quarters_match_previews() {
        for x in [550, 750, 950] {
            for y in [50, 250, 450] {
                let mut d = two_up();
                d.window_vanished(B);
                let at = Point { x, y };
                let preview = d.preview_rect(C, at, false).unwrap();
                assert_eq!(d.drop_window(C, at, false), DropEffect::Moved);
                assert_eq!(rect(&d, C), preview);
                assert_eq!(preview.w, if x == 750 { 500 } else { 250 });
                assert_eq!(preview.h, if y == 250 { 500 } else { 250 });
                assert_eq!(rect(&d, A), Rect::new(0, 0, 500, 500));
            }
        }
    }

    #[test]
    fn unstack_uses_a_vacancy_and_keeps_both_windows() {
        let mut d = two_up();
        d.drop_window(B, Point { x: 250, y: 250 }, true);
        assert!(d.is_stacked(A));
        assert!(d.unstack(B));
        assert!(!d.is_stacked(A));
        assert!(!d.is_stacked(B));
        assert!(d.contains(A) && d.contains(B));
        assert_eq!(rect(&d, B), Rect::new(500, 0, 500, 500));
    }

    #[test]
    fn drops_that_change_nothing_are_ignored() {
        let mut d = two_up();
        let before = d.clone();
        assert_eq!(d.drop_window(A, Point { x: 5000, y: 5 }, false), DropEffect::Ignored, "outside all monitors");
        assert_eq!(d.drop_window(A, Point { x: 250, y: 250 }, false), DropEffect::Ignored, "own slot centre");
        assert_eq!(
            d.drop_window(A, Point { x: 250, y: 480 }, false),
            DropEffect::Ignored,
            "own slot edge, not stacked"
        );
        assert_eq!(d, before);
    }

    #[test]
    fn untracked_window_dropped_on_an_occupied_centre_splits_along_the_longest_axis() {
        let mut d = two_up();
        assert_eq!(d.drop_window(C, Point { x: 750, y: 250 }, false), DropEffect::Split);
        assert_eq!(rect(&d, B), Rect::new(500, 0, 250, 500));
        assert_eq!(rect(&d, C), Rect::new(750, 0, 250, 500));
    }

    #[test]
    fn ctrl_centre_drop_stacks_and_stack_members_share_the_rect() {
        let mut d = two_up();
        assert_eq!(d.drop_window(A, Point { x: 750, y: 250 }, true), DropEffect::Stacked);
        assert_eq!(rect(&d, A), Rect::new(500, 0, 500, 500));
        assert_eq!(rect(&d, B), Rect::new(500, 0, 500, 500));
        let p = d.placements();
        let stacked: Vec<_> = p.iter().filter(|p| p.rect == Rect::new(500, 0, 500, 500)).collect();
        assert_eq!(stacked.len(), 2);
        assert_eq!(stacked.last().unwrap().window, A, "dragged window is active and last");
        // Tearing A out of the stack by dropping it on an edge of its own slot.
        assert_eq!(d.drop_window(A, Point { x: 750, y: 480 }, false), DropEffect::Split);
        assert_eq!(rect(&d, B), Rect::new(500, 0, 500, 250));
        assert_eq!(rect(&d, A), Rect::new(500, 250, 500, 250));
    }

    #[test]
    fn cross_monitor_drop_moves_between_trees() {
        let mut d = two_up();
        d.sync_monitors(&[(M1, M1_AREA), (M2, M2_AREA)]);
        assert_eq!(d.drop_window(A, Point { x: 1300, y: 250 }, false), DropEffect::Moved);
        assert_eq!(d.monitor_of(A), Some(M2));
        assert_eq!(rect(&d, A), M2_AREA);
        assert!(d.monitors()[0].tree.most_recently_vacated_empty().is_some(), "hole left on M1");
        // Swap across monitors.
        assert_eq!(d.drop_window(B, Point { x: 1300, y: 250 }, false), DropEffect::Swapped);
        assert_eq!(d.monitor_of(B), Some(M2));
        assert_eq!(d.monitor_of(A), Some(M1));
    }

    /// Holds the `zone == DropZone::Center` half of the own-slot guard, which no other test
    /// reaches: the `len() < 2` half already covers a lone window. Without it a centre drop
    /// onto a slot the dragged window already shares falls through to `zone_split(Center)`
    /// and hits its `unreachable!`. Sabotage: delete `zone == DropZone::Center ||` and this
    /// test must panic.
    #[test]
    fn centre_drop_of_a_stacked_window_on_its_own_slot_is_ignored() {
        let mut d = two_up();
        assert_eq!(d.drop_window(A, Point { x: 750, y: 250 }, true), DropEffect::Stacked);
        let before = d.clone();
        assert_eq!(d.drop_window(A, Point { x: 750, y: 250 }, false), DropEffect::Ignored);
        assert_eq!(d, before);
    }

    #[test]
    fn preview_matches_the_real_drop() {
        let d = two_up();
        let at = Point { x: 250, y: 480 };
        let preview = d.preview_rect(C, at, false).unwrap();
        let mut real = d.clone();
        real.drop_window(C, at, false);
        assert_eq!(preview, rect(&real, C));
        assert_eq!(d.preview_rect(A, Point { x: 5000, y: 5 }, false), None);
        assert_eq!(d.preview_rect(A, Point { x: 250, y: 250 }, false), None, "no-op drop has no preview");
        // Stacking is the one preview branch nothing else exercises: with `stack` the drop
        // joins the target's slot instead of splitting it, so the preview is B's whole rect.
        let stacked = d.preview_rect(A, Point { x: 750, y: 250 }, true).unwrap();
        assert_eq!(stacked, Rect::new(500, 0, 500, 500), "stack preview is the target's whole rect");
        // That assertion alone cannot fail on the flag: a TRACKED window dropped on an occupied
        // centre swaps, and a swap puts it on the target's rect too, so it reads the same with
        // `stack = false`. Dropping an UNTRACKED window is where the two answers diverge -- stack
        // assigns it to the target's slot, no-stack tiles it beside -- so this pair, not the line
        // above, is what goes red if `stack` stops being honoured.
        assert_eq!(
            d.preview_rect(C, Point { x: 750, y: 250 }, true).unwrap(),
            Rect::new(500, 0, 500, 500),
            "stacked: the untracked window joins B's slot"
        );
        assert_eq!(
            d.preview_rect(C, Point { x: 750, y: 250 }, false).unwrap(),
            Rect::new(750, 0, 250, 500),
            "not stacked: the same drop splits B along its longest axis instead"
        );
    }

    #[test]
    fn floating_window_drags_are_ignored() {
        let mut d = two_up();
        d.toggle_float(B, M1);
        assert!(d.is_floating(B));
        let before = d.clone();
        let at = Point { x: 250, y: 480 }; // bottom edge of A's slot: a live target for anyone else
        assert_eq!(d.drop_window(B, at, false), DropEffect::Ignored);
        assert_eq!(d, before, "the drop changed nothing at all");
        assert!(d.is_floating(B), "and it is still floating");
        assert_eq!(d.preview_rect(B, at, false), None, "so there is nothing to preview either");
        // The refusal is about the WINDOW, not the target: an untracked window dropped on the
        // same point still splits, which is the path this must not have broken.
        let mut d2 = d.clone();
        assert_eq!(d2.drop_window(C, at, false), DropEffect::Split);
        assert_eq!(rect(&d2, C), Rect::new(0, 250, 500, 250));
        // Win+Shift+F is the only way back into the tree, and it still works.
        assert!(!d.toggle_float(B, M1));
        assert!(d.contains(B) && !d.is_floating(B));
    }

    #[test]
    fn resizing_a_window_moves_the_shared_boundary() {
        let mut d = two_up();
        let before = rect(&d, A);
        let after = Rect::new(0, 0, 700, 500); // user dragged A's right edge to 700
        assert!(d.window_resized(A, before, after));
        assert_eq!(rect(&d, A), Rect::new(0, 0, 700, 500));
        assert_eq!(rect(&d, B), Rect::new(700, 0, 300, 500));
    }

    #[test]
    fn resizing_against_the_monitor_edge_changes_nothing() {
        let mut d = two_up();
        let before = rect(&d, A);
        let snapshot = d.clone();
        assert!(!d.window_resized(A, before, Rect::new(50, 0, 450, 500)));
        assert_eq!(d, snapshot);
        // A DIFFERING `after`, so it is the untracked-window path returning false and not
        // simply "nothing moved" — with `before == after` this assertion held for any window.
        assert!(!d.window_resized(WindowId(77), before, Rect::new(0, 0, 700, 500)), "untracked");
    }

    #[test]
    fn resizing_two_edges_adjusts_two_splits() {
        // A | (B / C): grow C's left edge inward and its top edge upward in one gesture.
        let mut d = two_up();
        d.focus_changed(B);
        d.drop_window(C, Point { x: 750, y: 480 }, false); // C under B
        let before = rect(&d, C);
        assert_eq!(before, Rect::new(500, 250, 500, 250));
        assert!(d.window_resized(C, before, Rect::new(400, 100, 600, 400)));
        assert_eq!(rect(&d, A), Rect::new(0, 0, 400, 500));
        assert_eq!(rect(&d, B), Rect::new(400, 0, 600, 100));
        assert_eq!(rect(&d, C), Rect::new(400, 100, 600, 400));
    }

    /// Holds the per-edge RECOMPUTE inside `window_resized`. The two edges of
    /// `resizing_two_edges_adjusts_two_splits` sit on orthogonal axes, so a stale rect is
    /// still correct there; only two edges on the SAME axis, owned by nested splits, can
    /// tell the difference. Sabotage: hoist `compute_rects` out of the loop and B lands at
    /// 360 wide instead of 400, because the inner split is measured against the column it
    /// occupied before the outer split moved.
    #[test]
    fn resizing_both_edges_of_a_middle_window_uses_fresh_rects() {
        let mut d = two_up();
        d.focus_changed(B);
        d.window_appeared(C, M1, None); // A (0,500) | B (500,250) | C (750,250)
        let before = rect(&d, B);
        assert_eq!(before, Rect::new(500, 0, 250, 500));
        assert!(d.window_resized(B, before, Rect::new(400, 0, 400, 500)));
        assert_eq!(rect(&d, A), Rect::new(0, 0, 400, 500));
        assert_eq!(rect(&d, B), Rect::new(400, 0, 400, 500), "right edge measured against the MOVED column");
        assert_eq!(rect(&d, C), Rect::new(800, 0, 200, 500));
    }

    /// The mirror of the test above: here the SECOND edge in `changed_edges` order is the
    /// one owned by the OUTER split, so applying them in that fixed order rescales the
    /// inner adjustment just made and C lands 40px short. Sabotage: drop the depth sort in
    /// `window_resized` and C is (240, 0, 360, 500) instead of (200, 0, 400, 500).
    #[test]
    fn resizing_both_edges_when_the_outer_split_owns_the_second_edge() {
        let mut d = two_up();
        d.focus_changed(A);
        d.window_appeared(C, M1, None); // (A | C) | B -- C's LEFT is inner, its RIGHT is the root
        let before = rect(&d, C);
        assert_eq!(before, Rect::new(250, 0, 250, 500));
        assert!(d.window_resized(C, before, Rect::new(200, 0, 400, 500)));
        assert_eq!(rect(&d, C), Rect::new(200, 0, 400, 500), "lands exactly where it was dragged");
        assert_eq!(rect(&d, A), Rect::new(0, 0, 200, 500));
        assert_eq!(rect(&d, B), Rect::new(600, 0, 400, 500));
    }

    #[test]
    fn compact_removes_holes_on_one_monitor() {
        let mut d = two_up();
        d.window_vanished(B);
        assert!(d.compact(M1));
        assert_eq!(rect(&d, A), M1_AREA);
        assert!(!d.compact(MonitorId(99)));
    }

    #[test]
    fn compact_all_covers_every_monitor() {
        let mut d = two_up();
        d.sync_monitors(&[(M1, M1_AREA), (M2, M2_AREA)]);
        d.window_appeared(C, M2, None);
        d.focus_changed(C);
        d.window_appeared(D, M2, None);
        d.window_vanished(B);
        d.window_vanished(D);
        d.compact_all();
        assert_eq!(rect(&d, A), M1_AREA);
        assert_eq!(rect(&d, C), M2_AREA);
    }

    #[test]
    fn toggle_float_removes_and_restores() {
        let mut d = two_up();
        assert!(d.toggle_float(B, M1));
        assert!(d.is_floating(B) && !d.contains(B));
        assert!(d.monitors().iter().all(|m| m.tree.slot_of(B).is_none()));
        assert_eq!(d.placements().len(), 1);
        assert_eq!(rect(&d, A), Rect::new(0, 0, 500, 500), "hole stays");
        assert!(!d.toggle_float(B, M1));
        assert_eq!(rect(&d, B), Rect::new(500, 0, 500, 500), "back into the vacated slot");
        assert!(d.toggle_float(C, M1), "an untracked window becomes floating");
        assert!(!d.window_appeared(C, M1, None), "floating windows are not auto-tiled");
    }

    #[test]
    fn floating_windows_can_hold_focus_and_vanish() {
        let mut d = two_up();
        d.toggle_float(B, M1);
        d.focus_changed(B);
        assert_eq!(d.focused(), Some(B));
        assert!(d.window_vanished(B));
        assert_eq!(d.focused(), None);
    }

    #[test]
    fn cycle_stack_rotates_the_focused_slot() {
        let mut d = two_up();
        d.drop_window(A, Point { x: 750, y: 250 }, true); // A stacked on B, A active
        d.focus_changed(A);
        assert_eq!(d.cycle_stack(1), Some(B));
        assert_eq!(d.focused(), Some(B));
        assert_eq!(d.cycle_stack(1), Some(A));
        let mut d = desk();
        assert_eq!(d.cycle_stack(1), None, "nothing focused");
    }

    fn ident(s: &str) -> AppIdentity {
        AppIdentity(s.to_string())
    }

    #[test]
    fn reopened_app_returns_to_its_remembered_slot_before_any_other_rule() {
        let mut d = two_up(); // A | B
        d.note_identity(A, ident("notepad"));
        d.focus_changed(B);
        d.window_appeared(C, M1, Some(ident("code"))); // B splits: B | C on the right
        let a_rect = rect(&d, A);
        d.window_vanished(A); // slot remembers notepad, and is also the most recently vacated
        d.window_vanished(C); // slot remembers code and is NOW the most recently vacated
                              // A new notepad must go to A's old slot even though C's slot was vacated later.
        assert!(d.window_appeared(D, M1, Some(ident("notepad"))));
        assert_eq!(rect(&d, D), a_rect);
    }

    #[test]
    fn without_a_remembered_match_the_vacated_rule_still_applies() {
        let mut d = two_up();
        d.note_identity(B, ident("code"));
        let b_rect = rect(&d, B);
        d.window_vanished(B);
        assert!(d.window_appeared(C, M1, Some(ident("chrome"))));
        assert_eq!(rect(&d, C), b_rect, "no slot remembers chrome; most recently vacated wins");
        assert_eq!(d.monitors()[0].tree.remembered(d.monitors()[0].tree.slot_of(C).unwrap()), None);
    }

    #[test]
    fn first_window_of_an_app_wins_the_remembered_slot_and_the_second_places_normally() {
        let mut d = two_up();
        d.note_identity(A, ident("code"));
        let a_rect = rect(&d, A);
        d.window_vanished(A);
        d.focus_changed(B);
        assert!(d.window_appeared(C, M1, Some(ident("code"))));
        assert_eq!(rect(&d, C), a_rect);
        assert!(d.window_appeared(D, M1, Some(ident("code"))));
        assert_ne!(rect(&d, D), a_rect, "second code window splits the focused slot");
        assert_eq!(rect(&d, B).w, 250, "B was split");
    }

    #[test]
    fn vanishing_without_a_known_identity_leaves_no_tag() {
        let mut d = two_up();
        d.window_vanished(B);
        let t = &d.monitors()[0].tree;
        let empty = t.most_recently_vacated_empty().unwrap();
        assert_eq!(t.remembered(empty), None);
        assert_eq!(d.identity_of(B), None);
    }

    #[test]
    fn floating_and_dragging_away_also_remember() {
        let mut d = two_up();
        d.note_identity(B, ident("code"));
        d.toggle_float(B, M1);
        let t = &d.monitors()[0].tree;
        let empty = t.most_recently_vacated_empty().unwrap();
        assert_eq!(t.remembered(empty), Some(&ident("code")), "float vacates like a close");
        let mut d = two_up();
        d.note_identity(A, ident("notepad"));
        d.drop_window(A, Point { x: 750, y: 480 }, false); // A under B, A's old slot empty
        let t = &d.monitors()[0].tree;
        let empty = t.most_recently_vacated_empty().unwrap();
        assert_eq!(t.remembered(empty), Some(&ident("notepad")));
    }

    #[test]
    fn identity_is_forgotten_when_the_window_vanishes() {
        let mut d = two_up();
        d.note_identity(A, ident("notepad"));
        assert_eq!(d.identity_of(A), Some(&ident("notepad")));
        d.window_vanished(A);
        assert_eq!(d.identity_of(A), None);
    }
}
