//! Constrained layout engine matching TypeScript `packages/tui/src/layout.ts`.

use crate::ansi_text::slice_by_column;
use crate::component::{wrap, Component};
use crate::diff::{composite_tui_line, visible_width};
use crate::terminal_image::{crop_kitty_image_line, get_kitty_image_metadata, is_image_line};
use crate::viewport::ViewportScroll;
use crate::widgets::CURSOR_MARKER;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl LayoutRect {
    fn intersect(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        Self {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }

    fn contains(self, x: usize, y: usize) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

#[derive(Clone)]
pub struct StackEntry {
    pub component: Node,
    pub basis: Option<usize>,
    pub grow: usize,
    pub shrink: usize,
    pub min_size: usize,
    pub max_size: usize,
}

impl StackEntry {
    pub fn auto(component: Node) -> Self {
        Self {
            component,
            basis: None,
            grow: 0,
            shrink: 1,
            min_size: 0,
            max_size: usize::MAX,
        }
    }

    pub fn sized(component: Node, basis: usize) -> Self {
        Self {
            component,
            basis: Some(basis),
            grow: 0,
            shrink: 0,
            min_size: 0,
            max_size: usize::MAX,
        }
    }

    pub fn grow(component: Node, grow: usize, min_size: usize) -> Self {
        Self {
            component,
            basis: Some(0),
            grow,
            shrink: 1,
            min_size,
            max_size: usize::MAX,
        }
    }
}

#[derive(Clone)]
pub struct LayoutVStack {
    pub entries: Rc<RefCell<Vec<StackEntry>>>,
    pub gap: usize,
}

impl LayoutVStack {
    pub fn new(entries: Vec<StackEntry>) -> Self {
        Self {
            entries: Rc::new(RefCell::new(entries)),
            gap: 0,
        }
    }
}

#[derive(Clone)]
pub struct LayoutHStack {
    pub entries: Rc<RefCell<Vec<StackEntry>>>,
    pub gap: usize,
}

impl LayoutHStack {
    pub fn new(entries: Vec<StackEntry>) -> Self {
        Self {
            entries: Rc::new(RefCell::new(entries)),
            gap: 0,
        }
    }
}

#[derive(Clone)]
pub enum Node {
    Text(Rc<RefCell<String>>),
    Render(Rc<dyn Fn(usize) -> Vec<String>>),
    Scroll(ViewportScroll),
    VStack(LayoutVStack),
    HStack(LayoutHStack),
}

impl Node {
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(Rc::new(RefCell::new(value.into())))
    }

    pub fn set_text(&self, value: impl Into<String>) {
        if let Self::Text(inner) = self {
            *inner.borrow_mut() = value.into();
        }
    }

    pub fn render(&self, width: usize) -> Vec<String> {
        match self {
            Self::Text(inner) => {
                let value = inner.borrow();
                if value.trim().is_empty() {
                    Vec::new()
                } else {
                    wrap(&value, width.max(1))
                }
            }
            Self::Render(render) => render(width),
            Self::Scroll(scroll) => scroll.render(width),
            Self::VStack(stack) => stack
                .entries
                .borrow()
                .iter()
                .flat_map(|entry| entry.component.render(width))
                .collect(),
            Self::HStack(stack) => {
                let gap = " ".repeat(stack.gap);
                let line = stack
                    .entries
                    .borrow()
                    .iter()
                    .flat_map(|entry| entry.component.render(width))
                    .collect::<Vec<_>>()
                    .join(&gap);
                wrap(&line, width.max(1))
            }
        }
    }
}

pub struct LayoutBox {
    pub rect: LayoutRect,
    pub clip: LayoutRect,
    pub children: Vec<LayoutBox>,
    pub lines: Option<Vec<String>>,
    pub line_offset: usize,
    pub scroll_view: Option<ViewportScroll>,
    pub scroll_content_lines: Option<Vec<String>>,
}

pub struct LayoutFrame {
    pub root: LayoutBox,
    pub width: usize,
    pub height: usize,
    pub lines: Vec<String>,
    pub primary_scroll_view: Option<ViewportScroll>,
}

pub struct ScrollbarGeometry {
    pub column: usize,
    pub track_top: usize,
    pub track_height: usize,
    pub thumb_top: usize,
    pub thumb_height: usize,
    pub max_scroll_top: usize,
}

struct LayoutContext {
    render_cache: Vec<(usize, usize, Vec<String>)>,
    primary: Option<ViewportScroll>,
}

impl LayoutContext {
    fn render(&mut self, node: &Node, width: usize) -> Vec<String> {
        let key = node as *const Node as usize;
        if let Some((_, _, lines)) = self
            .render_cache
            .iter()
            .find(|(k, w, _)| *k == key && *w == width)
        {
            return lines.clone();
        }
        let lines = node.render(width);
        self.render_cache.push((key, width, lines.clone()));
        lines
    }
}

fn allocate_stack_sizes(
    entries: &[StackEntry],
    intrinsic: &[usize],
    available: Option<usize>,
    gap: usize,
) -> Vec<usize> {
    let mut sizes: Vec<usize> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let raw = entry
                .basis
                .unwrap_or(intrinsic.get(index).copied().unwrap_or(0));
            raw.clamp(entry.min_size, entry.max_size)
        })
        .collect();
    let Some(available) = available else {
        return sizes;
    };
    let content = available.saturating_sub(entries.len().saturating_sub(1) * gap);
    let total: usize = sizes.iter().sum();
    match total.cmp(&content) {
        std::cmp::Ordering::Less => distribute(&mut sizes, entries, content - total, true),
        std::cmp::Ordering::Greater => distribute(&mut sizes, entries, total - content, false),
        std::cmp::Ordering::Equal => {}
    }
    sizes
}

