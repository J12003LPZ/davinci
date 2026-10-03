//! Pure terminal-cell geometry. The renderer and input share these rectangles.
//!
//! The command center lays agents out as a flow that reads like text: cards
//! wrap left to right, top to bottom, in dependency order. A worker's first
//! ready dependent is placed right after it, so a chain stays on one row and
//! reads `a → b → c`. Every other dependency is an elbow route through the gap
//! under the source row, ending in an arrow on the target card.
use crate::davinci::model::{GraphCanvasState, GraphRunSheet};
use ratatui::layout::Rect;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphResponsiveMode {
    Full,
    Adaptive,
    Compact,
    Structured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutNode {
    pub id: String,
    pub members: Vec<String>,
    pub task_index: usize,
    pub depth: usize,
    /// Reading-order grid position.
    pub row: usize,
    pub col: usize,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutEdge {
    pub from: usize,
    pub to: usize,
    /// Polyline of axis-aligned segments; the last point carries `head`.
    /// A single point is a bare arrow between neighbouring cards.
    pub points: Vec<(u16, u16)>,
    pub head: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLayout {
    pub mode: GraphResponsiveMode,
    pub nodes: Vec<LayoutNode>,
    pub edges: Vec<LayoutEdge>,
    pub canvas: Rect,
    pub inspector: Rect,
    pub content_width: u16,
    pub content_height: u16,
    pub issues: Vec<String>,
}

/// A horizontal routing track in the gap under a row, owned by one source
/// (a fork) or one target (a join). Sources sort first; both by slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Lane {
    Source(usize),
    Target(usize),
}

/// Cells between two cards on a row: `  →  `.
pub const GAP_X: u16 = 5;
/// Left margin, reserved for routes that descend past the first column.
pub const MARGIN_X: u16 = 2;
/// Rows from a card's top border to the row that carries a `→`.
pub const ARROW_ROW: u16 = 2;
const MAX_CARD_WIDTH: u16 = 40;

pub fn layout_graph(
    run: &GraphRunSheet,
    canvas: &GraphCanvasState,
    width: u16,
    height: u16,
) -> GraphLayout {
    let mode = match (width, height) {
        (0..=49, _) | (_, 0..=11) => GraphResponsiveMode::Structured,
        (100.., 16..) => GraphResponsiveMode::Full,
        (72.., _) => GraphResponsiveMode::Adaptive,
        _ => GraphResponsiveMode::Compact,
    };
    let inspector = match mode {
        GraphResponsiveMode::Full => {
            let panel_width = (width / 3).clamp(36, 72);
            Rect::new(width - panel_width, 0, panel_width, height)
        }
        GraphResponsiveMode::Adaptive | GraphResponsiveMode::Compact => {
            let drawer = (height / 3).clamp(4, 9);
            Rect::new(0, height - drawer, width, drawer)
        }
        GraphResponsiveMode::Structured => Rect::default(),
    };
    let mut layout = GraphLayout {
        mode,
        nodes: vec![],
        edges: vec![],
        canvas: match mode {
            // Two cells of air, the rule, one more before the panel text.
            GraphResponsiveMode::Full => Rect::new(0, 0, inspector.x.saturating_sub(4), height),
            GraphResponsiveMode::Adaptive | GraphResponsiveMode::Compact => {
                Rect::new(0, 0, width, inspector.y.saturating_sub(1))
            }
            GraphResponsiveMode::Structured => Rect::new(0, 0, width, height),
        },
        inspector,
        content_width: 0,
        content_height: 0,
        issues: vec![],
    };
    let ids: BTreeMap<&str, usize> = run
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    if ids.len() != run.tasks.len() {
        layout.mode = GraphResponsiveMode::Structured;
        layout
            .issues
            .push("layout unavailable: duplicate node IDs".into());
        return layout;
    }
    let mut children = vec![Vec::new(); run.tasks.len()];
    let mut pending = vec![0usize; run.tasks.len()];
    for (i, task) in run.tasks.iter().enumerate() {
        for dep in task.dependencies.iter().collect::<BTreeSet<_>>() {
            if let Some(&parent) = ids.get(dep.as_str()) {
                children[parent].push(i);
                pending[i] += 1;
            } else {
                layout
                    .issues
                    .push(format!("{}: unresolved dependency {dep}", task.id));
            }
        }
    }
    // Stable tie-break: first-seen order across snapshots, then task order,
    // so a worker appended by a refresh never displaces one already drawn.
    let order: BTreeMap<_, _> = canvas
        .node_order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let key = |i: usize| {
        (
            order
                .get(run.tasks[i].id.as_str())
                .copied()
                .unwrap_or(usize::MAX),
            i,
        )
    };
    let Some((sequence, depths)) = flow_order(&children, pending, key) else {
        layout.mode = GraphResponsiveMode::Structured;
        layout
            .issues
            .push("layout unavailable: cyclic dependencies".into());
        return layout;
    };
    let (members, owner) = fold_completed(run, canvas, &children, &depths, &sequence);

    let compact = mode == GraphResponsiveMode::Compact;
    let card_height: u16 = if compact { 5 } else { 6 };
    let min_card: u16 = if compact { 20 } else { 26 };
    let room = layout.canvas.width.saturating_sub(MARGIN_X);
    let cols = (room.saturating_add(GAP_X) / (min_card + GAP_X)).max(1) as usize;
    let card_width = (room.saturating_sub(GAP_X.saturating_mul(cols as u16 - 1)) / cols as u16)
        .clamp(min_card.min(room.max(1)), MAX_CARD_WIDTH);

    // Place representatives in flow order.
    let mut slot_of = BTreeMap::new();
    for &i in &sequence {
        if owner[i] == i {
            let slot = slot_of.len();
            slot_of.insert(i, (slot / cols, slot % cols));
        }
    }
    let rows = slot_of.values().map(|&(r, _)| r + 1).max().unwrap_or(0);
    let x_of = |col: usize| MARGIN_X as usize + col * (card_width + GAP_X) as usize;

    // Resolve every dependency between drawn cards once.
    let mut pairs = BTreeSet::new();
    for (from, next) in children.iter().enumerate() {
        for &to in next {
            let (a, b) = (owner[from], owner[to]);
            if a != b {
                pairs.insert((a, b));
            }
        }
    }
    // A horizontal lane may only be shared by routes that truly meet: a fork
    // (one source, its drops) or a join (one target, its feeders). Routes
    // with different sources AND different targets never share a lane, so no
    // lane reads as a bus that connects unrelated cards.
    let slot_number = |i: usize| {
        let (row, col) = slot_of[&i];
        row * cols + col
    };
    let adjacent = |a: usize, b: usize| {
        let ((ra, ca), (rb, cb)) = (slot_of[&a], slot_of[&b]);
        rb == ra && cb == ca + 1
    };
    let mut feeders = BTreeMap::<(usize, usize), usize>::new();
    for &(a, b) in pairs.iter().filter(|&&(a, b)| !adjacent(a, b)) {
        let (ra, rb) = (slot_of[&a].0, slot_of[&b].0);
        *feeders
            .entry((rb.saturating_sub(1).max(ra), b))
            .or_default() += 1;
    }
    // The lane each route uses in the gap under its source row, and (for a
    // route that descends further) in the gap above its target.
    let route_lanes = |a: usize, b: usize| -> (Lane, Option<Lane>) {
        let (ra, rb) = (slot_of[&a].0, slot_of[&b].0);
        if rb > ra + 1 {
            (
                Lane::Source(slot_number(a)),
                Some(Lane::Target(slot_number(b))),
            )
        } else if feeders[&(ra, b)] == 1 {
            (Lane::Source(slot_number(a)), None)
        } else {
            (Lane::Target(slot_number(b)), None)
        }
    };
    let mut lanes: Vec<BTreeSet<Lane>> = vec![BTreeSet::new(); rows];
    let mut arrives_from_above = vec![false; rows + 1];
    for &(a, b) in pairs.iter().filter(|&&(a, b)| !adjacent(a, b)) {
        let (ra, rb) = (slot_of[&a].0, slot_of[&b].0);
        let (first, last) = route_lanes(a, b);
        lanes[ra].insert(first);
        if rb > ra {
            arrives_from_above[rb] = true;
        }
        if let Some(last) = last {
            lanes[rb - 1].insert(last);
        }
    }
    // Row tops: card, stem row, lanes, arrow row.
    let mut tops = Vec::with_capacity(rows);
    let mut y = 0usize;
    for (row, row_lanes) in lanes.iter().enumerate() {
        tops.push(y);
        y += card_height as usize;
        let lane_count = row_lanes.len();
        let below = if lane_count > 0 {
            1 + lane_count + usize::from(arrives_from_above.get(row + 1) == Some(&true))
        } else {
            1
        };
        if row + 1 < rows || lane_count > 0 {
            y += below;
        }
    }
    if x_of(cols.saturating_sub(1)) + card_width as usize > u16::MAX as usize
        || y > u16::MAX as usize
    {
        layout.mode = GraphResponsiveMode::Structured;
        layout
            .issues
            .push("layout unavailable: graph exceeds cell coordinate range".into());
        return layout;
    }
    let mut index_of = BTreeMap::new();
    for &i in &sequence {
        let Some(&(row, col)) = slot_of.get(&i) else {
            continue;
        };
        let rect = Rect::new(x_of(col) as u16, tops[row] as u16, card_width, card_height);
        let (id, group) = match &members[i] {
            Some(group) => (format!("fold:{}", run.tasks[i].id), group.clone()),
            None => (run.tasks[i].id.clone(), vec![run.tasks[i].id.clone()]),
        };
        index_of.insert(i, layout.nodes.len());
        layout.content_width = layout.content_width.max(rect.right());
        layout.content_height = layout.content_height.max(rect.bottom());
        layout.nodes.push(LayoutNode {
            id,
            members: group,
            task_index: i,
            depth: depths[i],
            row,
            col,
            rect,
        });
    }
    for &(a, b) in &pairs {
        let (from, to) = (index_of[&a], index_of[&b]);
        let (source, target) = (&layout.nodes[from], &layout.nodes[to]);
        if (target.row, target.col) <= (source.row, source.col) {
            // Flow order is topological, so this cannot happen; if it ever
            // does, report it rather than route through cards.
            layout
                .issues
                .push(format!("{}: dependency drawn out of order", target.id));
            continue;
        }
        let (s, t) = (source.rect, target.rect);
        let (sx, tx) = (s.x + s.width / 2, t.x + t.width / 2);
        let edge = if target.row == source.row && target.col == source.col + 1 {
            LayoutEdge {
                from,
                to,
                points: vec![(s.right() + GAP_X / 2, s.y + ARROW_ROW)],
                head: "→",
            }
        } else {
            let (first, last) = route_lanes(a, b);
            let lane_y = |row: usize, lane: Lane| {
                let rank = lanes[row].iter().position(|&l| l == lane).unwrap_or(0);
                tops[row] as u16 + card_height + 1 + rank as u16
            };
            let stem = s.bottom();
            let lane = lane_y(source.row, first);
            let points = if target.row == source.row {
                // A later card on the same row: under the row, up into it.
                vec![(sx, stem), (sx, lane), (tx, lane), (tx, t.bottom())]
            } else if target.row == source.row + 1 {
                vec![(sx, stem), (sx, lane), (tx, lane), (tx, t.y - 1)]
            } else {
                // Descend in the gutter left of the target column, clear of
                // the `→` in the middle of the gap.
                let gutter = if target.col == 0 { 0 } else { t.x - 2 };
                let last = lane_y(target.row - 1, last.unwrap_or(first));
                vec![
                    (sx, stem),
                    (sx, lane),
                    (gutter, lane),
                    (gutter, last),
                    (tx, last),
                    (tx, t.y - 1),
                ]
            };
            LayoutEdge {
                from,
                to,
                points,
                head: if target.row == source.row {
                    "↑"
                } else {
                    "↓"
                },
            }
        };
        layout.content_height = layout
            .content_height
            .max(edge.points.iter().map(|p| p.1 + 1).max().unwrap_or(0));
        layout.edges.push(edge);
    }
    layout
}

/// Topological order that follows chains: after a worker, its dependents
/// that just became ready come next (depth first), otherwise the earliest
/// ready worker by `key`. `None` when the dependencies contain a cycle.
fn flow_order(
    children: &[Vec<usize>],
    mut pending: Vec<usize>,
    key: impl Fn(usize) -> (usize, usize),
) -> Option<(Vec<usize>, Vec<usize>)> {
    let mut depths = vec![0usize; children.len()];
    let mut roots: Vec<_> = (0..children.len()).filter(|&i| pending[i] == 0).collect();
    roots.sort_by_key(|&i| std::cmp::Reverse(key(i)));
    let mut stack = roots;
    let mut sequence = Vec::with_capacity(children.len());
    while let Some(i) = stack.pop() {
        sequence.push(i);
        let mut ready = Vec::new();
        for &child in &children[i] {
            depths[child] = depths[child].max(depths[i] + 1);
            pending[child] -= 1;
            if pending[child] == 0 {
                ready.push(child);
            }
        }
        ready.sort_by_key(|&c| std::cmp::Reverse(key(c)));
        stack.extend(ready);
    }
    (sequence.len() == children.len()).then_some((sequence, depths))
}

type Folds = (Vec<Option<Vec<String>>>, Vec<usize>);

/// Quiet completed siblings (same role, phase, depth, and identical incoming
/// AND outgoing frontiers) share one card until expanded, so a summary never
/// invents connectivity between a member and another member's dependent.
/// Returns each representative's member ids and every task's representative.
fn fold_completed(
    run: &GraphRunSheet,
    canvas: &GraphCanvasState,
    children: &[Vec<usize>],
    depths: &[usize],
    sequence: &[usize],
) -> Folds {
    use crate::davinci::theme::State;
    let mut members = vec![None; run.tasks.len()];
    let mut owner: Vec<usize> = (0..run.tasks.len()).collect();
    let real_ids: BTreeSet<_> = run.tasks.iter().map(|t| t.id.as_str()).collect();
    let mut groups = BTreeMap::<_, Vec<usize>>::new();
    for &i in sequence {
        let task = &run.tasks[i];
        if task.state != State::Done
            || task.error.is_some()
            || task.public_contract.is_some()
            || task.role.is_empty()
            || task.role.contains("verif")
            || task.role.contains("review")
            || matches!(task.phase.as_str(), "verify" | "review")
            || task
                .dependencies
                .iter()
                .any(|id| !real_ids.contains(id.as_str()))
            || run.selected_node_id.as_deref() == Some(&task.id)
        {
            continue;
        }
        let incoming: BTreeSet<_> = task.dependencies.iter().collect();
        let outgoing: BTreeSet<_> = children[i].iter().collect();
        groups
            .entry((depths[i], &task.role, &task.phase, incoming, outgoing))
            .or_default()
            .push(i);
    }
    for group in groups.values().filter(|g| g.len() >= 3) {
        let first = group[0];
        let id = format!("fold:{}", run.tasks[first].id);
        if canvas.expanded_groups.contains(&id) || real_ids.contains(id.as_str()) {
            continue;
        }
        for &member in group {
            owner[member] = first;
        }
        members[first] = Some(group.iter().map(|&i| run.tasks[i].id.clone()).collect());
    }
    (members, owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::GraphTask;

    pub fn diamond() -> GraphRunSheet {
        chain(&[
            ("start", &[]),
            ("left", &["start"]),
            ("right", &["start"]),
            ("join", &["left", "right"]),
        ])
    }

    fn chain(specs: &[(&str, &[&str])]) -> GraphRunSheet {
        GraphRunSheet {
            tasks: specs
                .iter()
                .map(|(id, deps)| GraphTask {
                    id: (*id).into(),
                    dependencies: deps.iter().map(|d| (*d).to_owned()).collect(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn node<'a>(layout: &'a GraphLayout, id: &str) -> &'a LayoutNode {
        layout.nodes.iter().find(|n| n.id == id).unwrap()
    }

    #[test]
    fn graph_layout_deterministic_dependency_order_and_selection() {
        let run = GraphRunSheet {
            selected_node_id: Some("right".into()),
            ..diamond()
        };
        let canvas = GraphCanvasState::default();
        let layout = layout_graph(&run, &canvas, 120, 30);
        assert_eq!(layout.nodes.len(), 4);
        assert_eq!(layout.edges.len(), 4);
        assert_eq!(layout, layout_graph(&run, &canvas, 120, 30));
        // Reading order respects every dependency.
        for edge in &layout.edges {
            let (a, b) = (&layout.nodes[edge.from], &layout.nodes[edge.to]);
            assert!((a.row, a.col) < (b.row, b.col), "{} before {}", a.id, b.id);
        }
    }

    #[test]
    fn graph_layout_mockup_flow_wraps_chains_and_routes_joins() {
        // The command-center reference: t1 → t2 → t3 on the first row,
        // t2 and t3 join into t4, which starts the second row.
        let run = chain(&[
            ("t1", &[]),
            ("t2", &["t1"]),
            ("t3", &["t2"]),
            ("t4", &["t2", "t3"]),
            ("t5", &["t4"]),
            ("t6", &["t5"]),
            ("t7", &["t4"]),
        ]);
        let layout = layout_graph(&run, &GraphCanvasState::default(), 160, 40);
        let slots: Vec<_> = ["t1", "t2", "t3", "t4", "t5", "t6", "t7"]
            .iter()
            .map(|id| (node(&layout, id).row, node(&layout, id).col))
            .collect();
        assert_eq!(
            slots,
            vec![(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (2, 0)]
        );
        let edge = |a: &str, b: &str| {
            layout
                .edges
                .iter()
                .find(|e| layout.nodes[e.from].id == a && layout.nodes[e.to].id == b)
                .unwrap()
        };
        // Neighbours on a row: a bare arrow centred in the gap.
        let arrow = edge("t1", "t2");
        assert_eq!(arrow.head, "→");
        assert_eq!(arrow.points.len(), 1);
        let (t1, t2) = (node(&layout, "t1").rect, node(&layout, "t2").rect);
        assert!(arrow.points[0].0 > t1.right() && arrow.points[0].0 < t2.x);
        // Both joins into t4 share its lane and land on its top border.
        let (a, b) = (edge("t2", "t4"), edge("t3", "t4"));
        assert_eq!(a.head, "↓");
        assert_eq!(a.points.last(), b.points.last());
        assert_eq!(a.points[1].1, b.points[1].1);
        let t4 = node(&layout, "t4").rect;
        assert_eq!(a.points.last().unwrap().1 + 1, t4.y);
        // t4 → t7 drops a row; t4 → t5 stays a neighbour.
        assert_eq!(edge("t4", "t5").head, "→");
        assert_eq!(edge("t4", "t7").head, "↓");
    }

    #[test]
    fn graph_layout_distinct_destinations_take_distinct_lanes() {
        let run = chain(&[
            ("a", &[]),
            ("b", &[]),
            ("c", &[]),
            ("x", &["a", "c"]),
            ("y", &["b", "c"]),
        ]);
        let layout = layout_graph(&run, &GraphCanvasState::default(), 160, 40);
        let lane_of = |id: &str| {
            layout
                .edges
                .iter()
                .filter(|e| layout.nodes[e.to].id == id && e.points.len() > 1)
                .map(|e| e.points[1].1)
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(lane_of("x").len(), 1);
        assert!(lane_of("x").is_disjoint(&lane_of("y")));
    }

    #[test]
    fn graph_layout_long_edges_descend_in_a_gutter_clear_of_cards() {
        let mut specs: Vec<(String, Vec<String>)> = vec![("root".into(), vec![])];
        for i in 0..6 {
            specs.push((
                format!("n{i}"),
                vec![if i == 0 {
                    "root".into()
                } else {
                    format!("n{}", i - 1)
                }],
            ));
        }
        specs.push(("late".into(), vec!["root".into(), "n5".into()]));
        let run = GraphRunSheet {
            tasks: specs
                .into_iter()
                .map(|(id, dependencies)| GraphTask {
                    id,
                    dependencies,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let layout = layout_graph(&run, &GraphCanvasState::default(), 160, 40);
        for edge in &layout.edges {
            for pair in edge.points.windows(2) {
                let ((x1, y1), (x2, y2)) = (pair[0], pair[1]);
                for node in &layout.nodes {
                    let r = node.rect;
                    let inside =
                        |x: u16, y: u16| x >= r.x && x < r.right() && y > r.y && y + 1 < r.bottom();
                    for y in y1.min(y2)..=y1.max(y2) {
                        for x in x1.min(x2)..=x1.max(x2) {
                            assert!(!inside(x, y), "route crosses {} at {x},{y}", node.id);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn graph_layout_responsive_and_bounded_inspector() {
        for (width, height, mode) in [
            (40, 30, GraphResponsiveMode::Structured),
            (50, 30, GraphResponsiveMode::Compact),
            (80, 30, GraphResponsiveMode::Adaptive),
            (120, 30, GraphResponsiveMode::Full),
            (120, 21, GraphResponsiveMode::Full),
            (109, 21, GraphResponsiveMode::Full),
            (120, 5, GraphResponsiveMode::Structured),
        ] {
            let layout = layout_graph(&diamond(), &GraphCanvasState::default(), width, height);
            assert_eq!(layout.mode, mode);
            assert!(layout.inspector.right() <= width);
            assert!(layout.inspector.bottom() <= height);
            assert!(layout.canvas.right() <= layout.inspector.x.max(layout.canvas.right()));
        }
    }

    #[test]
    fn graph_layout_refresh_appends_without_moving_drawn_cards() {
        let run = crate::davinci::fixtures::blueprint_graph();
        let canvas = GraphCanvasState {
            node_order: run.tasks.iter().map(|t| t.id.clone()).collect(),
            ..Default::default()
        };
        let before = layout_graph(&run, &canvas, 160, 40);
        let mut next = run.clone();
        let mut added = next.tasks[5].clone();
        added.id = "late-writer".into();
        next.tasks.insert(0, added);
        let after = layout_graph(&next, &canvas, 160, 40);
        // Cards keep their slot and column; a route the newcomer needs may
        // add a lane, which only moves later rows down.
        for drawn in &before.nodes {
            let moved = node(&after, &drawn.id);
            assert_eq!(
                (drawn.row, drawn.col),
                (moved.row, moved.col),
                "{}",
                drawn.id
            );
            assert_eq!(drawn.rect.x, moved.rect.x, "{}", drawn.id);
            assert!(moved.rect.y >= drawn.rect.y, "{}", drawn.id);
        }
        let late = node(&after, "late-writer");
        assert!(before
            .nodes
            .iter()
            .all(|n| (n.row, n.col) < (late.row, late.col)));
    }

    #[test]
    fn graph_layout_rejects_cycles_duplicates_and_marks_missing_edges() {
        let mut run = diamond();
        run.tasks[0].dependencies.push("join".into());
        let cycle = layout_graph(&run, &GraphCanvasState::default(), 120, 30);
        assert_eq!(cycle.mode, GraphResponsiveMode::Structured);
        assert!(!cycle.issues.is_empty());
        run.tasks[0].dependencies = vec!["missing".into()];
        let missing = layout_graph(&run, &GraphCanvasState::default(), 120, 30);
        assert_eq!(missing.nodes.len(), 4);
        assert!(missing.issues.iter().any(|s| s.contains("missing")));
        run.tasks[1].id = "start".into();
        assert_eq!(
            layout_graph(&run, &GraphCanvasState::default(), 120, 30).mode,
            GraphResponsiveMode::Structured
        );
    }
}
