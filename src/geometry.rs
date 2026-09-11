//! Pure geometry: rectangles, drop-zone hit testing, edge detection.
//! No Win32 types here so everything is unit-testable.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    #[serde(rename = "width", alias = "w")]
    pub w: i32,
    #[serde(rename = "height", alias = "h")]
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn area(&self) -> i64 {
        self.w as i64 * self.h as i64
    }
    /// Half-open on the right and bottom edges.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }
    /// Shrink by `by` on every side, never below zero size.
    pub fn inset(&self, by: i32) -> Rect {
        Rect::new(self.x + by, self.y + by, (self.w - 2 * by).max(0), (self.h - 2 * by).max(0))
    }
}

/// `X`: children sit side by side, `first` on the left. `Y`: stacked, `first` on top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropZone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

/// The centre zone spans the middle 70% of the slot in each dimension.
/// This makes swapping onto the full tile the default; the outer 15% bands
/// remain available for an intentional half-tile placement.
pub const CENTER_ZONE_HALF_EXTENT: f32 = 0.35;

/// Which zone of `rect` the point falls in. Points outside are projected as if
/// on the nearest edge, so callers should hit-test the slot first.
pub fn drop_zone(rect: Rect, p: Point) -> DropZone {
    if rect.w <= 0 || rect.h <= 0 {
        return DropZone::Center;
    }
    let u = (p.x - rect.x) as f32 / rect.w as f32 - 0.5;
    let v = (p.y - rect.y) as f32 / rect.h as f32 - 0.5;
    if u.abs() <= CENTER_ZONE_HALF_EXTENT && v.abs() <= CENTER_ZONE_HALF_EXTENT {
        return DropZone::Center;
    }
    if u.abs() >= v.abs() {
        if u < 0.0 {
            DropZone::Left
        } else {
            DropZone::Right
        }
    } else if v < 0.0 {
        DropZone::Top
    } else {
        DropZone::Bottom
    }
}

/// Wide slots split left/right, tall slots split top/bottom. Ties go to `X`.
pub fn longest_axis(rect: Rect) -> SplitAxis {
    if rect.w >= rect.h {
        SplitAxis::X
    } else {
        SplitAxis::Y
    }
}

/// The central 70% of each dimension selects the full empty space;
/// outer 15% bands select edge halves and corner quarters.
pub fn empty_splits(rect: Rect, p: Point) -> Vec<(SplitAxis, bool)> {
    if rect.w <= 0 || rect.h <= 0 {
        return Vec::new();
    }
    let x = (p.x - rect.x) as f64 / rect.w as f64;
    let y = (p.y - rect.y) as f64 / rect.h as f64;
    let mut splits = Vec::new();
    if !(0.15..=0.85).contains(&x) {
        splits.push((SplitAxis::X, x < 0.5));
    }
    if !(0.15..=0.85).contains(&y) {
        splits.push((SplitAxis::Y, y < 0.5));
    }
    splits
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub fn axis(self) -> SplitAxis {
        match self {
            Edge::Left | Edge::Right => SplitAxis::X,
            Edge::Top | Edge::Bottom => SplitAxis::Y,
        }
    }
}

/// Edges of `after` that differ from `before` by more than `tolerance` pixels,
/// in the fixed order Left, Right, Top, Bottom.
pub fn changed_edges(before: Rect, after: Rect, tolerance: i32) -> Vec<Edge> {
    let mut out = Vec::new();
    if (after.x - before.x).abs() > tolerance {
        out.push(Edge::Left);
    }
    if (after.right() - before.right()).abs() > tolerance {
        out.push(Edge::Right);
    }
    if (after.y - before.y).abs() > tolerance {
        out.push(Edge::Top);
    }
    if (after.bottom() - before.bottom()).abs() > tolerance {
        out.push(Edge::Bottom);
    }
    out
}

