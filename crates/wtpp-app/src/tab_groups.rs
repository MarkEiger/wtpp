//! Tab groups, like a browser's: a title and colour shared by tabs that sit
//! together in the strip, and that collapse into their group's chip.
//!
//! Pure logic only (no Slint), so the ordering rules can be unit-tested; the
//! App side is in tabs.rs.

use wtpp_config::Color;

pub type GroupId = u64;

pub fn alloc_group_id() -> GroupId {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct TabGroup {
    pub id: GroupId,
    /// Empty: the chip is just a dot of the group's colour.
    pub title: String,
    pub color: Color,
    /// Its tabs are hidden behind the chip (except the active tab, which
    /// never is: activating a tab expands its group).
    pub collapsed: bool,
    pub renaming: bool,
}

impl TabGroup {
    pub fn new(color: Color) -> TabGroup {
        TabGroup { id: alloc_group_id(), title: String::new(), color, collapsed: false, renaming: false }
    }
}

/// The colours a group can have: the tab colour presets plus grey.
pub const GROUP_COLORS: [(&str, Color); 9] = [
    ("Grey", Color::hex(0x7A7574)),
    ("Blue", Color::hex(0x0063B1)),
    ("Red", Color::hex(0xC50F1F)),
    ("Yellow", Color::hex(0xC19C00)),
    ("Green", Color::hex(0x13A10E)),
    ("Pink", Color::hex(0xC239B3)),
    ("Purple", Color::hex(0x881798)),
    ("Teal", Color::hex(0x038387)),
    ("Orange", Color::hex(0xCA5010)),
];

/// A colour for a new group: the first one no group uses yet, so groups
/// look different; with all of them taken, cycle through again.
pub fn next_color(used: &[Color]) -> Color {
    GROUP_COLORS
        .iter()
        .map(|(_, c)| *c)
        .find(|c| !used.contains(c))
        .unwrap_or(GROUP_COLORS[used.len() % GROUP_COLORS.len()].1)
}

/// A stable reordering (new position → old index) that pulls the tabs of
/// each group to where its first tab is, so every group sits together.
/// The identity when they already do.
pub fn contiguous_order(groups: &[Option<GroupId>]) -> Vec<usize> {
    let mut order = Vec::with_capacity(groups.len());
    let mut placed = vec![false; groups.len()];
    for i in 0..groups.len() {
        if placed[i] {
            continue;
        }
        match groups[i] {
            None => {
                order.push(i);
                placed[i] = true;
            }
            Some(g) => {
                for j in i..groups.len() {
                    if groups[j] == Some(g) {
                        order.push(j);
                        placed[j] = true;
                    }
                }
            }
        }
    }
    order
}

/// The group a tab of group `own` should be in after landing at `at` (in a
/// list that already holds it there). Dropped between two tabs of a group,
/// it joins that group; next to its own group (or alone in it, the group
/// travelling with it) it stays; anywhere else it leaves.
pub fn group_at_insert(groups: &[Option<GroupId>], at: usize, own: Option<GroupId>) -> Option<GroupId> {
    let left = at.checked_sub(1).and_then(|i| groups.get(i).copied().flatten());
    let right = groups.get(at + 1).copied().flatten();
    if left.is_some() && left == right {
        return left;
    }
    let others_in_own = groups.iter().enumerate().any(|(i, g)| i != at && own.is_some() && *g == own);
    if own.is_some() && (own == left || own == right || !others_in_own) {
        return own;
    }
    None
}

/// How far past its group's edge a dragged tab's pointer goes before the
/// tab leaves the group.
pub const DETACH_DISTANCE: f32 = 16.0;

/// A tab of group `origin`, dragged at its group's edge (the group on one
/// side of it only): whether the pointer at strip x `x` has pulled it out
/// (`Some(None)`) or it's still in (`Some(Some(origin))`). Measured from
/// the neighbouring tab of the group, which doesn't move when the tab
/// leaves, so going in and out doesn't flicker. `None` where this doesn't
/// apply (inside the group, or nowhere near it): the usual rules do.
pub fn edge_group(
    groups: &[Option<GroupId>],
    extents: &[(f32, f32)],
    at: usize,
    origin: GroupId,
    x: f32,
    gap: f32,
) -> Option<Option<GroupId>> {
    let left = at.checked_sub(1).and_then(|i| groups.get(i).copied().flatten()) == Some(origin);
    let right = groups.get(at + 1).copied().flatten() == Some(origin);
    let width = extents.get(at)?.1;
    let out = match (left, right) {
        // First in the group (or just left of it): out past its left edge,
        // over the group's chip.
        (false, true) => x < extents.get(at + 1)?.0 - gap - width - DETACH_DISTANCE,
        // Last in the group (or just right of it): out past its right edge.
        (true, false) => {
            let (start, w) = *extents.get(at - 1)?;
            x > start + w + gap + width + DETACH_DISTANCE
        }
        _ => return None,
    };
    Some((!out).then_some(origin))
}

/// Whether a tab of group `gid` sits right next to the tab at `at`.
pub fn next_to(groups: &[Option<GroupId>], at: usize, gid: GroupId) -> bool {
    let left = at.checked_sub(1).and_then(|i| groups.get(i).copied().flatten());
    let right = groups.get(at + 1).copied().flatten();
    left == Some(gid) || right == Some(gid)
}

/// Where the tab at `i` ends up when the tab at `from` moves to `to`.
pub fn moved_index(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        to
    } else if from < i && i <= to {
        i - 1
    } else if to <= i && i < from {
        i + 1
    } else {
        i
    }
}