fn distribute(sizes: &mut [usize], entries: &[StackEntry], amount: usize, grow: bool) {
    let mut remaining = amount;
    while remaining > 0 {
        let candidates: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(index, entry)| {
                if grow {
                    entry.grow > 0 && sizes[*index] < entry.max_size
                } else {
                    entry.shrink > 0 && sizes[*index] > entry.min_size
                }
            })
            .map(|(index, _)| index)
            .collect();
        if candidates.is_empty() {
            return;
        }
        let total_weight: usize = candidates
            .iter()
            .map(|&index| {
                if grow {
                    entries[index].grow
                } else {
                    entries[index].shrink * sizes[index].max(1)
                }
            })
            .sum();
        let mut distributed = 0usize;
        for index in candidates {
            if remaining == 0 {
                break;
            }
            let weight = if grow {
                entries[index].grow
            } else {
                entries[index].shrink * sizes[index].max(1)
            };
            let proposed = ((remaining * weight) / total_weight).max(1);
            let capacity = if grow {
                entries[index].max_size.saturating_sub(sizes[index])
            } else {
                sizes[index].saturating_sub(entries[index].min_size)
            };
            let delta = remaining.min(proposed).min(capacity);
            if delta == 0 {
                continue;
            }
            if grow {
                sizes[index] += delta;
            } else {
                sizes[index] -= delta;
            }
            remaining -= delta;
            distributed += delta;
        }
        if distributed == 0 {
            return;
        }
    }
}

fn translate_box(box_: &mut LayoutBox, delta_y: isize) {
    if delta_y >= 0 {
        box_.rect.y = box_.rect.y.saturating_add(delta_y as usize);
    } else {
        box_.rect.y = box_.rect.y.saturating_sub((-delta_y) as usize);
    }
    for child in &mut box_.children {
        translate_box(child, delta_y);
    }
}

fn update_clips(box_: &mut LayoutBox, parent_clip: LayoutRect) {
    box_.clip = parent_clip.intersect(box_.rect);
    let clip = box_.clip;
    for child in &mut box_.children {
        update_clips(child, clip);
    }
}