/// A native drag is a move when the size is unchanged; otherwise it is a resize.
pub fn is_pure_move(before: Rect, after: Rect, tolerance: i32) -> bool {
    (after.w - before.w).abs() <= tolerance && (after.h - before.h).abs() <= tolerance
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect::new(0, 0, 1000, 500);

    #[test]
    fn rect_edges_and_containment() {
        assert_eq!(R.right(), 1000);
        assert_eq!(R.bottom(), 500);
        assert_eq!(R.area(), 500_000);
        assert!(R.contains(Point { x: 0, y: 0 }));
        assert!(R.contains(Point { x: 999, y: 499 }));
        assert!(!R.contains(Point { x: 1000, y: 250 }), "right edge is exclusive");
        assert_eq!(R.inset(10), Rect::new(10, 10, 980, 480));
        assert_eq!(Rect::new(0, 0, 5, 5).inset(10), Rect::new(10, 10, 0, 0), "inset never goes negative");
    }

    #[test]
    fn centre_zone_is_the_middle_seventy_percent() {
        assert_eq!(drop_zone(R, Point { x: 500, y: 250 }), DropZone::Center);
        assert_eq!(drop_zone(R, Point { x: 160, y: 80 }), DropZone::Center); // u=-0.34, v=-0.34
        assert_eq!(drop_zone(R, Point { x: 840, y: 420 }), DropZone::Center);
        assert_eq!(
            drop_zone(R, Point { x: 150, y: 250 }),
            DropZone::Center,
            "exactly at the centre-zone boundary (u=-0.35)"
        );
    }

    #[test]
    fn edge_zones_pick_the_dominant_axis() {
        assert_eq!(drop_zone(R, Point { x: 140, y: 250 }), DropZone::Left);
        assert_eq!(drop_zone(R, Point { x: 860, y: 250 }), DropZone::Right);
        assert_eq!(drop_zone(R, Point { x: 500, y: 70 }), DropZone::Top);
        assert_eq!(drop_zone(R, Point { x: 500, y: 430 }), DropZone::Bottom);
        // Near the top-left corner but further from centre horizontally than vertically.
        assert_eq!(drop_zone(R, Point { x: 50, y: 100 }), DropZone::Left);
        // Exact corner: tie goes to the horizontal zone.
        assert_eq!(drop_zone(R, Point { x: 0, y: 0 }), DropZone::Left);
    }

    #[test]
    fn drop_zone_uses_the_rects_own_origin() {
        let r = Rect::new(2000, 100, 1000, 500);
        assert_eq!(drop_zone(r, Point { x: 2100, y: 350 }), DropZone::Left);
        assert_eq!(drop_zone(r, Point { x: 2500, y: 350 }), DropZone::Center);
    }

    #[test]
    fn longest_axis_prefers_x_on_ties() {
        assert_eq!(longest_axis(Rect::new(0, 0, 1000, 500)), SplitAxis::X);
        assert_eq!(longest_axis(Rect::new(0, 0, 500, 1000)), SplitAxis::Y);
        assert_eq!(longest_axis(Rect::new(0, 0, 500, 500)), SplitAxis::X);
    }
    #[test]
    fn empty_space_full_target_spans_central_seventy_percent() {
        let r = Rect::new(-500, 30, 1000, 1000);
        for x in [150, 200, 500, 800, 850] {
            for y in [150, 200, 500, 800, 850] {
                assert!(empty_splits(r, Point { x: r.x + x, y: r.y + y }).is_empty());
            }
        }
        assert_eq!(empty_splits(r, Point { x: r.x + 149, y: r.y + 500 }), vec![(SplitAxis::X, true)]);
        assert_eq!(empty_splits(r, Point { x: r.x + 851, y: r.y + 500 }), vec![(SplitAxis::X, false)]);
        assert_eq!(
            empty_splits(r, Point { x: r.x + 149, y: r.y + 149 }),
            vec![(SplitAxis::X, true), (SplitAxis::Y, true)]
        );
        assert_eq!(
            empty_splits(r, Point { x: r.x + 851, y: r.y + 851 }),
            vec![(SplitAxis::X, false), (SplitAxis::Y, false)]
        );
    }

    #[test]
    fn changed_edges_reports_only_edges_that_moved() {
        let before = Rect::new(0, 0, 100, 100);
        assert_eq!(changed_edges(before, Rect::new(10, 0, 90, 100), 2), vec![Edge::Left]);
        assert_eq!(changed_edges(before, Rect::new(0, 0, 120, 100), 2), vec![Edge::Right]);
        assert_eq!(changed_edges(before, Rect::new(0, 5, 100, 95), 2), vec![Edge::Top]);
        assert_eq!(changed_edges(before, Rect::new(0, 0, 100, 130), 2), vec![Edge::Bottom]);
        assert_eq!(changed_edges(before, Rect::new(1, 0, 99, 100), 2), Vec::<Edge>::new(), "within tolerance");
        assert_eq!(changed_edges(before, Rect::new(-10, 0, 120, 100), 2), vec![Edge::Left, Edge::Right]);
        assert_eq!(
            changed_edges(before, Rect::new(2, 0, 98, 100), 2),
            Vec::<Edge>::new(),
            "exactly at tolerance is not a change"
        );
        assert_eq!(
            changed_edges(before, Rect::new(3, 0, 97, 100), 2),
            vec![Edge::Left],
            "one past tolerance is a change"
        );
    }

    #[test]
    fn pure_move_keeps_size() {
        let before = Rect::new(0, 0, 100, 100);
        assert!(is_pure_move(before, Rect::new(50, 50, 100, 100), 2));
        assert!(is_pure_move(before, Rect::new(50, 50, 101, 99), 2));
        assert!(!is_pure_move(before, Rect::new(0, 0, 120, 100), 2));
        assert!(is_pure_move(before, Rect::new(0, 0, 102, 100), 2), "exactly at tolerance is still a pure move");
        assert!(!is_pure_move(before, Rect::new(0, 0, 104, 100), 2), "one past tolerance is a resize");
    }

    #[test]
    fn edge_axis() {
        assert_eq!(Edge::Left.axis(), SplitAxis::X);
        assert_eq!(Edge::Right.axis(), SplitAxis::X);
        assert_eq!(Edge::Top.axis(), SplitAxis::Y);
        assert_eq!(Edge::Bottom.axis(), SplitAxis::Y);
    }
}
