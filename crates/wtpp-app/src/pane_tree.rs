//! The split layout of one tab: a binary tree whose leaves are panes.

pub type PaneId = u64;

pub use wtpp_config::{Axis, Direction};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The gap between the two sides of a split, for dragging with the mouse.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    /// Child choices (false = first, true = second) from the root to the split.
    pub path: Vec<bool>,
    pub axis: Axis,
    /// The gap itself.
    pub rect: Rect,
    /// The whole area the split divides.
    pub area: Rect,
}

#[derive(Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split { axis: Axis, ratio: f32, first: Box<Node>, second: Box<Node> },
}

impl Node {
    /// Split pane `target`, putting `new` after it (right of or below), or
    /// before it when `new_first`.
    pub fn split(&mut self, target: PaneId, new: PaneId, axis: Axis, new_first: bool) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                let (a, b) = if new_first { (new, target) } else { (target, new) };
                *self = Node::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Node::Leaf(a)),
                    second: Box::new(Node::Leaf(b)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                first.split(target, new, axis, new_first) || second.split(target, new, axis, new_first)
            }
        }
    }

    /// Remove pane `target`; its sibling takes over the space. Removing the
    /// root leaf is the caller's job (it means closing the tab).
    pub fn remove(&mut self, target: PaneId) -> bool {
        let Node::Split { first, second, .. } = self else { return false };
        let survivor = if **first == Node::Leaf(target) {
            std::mem::replace(second.as_mut(), Node::Leaf(0))
        } else if **second == Node::Leaf(target) {
            std::mem::replace(first.as_mut(), Node::Leaf(0))
        } else {
            return first.remove(target) || second.remove(target);
        };
        *self = survivor;
        true
    }

    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    pub fn contains(&self, target: PaneId) -> bool {
        match self {
            Node::Leaf(id) => *id == target,
            Node::Split { first, second, .. } => first.contains(target) || second.contains(target),
        }
    }

    /// Pane rectangles inside `area`, with `gap` between siblings. Sizes are
    /// rounded to whole pixels so panes don't render blurry.
    pub fn layout(&self, area: Rect, gap: f32) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.layout_into(area, gap, &mut out);
        out
    }

    fn layout_into(&self, r: Rect, gap: f32, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Node::Leaf(id) => out.push((*id, r)),
            Node::Split { axis, ratio, first, second } => {
                let (a, b) = match axis {
                    Axis::Horizontal => {
                        let w1 = ((r.w - gap) * ratio).round();
                        (
                            Rect { w: w1, ..r },
                            Rect { x: r.x + w1 + gap, w: (r.w - w1 - gap).max(0.0), ..r },
                        )
                    }
                    Axis::Vertical => {
                        let h1 = ((r.h - gap) * ratio).round();
                        (
                            Rect { h: h1, ..r },
                            Rect { y: r.y + h1 + gap, h: (r.h - h1 - gap).max(0.0), ..r },
                        )
                    }
                };
                first.layout_into(a, gap, out);
                second.layout_into(b, gap, out);
            }
        }
    }

    /// Every split's divider in `area`.
    pub fn dividers(&self, area: Rect, gap: f32) -> Vec<Divider> {
        let mut out = Vec::new();
        self.dividers_into(area, gap, &mut Vec::new(), &mut out);
        out
    }

    fn dividers_into(&self, r: Rect, gap: f32, path: &mut Vec<bool>, out: &mut Vec<Divider>) {
        let Node::Split { axis, ratio, first, second } = self else { return };
        let (a, divider, b) = match axis {
            Axis::Horizontal => {
                let w1 = ((r.w - gap) * ratio).round();
                (
                    Rect { w: w1, ..r },
                    Rect { x: r.x + w1, w: gap, ..r },
                    Rect { x: r.x + w1 + gap, w: (r.w - w1 - gap).max(0.0), ..r },
                )
            }
            Axis::Vertical => {
                let h1 = ((r.h - gap) * ratio).round();
                (
                    Rect { h: h1, ..r },
                    Rect { y: r.y + h1, h: gap, ..r },
                    Rect { y: r.y + h1 + gap, h: (r.h - h1 - gap).max(0.0), ..r },
                )
            }
        };
        out.push(Divider { path: path.clone(), axis: *axis, rect: divider, area: r });
        path.push(false);
        first.dividers_into(a, gap, path, out);
        path.pop();
        path.push(true);
        second.dividers_into(b, gap, path, out);
        path.pop();
    }

    /// Exchange the places of panes `a` and `b`.
    pub fn swap(&mut self, a: PaneId, b: PaneId) {
        match self {
            Node::Leaf(id) if *id == a => *id = b,
            Node::Leaf(id) if *id == b => *id = a,
            Node::Leaf(_) => {}
            Node::Split { first, second, .. } => {
                first.swap(a, b);
                second.swap(a, b);
            }
        }
    }

    /// Set the ratio of the split at `path` (clamped so neither side vanishes).
    pub fn set_ratio(&mut self, path: &[bool], new_ratio: f32) {
        let Node::Split { ratio, first, second, .. } = self else { return };
        match path.split_first() {
            None => *ratio = new_ratio.clamp(0.1, 0.9),
            Some((false, rest)) => first.set_ratio(rest, new_ratio),
            Some((true, rest)) => second.set_ratio(rest, new_ratio),
        }
    }

    /// Move the divider nearest to `target` along `dir` by `delta` (a ratio).
    pub fn resize(&mut self, target: PaneId, dir: Direction, delta: f32) -> bool {
        let Node::Split { axis, ratio, first, second } = self else { return false };
        let child = if first.contains(target) {
            first
        } else if second.contains(target) {
            second
        } else {
            return false;
        };
        // Prefer the deepest matching divider, like WT.
        if child.resize(target, dir, delta) {
            return true;
        }
        if *axis != dir.axis() {
            return false;
        }
        let sign = if matches!(dir, Direction::Right | Direction::Down) { 1.0 } else { -1.0 };
        *ratio = (*ratio + sign * delta).clamp(0.1, 0.9);
        true
    }
}