fn layout_component(
    context: &mut LayoutContext,
    component: &Node,
    x: usize,
    y: usize,
    width: usize,
    height: Option<usize>,
    clip: LayoutRect,
) -> LayoutBox {
    let safe_width = width.max(1);
    match component {
        Node::Scroll(scroll) => {
            let previous = scroll.scroll_top();
            let content_width = scroll.content_width(safe_width);
            let child = scroll.inner.borrow().child.clone();
            let mut child_box = layout_component(
                context,
                &child,
                x,
                y.saturating_sub(previous),
                content_width,
                None,
                clip,
            );
            let content_height = child_box.rect.height;
            let viewport_height = height.unwrap_or(content_height);
            scroll.update_layout(content_height, viewport_height);
            let delta = previous as isize - scroll.scroll_top() as isize;
            translate_box(&mut child_box, delta);
            if scroll.inner.borrow().primary || context.primary.is_none() {
                context.primary = Some(scroll.clone());
            }
            let rect = LayoutRect {
                x,
                y,
                width: safe_width,
                height: viewport_height,
            };
            let child_clip = clip.intersect(rect);
            let scroll_content = context.render(&child, content_width);
            let mut box_ = LayoutBox {
                rect,
                clip: child_clip,
                children: vec![child_box],
                lines: None,
                line_offset: 0,
                scroll_view: Some(scroll.clone()),
                scroll_content_lines: Some(scroll_content),
            };
            update_clips(&mut box_.children[0], child_clip);
            box_
        }
        Node::VStack(stack) => {
            let entries = stack.entries.borrow().clone();
            let intrinsic: Vec<usize> = entries
                .iter()
                .map(|entry| {
                    entry
                        .basis
                        .unwrap_or_else(|| context.render(&entry.component, safe_width).len())
                })
                .collect();
            let sizes = allocate_stack_sizes(&entries, &intrinsic, height, stack.gap);
            let natural: usize =
                sizes.iter().sum::<usize>() + entries.len().saturating_sub(1) * stack.gap;
            let allocated = height.unwrap_or(natural);
            let rect = LayoutRect {
                x,
                y,
                width: safe_width,
                height: allocated,
            };
            let mut box_ = LayoutBox {
                rect,
                clip: clip.intersect(rect),
                children: Vec::new(),
                lines: None,
                line_offset: 0,
                scroll_view: None,
                scroll_content_lines: None,
            };
            let mut child_y = y;
            for (index, entry) in entries.iter().enumerate() {
                box_.children.push(layout_component(
                    context,
                    &entry.component,
                    x,
                    child_y,
                    safe_width,
                    Some(sizes[index]),
                    box_.clip,
                ));
                child_y += sizes[index] + stack.gap;
            }
            box_
        }
        Node::HStack(stack) => {
            let entries = stack.entries.borrow().clone();
            let intrinsic_widths: Vec<usize> = entries
                .iter()
                .map(|entry| {
                    entry.basis.unwrap_or_else(|| {
                        context
                            .render(&entry.component, safe_width)
                            .iter()
                            .map(|line| visible_width(line))
                            .max()
                            .unwrap_or(0)
                    })
                })
                .collect();
            let widths =
                allocate_stack_sizes(&entries, &intrinsic_widths, Some(safe_width), stack.gap);
            let intrinsic_heights: Vec<usize> = entries
                .iter()
                .enumerate()
                .map(|(index, entry)| context.render(&entry.component, widths[index].max(1)).len())
                .collect();
            let allocated = height.unwrap_or(intrinsic_heights.iter().copied().max().unwrap_or(0));
            let rect = LayoutRect {
                x,
                y,
                width: safe_width,
                height: allocated,
            };
            let mut box_ = LayoutBox {
                rect,
                clip: clip.intersect(rect),
                children: Vec::new(),
                lines: None,
                line_offset: 0,
                scroll_view: None,
                scroll_content_lines: None,
            };
            let mut child_x = x;
            for (index, entry) in entries.iter().enumerate() {
                let child_height = intrinsic_heights[index].min(allocated);
                if widths[index] == 0 {
                    box_.children.push(LayoutBox {
                        rect: LayoutRect {
                            x: child_x,
                            y,
                            width: 0,
                            height: child_height,
                        },
                        clip: LayoutRect {
                            x: child_x,
                            y,
                            width: 0,
                            height: 0,
                        },
                        children: Vec::new(),
                        lines: None,
                        line_offset: 0,
                        scroll_view: None,
                        scroll_content_lines: None,
                    });
                } else {
                    box_.children.push(layout_component(
                        context,
                        &entry.component,
                        child_x,
                        y,
                        widths[index],
                        Some(child_height),
                        box_.clip,
                    ));
                }
                child_x += widths[index] + stack.gap;
            }
            box_
        }
        _ => {
            let lines = context.render(component, safe_width);
            let allocated = height.unwrap_or(lines.len());
            let mut line_offset = 0;
            if lines.len() > allocated && allocated > 0 {
                if let Some(cursor) = lines.iter().position(|line| line.contains(CURSOR_MARKER)) {
                    if cursor >= allocated {
                        line_offset = cursor - allocated + 1;
                    }
                }
            }
            LayoutBox {
                rect: LayoutRect {
                    x,
                    y,
                    width: safe_width,
                    height: allocated,
                },
                clip: clip.intersect(LayoutRect {
                    x,
                    y,
                    width: safe_width,
                    height: allocated,
                }),
                children: Vec::new(),
                lines: Some(lines),
                line_offset,
                scroll_view: None,
                scroll_content_lines: None,
            }
        }
    }
}