/// A reordering (new position → old index) turned around: old index → new
/// position.
pub fn inverse(order: &[usize]) -> Vec<usize> {
    let mut new_index = vec![0; order.len()];
    for (k, &i) in order.iter().enumerate() {
        new_index[i] = k;
    }
    new_index
}

/// Where tab `index` goes on leaving group `gid` (already taken out of it in
/// `groups`): past the group's last other tab, so the rest stay together.
pub fn leave_target(groups: &[Option<GroupId>], index: usize, gid: GroupId) -> usize {
    groups.iter().rposition(|&g| g == Some(gid)).filter(|&last| last > index).unwrap_or(index)
}

/// Where tab `at` goes on joining group `gid`: just after the group's last
/// tab (as an index after the move). `None` if the group has no tabs.
pub fn join_target(groups: &[Option<GroupId>], at: usize, gid: GroupId) -> Option<usize> {
    let last = groups.iter().rposition(|&g| g == Some(gid))?;
    Some(if at < last { last } else { last + 1 })
}

/// The tab next to `active` (right if `forward`, wrapping), over hidden
/// ones; if those are all there is, the plain next one (which opens its
/// group). `len` must be at least 2.
pub fn step_visible(hidden: &[bool], active: usize, forward: bool) -> usize {
    let n = hidden.len();
    let step = |i: usize| if forward { (i + 1) % n } else { (i + n - 1) % n };
    let mut next = step(active);
    while hidden[next] && next != active {
        next = step(next);
    }
    if next == active { step(active) } else { next }
}

/// The active tab once tab `closed` is gone (`hidden` is for the tabs
/// left): the same tab if another closed, else the next shown one, or the
/// one before at the end, skipping collapsed groups rather than opening one.
pub fn active_after_close(hidden: &[bool], active: usize, closed: usize) -> usize {
    if active > closed {
        return active - 1;
    }
    if active < closed {
        return active;
    }
    let from = closed.min(hidden.len().saturating_sub(1));
    nearest_visible(hidden, from).unwrap_or(from)
}

/// Which tabs are hidden: those in collapsed groups, except the active one.
pub fn hidden(groups: &[Option<GroupId>], collapsed: impl Fn(GroupId) -> bool, active: usize) -> Vec<bool> {
    groups.iter().enumerate().map(|(i, g)| i != active && g.is_some_and(&collapsed)).collect()
}

/// The first shown tab at or right of `from`, else the nearest one left of
/// it.
pub fn nearest_visible(hidden: &[bool], from: usize) -> Option<usize> {
    (from..hidden.len()).find(|&i| !hidden[i]).or_else(|| (0..from.min(hidden.len())).rev().find(|&i| !hidden[i]))
}