/// The pane next to `from` in direction `dir`: among panes on that side that
/// overlap `from` on the other axis, the closest one.
pub fn neighbor(layout: &[(PaneId, Rect)], from: PaneId, dir: Direction) -> Option<PaneId> {
    let (_, r) = layout.iter().find(|(id, _)| *id == from)?;
    let overlaps = |a0: f32, a1: f32, b0: f32, b1: f32| a0 < b1 && b0 < a1;
    layout
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|(id, o)| {
            let distance = match dir {
                Direction::Left if overlaps(r.y, r.y + r.h, o.y, o.y + o.h) => r.x - (o.x + o.w),
                Direction::Right if overlaps(r.y, r.y + r.h, o.y, o.y + o.h) => o.x - (r.x + r.w),
                Direction::Up if overlaps(r.x, r.x + r.w, o.x, o.x + o.w) => r.y - (o.y + o.h),
                Direction::Down if overlaps(r.x, r.x + r.w, o.x, o.x + o.w) => o.y - (r.y + r.h),
                _ => return None,
            };
            (distance >= 0.0).then_some((*id, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 };

    /// [1 | [2 / 3]]
    fn three_panes() -> Node {
        let mut tree = Node::Leaf(1);
        assert!(tree.split(1, 2, Axis::Horizontal, false));
        assert!(tree.split(2, 3, Axis::Vertical, false));
        tree
    }

    #[test]
    fn split_and_layout() {
        let layout = three_panes().layout(AREA, 0.0);
        assert_eq!(layout[0], (1, Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 }));
        assert_eq!(layout[1], (2, Rect { x: 50.0, y: 0.0, w: 50.0, h: 25.0 }));
        assert_eq!(layout[2], (3, Rect { x: 50.0, y: 25.0, w: 50.0, h: 25.0 }));
    }

    #[test]
    fn layout_leaves_gaps() {
        let mut tree = Node::Leaf(1);
        tree.split(1, 2, Axis::Horizontal, false);
        let layout = tree.layout(AREA, 2.0);
        assert_eq!(layout[0].1.w, 49.0);
        assert_eq!(layout[1].1.x, 51.0);
        assert_eq!(layout[1].1.w, 49.0);
    }

    #[test]
    fn dividers_and_ratios() {
        let mut tree = three_panes();
        let dividers = tree.dividers(AREA, 0.0);
        assert_eq!(dividers.len(), 2);
        assert_eq!(dividers[0].path, Vec::<bool>::new());
        assert_eq!(dividers[0].rect.x, 50.0);
        assert_eq!(dividers[1].path, vec![true]);
        assert_eq!(dividers[1].rect.y, 25.0);

        tree.set_ratio(&[true], 0.8);
        assert_eq!(tree.layout(AREA, 0.0)[1].1.h, 40.0);
        tree.set_ratio(&[], 0.0);
        assert_eq!(tree.layout(AREA, 0.0)[0].1.w, 10.0);
    }

    #[test]
    fn swap_panes() {
        let mut tree = three_panes();
        tree.swap(1, 3);
        assert_eq!(tree.panes(), [3, 2, 1]);
    }

    #[test]
    fn split_before() {
        let mut tree = Node::Leaf(1);
        tree.split(1, 2, Axis::Vertical, true);
        assert_eq!(tree.panes(), [2, 1]);
    }

    #[test]
    fn remove_promotes_sibling() {
        let mut tree = three_panes();
        assert!(tree.remove(2));
        assert_eq!(tree.panes(), [1, 3]);
        assert!(tree.remove(1));
        assert_eq!(tree, Node::Leaf(3));
        assert!(!tree.remove(3));
    }

    #[test]
    fn neighbors() {
        let layout = three_panes().layout(AREA, 0.0);
        assert_eq!(neighbor(&layout, 1, Direction::Right), Some(2));
        assert_eq!(neighbor(&layout, 3, Direction::Up), Some(2));
        assert_eq!(neighbor(&layout, 3, Direction::Left), Some(1));
        assert_eq!(neighbor(&layout, 1, Direction::Left), None);
    }

    #[test]
    fn resize_moves_nearest_matching_divider() {
        let mut tree = three_panes();
        // Pane 3 is in a vertical split; moving left must hit the outer one.
        assert!(tree.resize(3, Direction::Left, 0.1));
        let layout = tree.layout(AREA, 0.0);
        assert_eq!(layout[0].1.w, 40.0);
        assert!(tree.resize(3, Direction::Up, 0.1));
        assert_eq!(tree.layout(AREA, 0.0)[1].1.h, 20.0);
        assert!(!Node::Leaf(1).resize(1, Direction::Left, 0.1));
    }
}