fn style_scrollbar_cell(
    line: &str,
    column: usize,
    total_width: usize,
    style: fn(&str) -> String,
) -> String {
    if is_image_line(line) {
        return line.to_string();
    }
    let range = crate::ansi_text::get_grapheme_cell_range(line, column);
    let start = range.map(|r| r.start).unwrap_or(column);
    let end = range.map(|r| r.end).unwrap_or(column + 1);
    let before = slice_by_column(line, 0, start, true);
    let target = slice_by_column(line, start, end.saturating_sub(start), true);
    let after = slice_by_column(line, end, total_width.saturating_sub(end), true);
    let mut target_index = 0usize;
    let mut target_prefix = String::new();
    while target_index < target.len() {
        if let Some(ansi) = crate::ansi_text::extract_ansi_code(&target, target_index) {
            target_prefix.push_str(ansi.code);
            target_index += ansi.length;
        } else {
            break;
        }
    }
    let target_text = if target_index < target.len() {
        target[target_index..].to_string()
    } else {
        " ".repeat(end.saturating_sub(start).max(1))
    };
    format!("{before}{target_prefix}{}{after}", style(&target_text))
}

pub fn get_scrollbar_geometry(box_: &LayoutBox) -> Option<ScrollbarGeometry> {
    let scroll = box_.scroll_view.as_ref()?;
    if !scroll.is_scrollbar_visible() || box_.rect.width == 0 || box_.rect.height == 0 {
        return None;
    }
    let content_height = box_
        .children
        .first()
        .map(|child| child.rect.height)
        .or_else(|| box_.scroll_content_lines.as_ref().map(Vec::len))
        .unwrap_or(0);
    let track_height = box_.rect.height;
    let min_thumb = 2.min(track_height);
    let thumb_height = if content_height == 0 {
        track_height
    } else {
        ((track_height * track_height) / content_height)
            .max(min_thumb)
            .min(track_height)
    };
    let max_scroll_top = content_height.saturating_sub(track_height);
    let max_thumb_top = track_height.saturating_sub(thumb_height);
    let thumb_offset = if max_scroll_top == 0 {
        0
    } else {
        (scroll.scroll_top() * max_thumb_top + max_scroll_top / 2) / max_scroll_top
    };
    let column = box_.rect.x + box_.rect.width - 1;
    if column < box_.clip.x || column >= box_.clip.x + box_.clip.width {
        return None;
    }
    Some(ScrollbarGeometry {
        column,
        track_top: box_.rect.y,
        track_height,
        thumb_top: box_.rect.y + thumb_offset,
        thumb_height,
        max_scroll_top,
    })
}

fn paint_scrollbar(box_: &LayoutBox, screen: &mut [String], total_width: usize) {
    let Some(geometry) = get_scrollbar_geometry(box_) else {
        return;
    };
    let Some(scroll) = &box_.scroll_view else {
        return;
    };
    let style = scroll.inner.borrow().scrollbar_style;
    for offset in 0..geometry.thumb_height {
        let row = geometry.thumb_top + offset;
        if row < box_.clip.y || row >= box_.clip.y + box_.clip.height || row >= screen.len() {
            continue;
        }
        screen[row] = style_scrollbar_cell(&screen[row], geometry.column, total_width, style);
    }
}