/// Where a tab dropped at strip x `x` goes: the number of tabs (start and
/// width in the strip) whose middle is left of it. Hidden tabs have no
/// width and sit at the end of their group's chip, so a collapsed group is
/// passed over as a whole.
pub fn slot_at(extents: &[(f32, f32)], x: f32) -> usize {
    extents.iter().filter(|(start, width)| start + width / 2.0 < x).count()
}

/// Collapsing group `gid` while tab `active` is in it: the tab to show
/// instead, the nearest one that stays visible. `None` if there's none (the
/// group then stays open).
pub fn collapse_target(
    groups: &[Option<GroupId>],
    gid: GroupId,
    collapsed: impl Fn(GroupId) -> bool,
    active: usize,
) -> Option<usize> {
    if groups.get(active).copied().flatten() != Some(gid) {
        return Some(active);
    }
    let hidden = hidden(groups, |g| g == gid || collapsed(g), usize::MAX);
    nearest_visible(&hidden, active)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Option<GroupId> = Some(1);
    const B: Option<GroupId> = Some(2);
    const N: Option<GroupId> = None;

    #[test]
    fn dragging_out_over_the_edge() {
        // Tabs 100 wide, 2 apart: [A 0..100] [A 102..202] [N 204..304].
        let extents = [(0.0, 100.0), (102.0, 100.0), (204.0, 100.0)];
        let groups = [A, A, N];
        // The last tab of A, dragged right: in until 16 past its own edge.
        assert_eq!(edge_group(&groups, &extents, 1, 1, 210.0, 2.0), Some(A));
        assert_eq!(edge_group(&groups, &extents, 1, 1, 220.0, 2.0), Some(N));
        // Out already (A's tab is still on its left): the same boundary
        // brings it back in.
        assert_eq!(edge_group(&[A, N, N], &extents, 1, 1, 210.0, 2.0), Some(A));
        // The first tab of A, dragged left over the chip.
        let extents = [(30.0, 100.0), (132.0, 100.0)];
        assert_eq!(edge_group(&[A, A], &extents, 0, 1, 20.0, 2.0), Some(A));
        assert_eq!(edge_group(&[A, A], &extents, 0, 1, 10.0, 2.0), Some(N));
        // Out on the left, the chip moved right of it: same boundary.
        let extents = [(0.0, 100.0), (132.0, 100.0)];
        assert_eq!(edge_group(&[N, A], &extents, 0, 1, 10.0, 2.0), Some(N));
        assert_eq!(edge_group(&[N, A], &extents, 0, 1, 50.0, 2.0), Some(A));
        // In the middle of its group, or away from it: not an edge.
        assert_eq!(edge_group(&[A, A, A], &[(0.0, 100.0); 3], 1, 1, 500.0, 2.0), None);
        assert_eq!(edge_group(&[A, N, N], &[(0.0, 100.0); 3], 2, 1, 500.0, 2.0), None);
        // Alone in its group: the group goes with it.
        assert_eq!(edge_group(&[N, A, N], &[(0.0, 100.0); 3], 1, 1, 500.0, 2.0), None);
    }

    fn apply<T: Copy>(order: &[usize], v: &[T]) -> Vec<T> {
        order.iter().map(|&i| v[i]).collect()
    }

    #[test]
    fn contiguity() {
        let together = [N, A, A, B, N];
        assert_eq!(contiguous_order(&together), vec![0, 1, 2, 3, 4]);
        let split = [A, N, B, A, B];
        assert_eq!(apply(&contiguous_order(&split), &split), vec![A, A, N, B, B]);
        assert_eq!(contiguous_order(&split), vec![0, 3, 1, 2, 4]);
        assert!(contiguous_order(&[]).is_empty());
    }

    #[test]
    fn join_leave_stay() {
        // Dropped between two tabs of a group: joins it.
        assert_eq!(group_at_insert(&[A, N, A], 1, None), A);
        assert_eq!(group_at_insert(&[A, B, A], 1, Some(2)), A);
        // At its group's edge: stays.
        assert_eq!(group_at_insert(&[A, A, N], 1, A), A);
        assert_eq!(group_at_insert(&[N, A, A], 1, A), A);
        // Next to a group it isn't in: doesn't join.
        assert_eq!(group_at_insert(&[A, N, N], 1, None), None);
        assert_eq!(group_at_insert(&[N, N, A], 1, None), None);
        // Dragged out, away from its group: leaves.
        assert_eq!(group_at_insert(&[A, N, N, A], 3, A), None);
        assert_eq!(group_at_insert(&[A, A, N], 0, A), A);
        assert_eq!(group_at_insert(&[A, A, N, N], 3, A), None);
        // The only tab of its group keeps it wherever it goes.
        assert_eq!(group_at_insert(&[N, N, A], 2, A), A);
        // ...unless dropped inside another group.
        assert_eq!(group_at_insert(&[B, A, B], 1, A), B);
        // At the ends of the strip.
        assert_eq!(group_at_insert(&[N], 0, None), None);
        assert_eq!(group_at_insert(&[N, A, B], 2, B), B);
    }

    #[test]
    fn hidden_never_active() {
        let groups = [N, A, A, B];
        assert_eq!(hidden(&groups, |g| g == 1, 0), vec![false, true, true, false]);
        assert_eq!(hidden(&groups, |g| g == 1, 2), vec![false, true, false, false]);
        assert_eq!(hidden(&groups, |_| false, 1), vec![false; 4]);
    }

    #[test]
    fn nearest() {
        let hidden = [false, true, true, false];
        assert_eq!(nearest_visible(&hidden, 0), Some(0));
        assert_eq!(nearest_visible(&hidden, 1), Some(3));
        assert_eq!(nearest_visible(&[false, true, true], 1), Some(0));
        assert_eq!(nearest_visible(&[false, true, true], 3), Some(0));
        assert_eq!(nearest_visible(&[true, true], 0), None);
        assert_eq!(nearest_visible(&[], 0), None);
    }

    #[test]
    fn slots() {
        let even = [(0.0, 100.0), (102.0, 100.0), (204.0, 100.0)];
        assert_eq!(slot_at(&even, -5.0), 0);
        assert_eq!(slot_at(&even, 49.0), 0);
        assert_eq!(slot_at(&even, 51.0), 1);
        assert_eq!(slot_at(&even, 160.0), 2);
        assert_eq!(slot_at(&even, 1000.0), 3);
        // Tab 0, then a collapsed group of two behind a chip ending at 160,
        // then tab 3: the group goes before or after as a whole.
        let collapsed = [(0.0, 100.0), (160.0, 0.0), (160.0, 0.0), (162.0, 80.0)];
        assert_eq!(slot_at(&collapsed, 120.0), 1);
        assert_eq!(slot_at(&collapsed, 165.0), 3);
        assert_eq!(slot_at(&collapsed, 250.0), 4);
    }

    #[test]
    fn colors_cycle() {
        assert_eq!(next_color(&[]), GROUP_COLORS[0].1);
        assert_eq!(next_color(&[GROUP_COLORS[0].1]), GROUP_COLORS[1].1);
        assert_eq!(next_color(&[GROUP_COLORS[1].1]), GROUP_COLORS[0].1);
        let all: Vec<Color> = GROUP_COLORS.iter().map(|(_, c)| *c).collect();
        assert_eq!(next_color(&all), GROUP_COLORS[0].1);
        let more: Vec<Color> = all.iter().chain(&all[..2]).copied().collect();
        assert_eq!(next_color(&more), GROUP_COLORS[2].1);
    }

    #[test]
    fn collapsing_the_active_tabs_group() {
        let groups = [N, A, A, N];
        // Moves to the next tab outside the group, else the one before.
        assert_eq!(collapse_target(&groups, 1, |_| false, 1), Some(3));
        assert_eq!(collapse_target(&[N, A, A], 1, |_| false, 2), Some(0));
        // Not in the group: stays.
        assert_eq!(collapse_target(&groups, 1, |_| false, 0), Some(0));
        // Nothing else to show.
        assert_eq!(collapse_target(&[A, A], 1, |_| false, 0), None);
        assert_eq!(collapse_target(&[B, A], 1, |g| g == 2, 1), None);
    }

    #[test]
    fn moving_indices() {
        // Tab 1 moved to 3: 2 and 3 shift left, the rest stay.
        assert_eq!((0..5).map(|i| moved_index(i, 1, 3)).collect::<Vec<_>>(), vec![0, 3, 1, 2, 4]);
        // Tab 3 moved to 1: 1 and 2 shift right.
        assert_eq!((0..5).map(|i| moved_index(i, 3, 1)).collect::<Vec<_>>(), vec![0, 2, 3, 1, 4]);
        assert_eq!((0..3).map(|i| moved_index(i, 2, 2)).collect::<Vec<_>>(), vec![0, 1, 2]);
        // Agrees with a real remove and insert.
        for from in 0..5 {
            for to in 0..5 {
                let mut v: Vec<usize> = (0..5).collect();
                let t = v.remove(from);
                v.insert(to, t);
                for i in 0..5 {
                    assert_eq!(v[moved_index(i, from, to)], i);
                }
            }
        }
    }

    #[test]
    fn tidying_keeps_the_active_tab() {
        let split = [A, N, B, A, B];
        let order = contiguous_order(&split);
        let new_index = inverse(&order);
        // The second A (old 3) is now next to the first; old 1 moved right.
        assert_eq!(new_index, vec![0, 2, 3, 1, 4]);
        for (old, &new) in new_index.iter().enumerate() {
            assert_eq!(order[new], old);
        }
        assert!(inverse(&[]).is_empty());
    }

    #[test]
    fn leaving_and_joining() {
        // Leaving from the middle of a group: past its other tabs.
        assert_eq!(leave_target(&[A, N, A, A, N], 1, 1), 3);
        // Already its last tab, or the only one: stays.
        assert_eq!(leave_target(&[A, A, N, N], 2, 1), 2);
        assert_eq!(leave_target(&[N, N], 1, 1), 1);
        // Joining from the left: the group shifts left when it's taken out.
        assert_eq!(join_target(&[N, B, B, N], 0, 2), Some(2));
        // From the right: just after the group.
        assert_eq!(join_target(&[B, B, N, N], 3, 2), Some(2));
        assert_eq!(join_target(&[N, N], 0, 2), None);
        // Agrees with moving it there: the group's tabs then sit together.
        for (groups, at) in [(vec![N, B, B, N], 0), (vec![B, B, N, N], 3), (vec![N, B, B, N, N], 4), (vec![N, N, B, B], 0)] {
            let to = join_target(&groups, at, 2).unwrap();
            let mut v = groups.clone();
            v.remove(at);
            v.insert(to, B);
            assert_eq!(contiguous_order(&v), (0..v.len()).collect::<Vec<_>>(), "{groups:?} {at}");
        }
    }

    #[test]
    fn stepping_over_hidden_tabs() {
        let hidden = [false, true, true, false];
        assert_eq!(step_visible(&hidden, 0, true), 3);
        assert_eq!(step_visible(&hidden, 3, true), 0);
        assert_eq!(step_visible(&hidden, 3, false), 0);
        assert_eq!(step_visible(&hidden, 0, false), 3);
        // Nothing else shown: the next tab anyway.
        assert_eq!(step_visible(&[false, true, true], 0, true), 1);
        assert_eq!(step_visible(&[false, true, true], 0, false), 2);
        assert_eq!(step_visible(&[false, false], 1, true), 0);
    }

    #[test]
    fn closing_picks_a_shown_tab() {
        // Another tab closed: the active one keeps its place.
        assert_eq!(active_after_close(&[false; 3], 2, 0), 1);
        assert_eq!(active_after_close(&[false; 3], 0, 2), 0);
        // The active one closed: the next shown tab, past a collapsed group.
        assert_eq!(active_after_close(&[false, true, true, false], 1, 1), 3);
        // At the end: the one before, skipping hidden ones.
        assert_eq!(active_after_close(&[false, true, true], 3, 3), 0);
        // Only hidden tabs left: one of them (its group then opens).
        assert_eq!(active_after_close(&[true, true], 0, 0), 0);
    }
}