fn paint_box(box_: &LayoutBox, screen: &mut [String], total_width: usize) {
    if let Some(lines) = &box_.lines {
        let offset = box_.line_offset;
        let first = box_.rect.y.max(box_.clip.y);
        let last = (box_.rect.y + box_.rect.height)
            .min(box_.clip.y + box_.clip.height)
            .min(screen.len());
        for row in first..last {
            let Some(source) = lines.get(offset + row - box_.rect.y) else {
                continue;
            };
            let mut line = crate::ansi_text::strip_osc133_zone_prefix(source);
            if let Some(metadata) = get_kitty_image_metadata(&line) {
                let clip_bottom = screen.len().min(box_.clip.y + box_.clip.height);
                let visible_rows =
                    (metadata.rows as usize).min(clip_bottom.saturating_sub(row)) as u32;
                if visible_rows < metadata.rows {
                    line = crop_kitty_image_line(&line, 0, visible_rows);
                }
            }
            if box_.rect.x == 0
                && box_.rect.width >= total_width
                && (is_image_line(&line) || screen[row].is_empty())
            {
                screen[row] = line;
            } else {
                screen[row] = composite_tui_line(
                    &screen[row],
                    &line,
                    box_.rect.x,
                    box_.rect.width,
                    total_width,
                );
            }
        }
    }
    for child in &box_.children {
        paint_box(child, screen, total_width);
    }
    if let (Some(scroll), Some(content)) = (&box_.scroll_view, &box_.scroll_content_lines) {
        if scroll.scroll_top() > 0 && box_.rect.height > 0 {
            for image_row in (0..scroll.scroll_top()).rev() {
                let image_line = content.get(image_row).map(String::as_str).unwrap_or("");
                if let Some(metadata) = get_kitty_image_metadata(image_line) {
                    let hidden = (scroll.scroll_top() - image_row) as u32;
                    if hidden < metadata.rows {
                        let visible = (box_.rect.height as u32).min(metadata.rows - hidden);
                        let cropped = crop_kitty_image_line(image_line, hidden, visible);
                        if box_.rect.x == 0 && box_.rect.width >= total_width {
                            screen[box_.rect.y] = cropped;
                        }
                    }
                    break;
                }
                if !image_line.is_empty() {
                    break;
                }
            }
        }
    }
    paint_scrollbar(box_, screen, total_width);
}

pub fn render_layout_frame(root: &Node, width: usize, height: usize) -> LayoutFrame {
    let safe_width = width.max(1);
    let safe_height = height.max(1);
    let mut context = LayoutContext {
        render_cache: Vec::new(),
        primary: None,
    };
    let root_box = layout_component(
        &mut context,
        root,
        0,
        0,
        safe_width,
        Some(safe_height),
        LayoutRect {
            x: 0,
            y: 0,
            width: safe_width,
            height: safe_height,
        },
    );
    let mut lines = vec![String::new(); safe_height];
    paint_box(&root_box, &mut lines, safe_width);
    LayoutFrame {
        root: root_box,
        width: safe_width,
        height: safe_height,
        lines,
        primary_scroll_view: context.primary,
    }
}

pub fn get_scroll_view_box<'a>(
    frame: &'a LayoutFrame,
    scroll: &ViewportScroll,
) -> Option<&'a LayoutBox> {
    fn visit<'a>(box_: &'a LayoutBox, scroll: &ViewportScroll) -> Option<&'a LayoutBox> {
        if box_
            .scroll_view
            .as_ref()
            .is_some_and(|candidate| candidate.ptr_eq(scroll))
        {
            return Some(box_);
        }
        box_.children.iter().find_map(|child| visit(child, scroll))
    }
    visit(&frame.root, scroll)
}

pub fn get_scroll_views_at(frame: &LayoutFrame, x: usize, y: usize) -> Vec<ViewportScroll> {
    let mut result = Vec::new();
    fn visit(
        box_: &LayoutBox,
        x: usize,
        y: usize,
        depth: usize,
        result: &mut Vec<(ViewportScroll, usize)>,
    ) {
        if !box_.clip.contains(x, y) {
            return;
        }
        if let Some(scroll) = &box_.scroll_view {
            if box_.rect.contains(x, y) {
                result.push((scroll.clone(), depth));
            }
        }
        for child in &box_.children {
            visit(child, x, y, depth + 1, result);
        }
    }
    let mut ranked = Vec::new();
    visit(&frame.root, x, y, 0, &mut ranked);
    ranked.sort_by_key(|(_, depth)| std::cmp::Reverse(*depth));
    result.extend(ranked.into_iter().map(|(scroll, _)| scroll));
    result
}
