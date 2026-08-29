//! Viewport layout matching `vendor/pi/packages/tui/src/layout.ts`.

use std::collections::HashMap;
use std::rc::Rc;

use crate::ansi::{get_grapheme_cell_range, slice_by_column, visible_width};
use crate::image::{crop_kitty_image_line, get_kitty_image_metadata, is_image_line};
use crate::overlay::composite_tui_line;
use crate::render::Component;
use crate::scroll::ScrollView;
use crate::stack::{
    allocate_stack_sizes, HStack, LayoutViewport, StackAlign, StackBasis, StackLayoutEntry, VStack,
};
use crate::CURSOR_MARKER;

const OSC133_ZONE_PREFIX: &str = "\x1b]133;";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

pub struct LayoutBox {
    pub rect: LayoutRect,
    pub clip: LayoutRect,
    pub children: Vec<LayoutBox>,
    pub lines: Option<Vec<String>>,
    pub line_offset: usize,
    pub scroll_top: usize,
    pub scroll_content_lines: Option<Vec<String>>,
    pub scrollbar: Option<ScrollbarPaint>,
    pub scroll_id: Option<usize>,
    pub layer: usize,
}

pub struct ScrollbarPaint {
    pub style: Rc<dyn Fn(&str) -> String>,
    pub visible: bool,
}

pub struct LayoutFrame {
    pub root: LayoutBox,
    pub width: usize,
    pub height: usize,
    pub lines: Vec<String>,
    pub primary_scroll_id: Option<usize>,
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
    viewport: LayoutViewport,
    render_cache: HashMap<(usize, usize), Vec<String>>,
    primary_scroll_id: Option<usize>,
}

fn component_id(component: &dyn Component) -> usize {
    component as *const dyn Component as *const () as usize
}

fn intersect(a: LayoutRect, b: LayoutRect) -> LayoutRect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    LayoutRect {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

fn render_cached(
    context: &mut LayoutContext,
    component: &dyn Component,
    width: usize,
) -> Vec<String> {
    let safe_width = width.max(1);
    let key = (component_id(component), safe_width);
    if let Some(lines) = context.render_cache.get(&key) {
        return lines.clone();
    }
    let lines = component.render(safe_width);
    context.render_cache.insert(key, lines.clone());
    lines
}

fn measure_height(context: &mut LayoutContext, component: &dyn Component, width: usize) -> usize {
    render_cached(context, component, width).len()
}

fn measure_width(context: &mut LayoutContext, component: &dyn Component, width: usize) -> usize {
    render_cached(context, component, width)
        .iter()
        .map(|line| visible_width(line))
        .max()
        .unwrap_or(0)
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
    box_.clip = intersect(parent_clip, box_.rect);
    let clip = box_.clip;
    for child in &mut box_.children {
        update_clips(child, clip);
    }
}

fn strip_osc133_prefix(line: &str) -> String {
    let mut rest = line;
    while rest.starts_with(OSC133_ZONE_PREFIX) {
        let after = &rest[OSC133_ZONE_PREFIX.len()..];
        if after.starts_with('A') || after.starts_with('B') || after.starts_with('C') {
            let body = &after[1..];
            if let Some(end) = body.find('\u{7}') {
                rest = &body[end + 1..];
                continue;
            }
            if let Some(end) = body.find("\x1b\\") {
                rest = &body[end + 2..];
                continue;
            }
        }
        break;
    }
    rest.to_string()
}

fn visible_indices(entries: &[StackLayoutEntry], viewport: LayoutViewport) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry
                .visible
                .as_ref()
                .map_or(true, |visible| visible(viewport))
        })
        .map(|(index, _)| index)
        .collect()
}

fn layout_component(
    context: &mut LayoutContext,
    component: &mut dyn Component,
    x: usize,
    y: usize,
    width: usize,
    height: Option<usize>,
    clip: LayoutRect,
) -> LayoutBox {
    let safe_width = width.max(1);
    if let Some(scroll) = component.as_any_mut().downcast_mut::<ScrollView>() {
        return layout_scroll(context, scroll, x, y, safe_width, height, clip);
    }
    if let Some(stack) = component.as_any_mut().downcast_mut::<VStack>() {
        return layout_vstack(context, stack, x, y, safe_width, height, clip);
    }
    if let Some(stack) = component.as_any_mut().downcast_mut::<HStack>() {
        return layout_hstack(context, stack, x, y, safe_width, height, clip);
    }
    let lines = render_cached(context, component, safe_width);
    let allocated_height = height.unwrap_or(lines.len());
    let mut line_offset = 0usize;
    if lines.len() > allocated_height && allocated_height > 0 {
        if let Some(cursor_line) = lines.iter().position(|line| line.contains(CURSOR_MARKER)) {
            if cursor_line >= allocated_height {
                line_offset = cursor_line - allocated_height + 1;
            }
        }
    }
    LayoutBox {
        rect: LayoutRect {
            x,
            y,
            width: safe_width,
            height: allocated_height,
        },
        clip: intersect(
            clip,
            LayoutRect {
                x,
                y,
                width: safe_width,
                height: allocated_height,
            },
        ),
        children: Vec::new(),
        lines: Some(lines),
        line_offset,
        scroll_top: 0,
        scroll_content_lines: None,
        scrollbar: None,
        scroll_id: None,
        layer: 0,
    }
}

fn layout_scroll(
    context: &mut LayoutContext,
    scroll: &mut ScrollView,
    x: usize,
    y: usize,
    safe_width: usize,
    height: Option<usize>,
    clip: LayoutRect,
) -> LayoutBox {
    let previous_scroll_top = scroll.scroll_top();
    let content_width = scroll.get_content_width(safe_width);
    let mut child_box = layout_component(
        context,
        scroll.child_mut(),
        x,
        y.saturating_sub(previous_scroll_top),
        content_width,
        None,
        clip,
    );
    let content_height = child_box.rect.height;
    let viewport_height = height.unwrap_or(content_height);
    scroll.update_layout(content_height, viewport_height);
    let delta = previous_scroll_top as isize - scroll.scroll_top() as isize;
    translate_box(&mut child_box, delta);
    let rect = LayoutRect {
        x,
        y,
        width: safe_width,
        height: viewport_height,
    };
    let child_clip = intersect(clip, rect);
    child_box.clip = intersect(child_clip, child_box.rect);
    let scroll_content_lines = render_cached(context, scroll.child_mut(), content_width);
    let scrollbar = if scroll.is_scrollbar_visible() {
        Some(ScrollbarPaint {
            style: scroll.scrollbar_style(),
            visible: true,
        })
    } else {
        None
    };
    let scroll_top = scroll.scroll_top();
    let scroll_id = component_id(scroll);
    if scroll.primary || context.primary_scroll_id.is_none() {
        context.primary_scroll_id = Some(scroll_id);
    }
    let mut box_ = LayoutBox {
        rect,
        clip: child_clip,
        children: vec![child_box],
        lines: None,
        line_offset: 0,
        scroll_top,
        scroll_content_lines: Some(scroll_content_lines),
        scrollbar,
        scroll_id: Some(scroll_id),
        layer: 0,
    };
    update_clips(&mut box_, child_clip);
    box_
}

fn layout_vstack(
    context: &mut LayoutContext,
    stack: &mut VStack,
    x: usize,
    y: usize,
    safe_width: usize,
    height: Option<usize>,
    clip: LayoutRect,
) -> LayoutBox {
    let visible = visible_indices(&stack.entries, context.viewport);
    let gap_total = visible.len().saturating_sub(1) * stack.gap;
    let intrinsic: Vec<usize> = visible
        .iter()
        .map(|&index| match stack.entries[index].basis {
            Some(StackBasis::Fixed(basis)) => basis,
            _ => measure_height(context, &*stack.entries[index].component, safe_width),
        })
        .collect();
    let sizes = {
        let refs: Vec<&StackLayoutEntry> =
            visible.iter().map(|&index| &stack.entries[index]).collect();
        allocate_stack_sizes(&refs, &intrinsic, height, stack.gap)
    };
    let natural_height = sizes.iter().sum::<usize>() + gap_total;
    let allocated_height = height.unwrap_or(natural_height);
    let rect = LayoutRect {
        x,
        y,
        width: safe_width,
        height: allocated_height,
    };
    let mut box_ = LayoutBox {
        rect,
        clip: intersect(clip, rect),
        children: Vec::new(),
        lines: None,
        line_offset: 0,
        scroll_top: 0,
        scroll_content_lines: None,
        scrollbar: None,
        scroll_id: None,
        layer: 0,
    };
    let parent_clip = box_.clip;
    let mut child_y = y;
    for (k, &index) in visible.iter().enumerate() {
        let size = sizes[k];
        let child = layout_component(
            context,
            &mut *stack.entries[index].component,
            x,
            child_y,
            safe_width,
            Some(size),
            parent_clip,
        );
        box_.children.push(child);
        child_y += size + stack.gap;
    }
    box_
}

fn layout_hstack(
    context: &mut LayoutContext,
    stack: &mut HStack,
    x: usize,
    y: usize,
    safe_width: usize,
    height: Option<usize>,
    clip: LayoutRect,
) -> LayoutBox {
    let visible = visible_indices(&stack.entries, context.viewport);
    let intrinsic_widths: Vec<usize> = visible
        .iter()
        .map(|&index| match stack.entries[index].basis {
            Some(StackBasis::Fixed(basis)) => basis,
            _ => measure_width(context, &*stack.entries[index].component, safe_width),
        })
        .collect();
    let widths = {
        let refs: Vec<&StackLayoutEntry> =
            visible.iter().map(|&index| &stack.entries[index]).collect();
        allocate_stack_sizes(&refs, &intrinsic_widths, Some(safe_width), stack.gap)
    };
    let intrinsic_heights: Vec<usize> = visible
        .iter()
        .enumerate()
        .map(|(k, &index)| {
            measure_height(context, &*stack.entries[index].component, widths[k].max(1))
        })
        .collect();
    let allocated_height =
        height.unwrap_or_else(|| intrinsic_heights.iter().copied().max().unwrap_or(0));
    let rect = LayoutRect {
        x,
        y,
        width: safe_width,
        height: allocated_height,
    };
    let mut box_ = LayoutBox {
        rect,
        clip: intersect(clip, rect),
        children: Vec::new(),
        lines: None,
        line_offset: 0,
        scroll_top: 0,
        scroll_content_lines: None,
        scrollbar: None,
        scroll_id: None,
        layer: 0,
    };
    let parent_clip = box_.clip;
    let mut child_x = x;
    for (k, &index) in visible.iter().enumerate() {
        let natural = intrinsic_heights[k];
        let child_height = if stack.align == StackAlign::Stretch {
            allocated_height
        } else {
            allocated_height.min(natural)
        };
        let mut child_y = y;
        if stack.align == StackAlign::Center {
            child_y += allocated_height.saturating_sub(child_height) / 2;
        } else if stack.align == StackAlign::End {
            child_y += allocated_height.saturating_sub(child_height);
        }
        let child_width = widths[k];
        if child_width == 0 {
            box_.children.push(LayoutBox {
                rect: LayoutRect {
                    x: child_x,
                    y: child_y,
                    width: 0,
                    height: child_height,
                },
                clip: LayoutRect {
                    x: child_x,
                    y: child_y,
                    width: 0,
                    height: 0,
                },
                children: Vec::new(),
                lines: None,
                line_offset: 0,
                scroll_top: 0,
                scroll_content_lines: None,
                scrollbar: None,
                scroll_id: None,
                layer: 0,
            });
        } else {
            box_.children.push(layout_component(
                context,
                &mut *stack.entries[index].component,
                child_x,
                child_y,
                child_width,
                Some(child_height),
                parent_clip,
            ));
        }
        child_x += child_width + stack.gap;
    }
    box_
}

fn style_scrollbar_cell(
    line: &str,
    column: usize,
    total_width: usize,
    style: &dyn Fn(&str) -> String,
) -> String {
    if is_image_line(line) {
        return line.to_string();
    }
    let range = get_grapheme_cell_range(line, column);
    let start = range.map(|item| item.start).unwrap_or(column);
    let end = range.map(|item| item.end).unwrap_or(column + 1);
    let before = slice_by_column(line, 0, start, true);
    let target = slice_by_column(line, start, end.saturating_sub(start), true);
    let after = slice_by_column(line, end, total_width.saturating_sub(end), true);
    let mut target_prefix = String::new();
    let mut target_index = 0usize;
    while target_index < target.len() {
        if let Some((code, len)) = crate::ansi::extract_ansi_code(&target, target_index) {
            target_prefix.push_str(&code);
            target_index += len;
        } else {
            break;
        }
    }
    let target_text = if target_index < target.len() {
        target[target_index..].to_string()
    } else {
        " ".repeat(end.saturating_sub(start))
    };
    let before_padding = " ".repeat(start.saturating_sub(visible_width(&before)));
    format!(
        "{before}{before_padding}{target_prefix}{}{after}",
        style(&target_text)
    )
}

pub fn get_scrollbar_geometry(box_: &LayoutBox) -> Option<ScrollbarGeometry> {
    let scrollbar = box_.scrollbar.as_ref()?;
    if !scrollbar.visible || box_.rect.width == 0 || box_.rect.height == 0 {
        return None;
    }
    let content_height = box_
        .children
        .first()
        .map(|child| child.rect.height)
        .or_else(|| box_.scroll_content_lines.as_ref().map(Vec::len))
        .unwrap_or(0);
    let track_height = box_.rect.height;
    let min_thumb_height = 2.min(track_height);
    let thumb_height = min_thumb_height.max(track_height.min(
        ((track_height * track_height) as f64 / content_height.max(1) as f64).round() as usize,
    ));
    let max_scroll_top = content_height.saturating_sub(track_height);
    let max_thumb_top = track_height.saturating_sub(thumb_height);
    let thumb_offset = if max_scroll_top == 0 {
        0
    } else {
        ((box_.scroll_top as f64 / max_scroll_top as f64) * max_thumb_top as f64).round() as usize
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
    let Some(scrollbar) = &box_.scrollbar else {
        return;
    };
    for offset in 0..geometry.thumb_height {
        let row = geometry.thumb_top + offset;
        if row < box_.clip.y || row >= box_.clip.y + box_.clip.height || row >= screen.len() {
            continue;
        }
        screen[row] = style_scrollbar_cell(
            screen.get(row).map(String::as_str).unwrap_or(""),
            geometry.column,
            total_width,
            scrollbar.style.as_ref(),
        );
    }
}

fn paint_box(box_: &LayoutBox, screen: &mut [String], total_width: usize) {
    if let Some(lines) = &box_.lines {
        let offset = box_.line_offset;
        let first_row = box_.rect.y.max(box_.clip.y);
        let last_row = (box_.rect.y + box_.rect.height)
            .min(box_.clip.y + box_.clip.height)
            .min(screen.len());
        for row in first_row..last_row {
            let Some(source_line) = lines.get(offset + row - box_.rect.y) else {
                continue;
            };
            let mut line = strip_osc133_prefix(source_line);
            if let Some(metadata) = get_kitty_image_metadata(&line) {
                let clip_bottom = screen.len().min(box_.clip.y + box_.clip.height);
                let visible_rows = (metadata.rows as usize).min(clip_bottom.saturating_sub(row));
                if visible_rows < metadata.rows as usize {
                    line = crop_kitty_image_line(&line, 0, visible_rows as u32);
                }
            }
            if box_.rect.x == 0
                && box_.rect.width >= total_width
                && (is_image_line(&line) || screen[row].is_empty())
            {
                screen[row] = line;
            } else {
                screen[row] = composite_tui_line(
                    screen.get(row).map(String::as_str).unwrap_or(""),
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
    if box_.scrollbar.is_some() {
        if let Some(content) = &box_.scroll_content_lines {
            if box_.scroll_top > 0 && box_.rect.height > 0 {
                for image_row in (0..box_.scroll_top).rev() {
                    let image_line = content.get(image_row).map(String::as_str).unwrap_or("");
                    if let Some(metadata) = get_kitty_image_metadata(image_line) {
                        let hidden_rows = box_.scroll_top - image_row;
                        if hidden_rows < metadata.rows as usize {
                            let visible_rows =
                                box_.rect.height.min(metadata.rows as usize - hidden_rows);
                            let cropped = crop_kitty_image_line(
                                image_line,
                                hidden_rows as u32,
                                visible_rows as u32,
                            );
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
}

/// TS `renderLayoutFrame`.
pub fn render_layout_frame(root: &mut dyn Component, width: usize, height: usize) -> LayoutFrame {
    let safe_width = width.max(1);
    let safe_height = height.max(1);
    let mut context = LayoutContext {
        viewport: LayoutViewport {
            width: safe_width,
            height: safe_height,
        },
        render_cache: HashMap::new(),
        primary_scroll_id: None,
    };
    let clip = LayoutRect {
        x: 0,
        y: 0,
        width: safe_width,
        height: safe_height,
    };
    let root_box = layout_component(
        &mut context,
        root,
        0,
        0,
        safe_width,
        Some(safe_height),
        clip,
    );
    let mut lines = vec![String::new(); safe_height];
    paint_box(&root_box, &mut lines, safe_width);
    LayoutFrame {
        root: root_box,
        width: safe_width,
        height: safe_height,
        lines,
        primary_scroll_id: context.primary_scroll_id,
    }
}

fn contains_point(rect: LayoutRect, x: usize, y: usize) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

/// TS `getScrollViewBox`.
pub fn get_scroll_view_box(frame: &LayoutFrame, scroll_id: usize) -> Option<&LayoutBox> {
    fn visit(box_: &LayoutBox, scroll_id: usize) -> Option<&LayoutBox> {
        if box_.scroll_id == Some(scroll_id) {
            return Some(box_);
        }
        for child in &box_.children {
            if let Some(found) = visit(child, scroll_id) {
                return Some(found);
            }
        }
        None
    }
    visit(&frame.root, scroll_id)
}

/// TS `getScrollViewsAt`.
pub fn get_scroll_views_at(frame: &LayoutFrame, x: usize, y: usize) -> Vec<usize> {
    let mut result = Vec::new();
    fn visit(box_: &LayoutBox, x: usize, y: usize, depth: usize, result: &mut Vec<(usize, usize)>) {
        if !contains_point(box_.clip, x, y) {
            return;
        }
        if let Some(scroll_id) = box_.scroll_id {
            if contains_point(box_.rect, x, y) {
                result.push((scroll_id, depth));
            }
        }
        for child in &box_.children {
            visit(child, x, y, depth + 1, result);
        }
    }
    visit(&frame.root, x, y, 0, &mut result);
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result.into_iter().map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{encode_kitty, register_kitty_image_metadata, KittyImageMetadata};
    use crate::render::Component;
    use crate::scroll::{ScrollFollow, ScrollView, ScrollViewOptions, ScrollViewScrollbar};
    use crate::stack::{HStack, StackAlign, StackBasis, StackEntryOptions, VStack};
    use crate::tui_text::TuiText;
    use std::cell::Cell;
    use std::rc::Rc;

    fn visible_lines(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                crate::render::strip_terminal_sequences(line)
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    struct Counting {
        count: Rc<Cell<usize>>,
        lines: Vec<String>,
    }

    impl Component for Counting {
        fn render(&self, _width: usize) -> Vec<String> {
            self.count.set(self.count.get() + 1);
            self.lines.clone()
        }
        fn invalidate(&mut self) {}
    }

    #[test]
    fn allocates_vertical_grow_space_deterministically() {
        let mut root = VStack::new(0);
        root.add_child(
            Box::new(TuiText::new("top", 0, 0)),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(1)),
                shrink: Some(0),
                ..StackEntryOptions::default()
            },
        );
        root.add_child(
            Box::new(TuiText::new("body", 0, 0)),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(0)),
                grow: Some(1),
                ..StackEntryOptions::default()
            },
        );
        let frame = render_layout_frame(&mut root, 10, 4);
        let heights: Vec<usize> = frame
            .root
            .children
            .iter()
            .map(|child| child.rect.height)
            .collect();
        assert_eq!(heights, vec![1, 3]);
        assert_eq!(visible_lines(&frame.lines), vec!["top", "body", "", ""]);
    }

    #[test]
    fn does_not_render_fixed_basis_scroll_content_during_stack_measurement() {
        let count = Rc::new(Cell::new(0));
        let transcript = Counting {
            count: count.clone(),
            lines: vec!["one".into(), "two".into(), "three".into()],
        };
        let mut root = VStack::new(0);
        root.add_child(
            Box::new(ScrollView::new(Box::new(transcript), ScrollViewOptions::default()).unwrap()),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(0)),
                grow: Some(1),
                ..StackEntryOptions::default()
            },
        );
        root.add_child(
            Box::new(TuiText::new("dock", 0, 0)),
            StackEntryOptions {
                basis: Some(StackBasis::Auto),
                ..StackEntryOptions::default()
            },
        );
        render_layout_frame(&mut root, 10, 3);
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn shrinks_entries_to_their_minimum_sizes() {
        let mut root = VStack::new(0);
        root.add_child(
            Box::new(TuiText::new("a1\na2\na3", 0, 0)),
            StackEntryOptions {
                shrink: Some(1),
                min_size: Some(1),
                ..StackEntryOptions::default()
            },
        );
        root.add_child(
            Box::new(TuiText::new("b1\nb2\nb3", 0, 0)),
            StackEntryOptions {
                shrink: Some(0),
                ..StackEntryOptions::default()
            },
        );
        let frame = render_layout_frame(&mut root, 10, 4);
        let heights: Vec<usize> = frame
            .root
            .children
            .iter()
            .map(|child| child.rect.height)
            .collect();
        assert_eq!(heights, vec![1, 3]);
        assert_eq!(visible_lines(&frame.lines), vec!["a1", "b1", "b2", "b3"]);
    }

    #[test]
    fn tracks_follow_end_and_unused_scroll_delta() {
        let mut scroll = ScrollView::new(
            Box::new(TuiText::new("1\n2\n3\n4\n5\n6", 0, 0)),
            ScrollViewOptions {
                follow: ScrollFollow::End,
                primary: true,
                ..ScrollViewOptions::default()
            },
        )
        .unwrap();
        render_layout_frame(&mut scroll, 10, 3);
        assert_eq!(scroll.scroll_top(), 3);
        assert!(scroll.is_following_end());
        assert_eq!(scroll.scroll_by(-2), 0);
        assert_eq!(scroll.scroll_top(), 1);
        assert!(!scroll.is_following_end());
        assert_eq!(scroll.scroll_by(-3), -2);
        assert_eq!(scroll.scroll_top(), 0);
        assert_eq!(scroll.scroll_by(10), 7);
        assert_eq!(scroll.scroll_top(), 3);
        assert!(scroll.is_following_end());
    }

    #[test]
    fn crops_kitty_images_at_scroll_lower_boundary() {
        let image_id = 124u32;
        let image_line = encode_kitty("AAAA", Some(2), Some(3), Some(image_id), false);
        register_kitty_image_metadata(KittyImageMetadata {
            image_id,
            columns: 2,
            rows: 3,
            width_px: 100,
            height_px: 100,
        });
        let transcript = Counting {
            count: Rc::new(Cell::new(0)),
            lines: vec![
                "one".into(),
                "two".into(),
                image_line,
                String::new(),
                String::new(),
            ],
        };
        let mut root = VStack::new(0);
        root.add_child(
            Box::new(ScrollView::new(Box::new(transcript), ScrollViewOptions::default()).unwrap()),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(0)),
                grow: Some(1),
                ..StackEntryOptions::default()
            },
        );
        root.add_child(
            Box::new(TuiText::new("dock", 0, 0)),
            StackEntryOptions::default(),
        );
        let frame = render_layout_frame(&mut root, 20, 4);
        assert!(frame.lines[2].contains("y=0,h=34,r=1"));
    }

    #[test]
    fn composes_horizontal_children_at_allocated_widths() {
        let mut stack = HStack::new(0, StackAlign::Stretch);
        stack.add_child(
            Box::new(TuiText::new("left", 0, 0)),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(6)),
                shrink: Some(0),
                ..StackEntryOptions::default()
            },
        );
        stack.add_child(
            Box::new(TuiText::new("right", 0, 0)),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(6)),
                shrink: Some(0),
                ..StackEntryOptions::default()
            },
        );
        let frame = render_layout_frame(&mut stack, 12, 1);
        assert_eq!(visible_lines(&frame.lines), vec!["left  right"]);
    }

    #[test]
    fn transient_scrollbar_and_always_reserved_width() {
        let source_lines = [
            "abcd界", "abcde2", "abcde3", "abcde4", "abcde5", "abcde6", "abcde7", "abcde8",
        ];
        let content_background = "\x1b[42m";
        let scrollbar_background = "\x1b[48;5;1m";
        let style = {
            let scrollbar_background = scrollbar_background.to_string();
            Rc::new(move |text: &str| format!("{scrollbar_background}{text}\x1b[49m"))
        };
        let content = TuiText::with_background(source_lines.join("\n"), 0, 0, {
            let content_background = content_background.to_string();
            move |text: &str| format!("{content_background}{text}\x1b[49m")
        });
        let mut scroll = ScrollView::new(
            Box::new(content),
            ScrollViewOptions {
                scrollbar: ScrollViewScrollbar::Auto,
                scrollbar_style: style.clone(),
                scrollbar_hide_delay_ms: 10,
                ..ScrollViewOptions::default()
            },
        )
        .unwrap();
        let lines = render_layout_frame(&mut scroll, 6, 4).lines;
        assert_eq!(
            lines
                .iter()
                .map(|line| line.contains(scrollbar_background))
                .collect::<Vec<_>>(),
            vec![false, false, false, false]
        );
        scroll.scroll_by(2);
        let lines = render_layout_frame(&mut scroll, 6, 4).lines;
        assert_eq!(
            lines
                .iter()
                .map(|line| line.contains(scrollbar_background))
                .collect::<Vec<_>>(),
            vec![false, true, true, false]
        );
        scroll.tick(30);
        let lines = render_layout_frame(&mut scroll, 6, 4).lines;
        assert!(lines
            .iter()
            .all(|line| !line.contains(scrollbar_background)));
        scroll.scroll_to_end();
        let lines = render_layout_frame(&mut scroll, 6, 4).lines;
        assert_eq!(
            lines
                .iter()
                .map(|line| line.contains(scrollbar_background))
                .collect::<Vec<_>>(),
            vec![false, false, true, true]
        );

        let fitting = TuiText::new("1\n2", 0, 0);
        let mut always = ScrollView::new(
            Box::new(fitting),
            ScrollViewOptions {
                scrollbar: ScrollViewScrollbar::Always,
                scrollbar_style: style,
                ..ScrollViewOptions::default()
            },
        )
        .unwrap();
        let frame = render_layout_frame(&mut always, 6, 4);
        assert_eq!(frame.root.children[0].rect.width, 5);
        assert!(frame
            .lines
            .iter()
            .all(|line| line.contains(scrollbar_background)));
    }

    #[test]
    fn updates_reserved_scrollbar_layout_at_runtime() {
        let mut wrap = HStack::new(0, StackAlign::Start);
        wrap.add_child(
            Box::new(
                ScrollView::new(
                    Box::new(TuiText::new("123456", 0, 0)),
                    ScrollViewOptions {
                        scrollbar: ScrollViewScrollbar::Always,
                        ..ScrollViewOptions::default()
                    },
                )
                .unwrap(),
            ),
            StackEntryOptions::default(),
        );
        let always = render_layout_frame(&mut wrap, 6, 2);
        assert_eq!(visible_lines(&always.lines), vec!["12345", "6"]);
        assert_eq!(always.root.children[0].rect.width, 6);
        assert_eq!(always.root.children[0].children[0].rect.width, 5);

        let scroll = wrap.entries[0]
            .component
            .as_any_mut()
            .downcast_mut::<ScrollView>()
            .expect("scroll");
        scroll.set_scrollbar(ScrollViewScrollbar::Hidden);
        let hidden = render_layout_frame(&mut wrap, 6, 2);
        assert_eq!(hidden.root.children[0].children[0].rect.width, 6);
        let scroll = wrap.entries[0]
            .component
            .as_any_mut()
            .downcast_mut::<ScrollView>()
            .expect("scroll");
        assert!(!scroll.is_scrollbar_visible());
    }

    #[test]
    fn includes_nested_minimum_sizes_in_intrinsic_stack_measurement() {
        let mut dock = VStack::new(0);
        dock.add_child(
            Box::new(TuiText::new("top1\ntop2\ntop3", 0, 0)),
            StackEntryOptions::default(),
        );
        dock.add_child(
            Box::new(TuiText::new("selector", 0, 0)),
            StackEntryOptions {
                min_size: Some(3),
                ..StackEntryOptions::default()
            },
        );
        dock.add_child(
            Box::new(TuiText::new("below", 0, 0)),
            StackEntryOptions::default(),
        );
        dock.add_child(
            Box::new(TuiText::new("footer", 0, 0)),
            StackEntryOptions {
                min_size: Some(1),
                ..StackEntryOptions::default()
            },
        );
        let mut root = VStack::new(0);
        root.add_child(
            Box::new(TuiText::new("body", 0, 0)),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(0)),
                grow: Some(1),
                min_size: Some(1),
                ..StackEntryOptions::default()
            },
        );
        root.add_child(
            Box::new(dock),
            StackEntryOptions {
                basis: Some(StackBasis::Auto),
                min_size: Some(1),
                ..StackEntryOptions::default()
            },
        );
        let frame = render_layout_frame(&mut root, 10, 9);
        assert_eq!(
            visible_lines(&frame.lines),
            vec!["body", "top1", "top2", "top3", "selector", "", "", "below", "footer"]
        );
    }

    #[test]
    fn does_not_paint_zero_width_horizontal_children() {
        let mut stack = HStack::new(0, StackAlign::Stretch);
        stack.add_child(
            Box::new(TuiText::new("hidden", 0, 0)),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(0)),
                shrink: Some(0),
                ..StackEntryOptions::default()
            },
        );
        stack.add_child(
            Box::new(TuiText::new("shown", 0, 0)),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(0)),
                grow: Some(1),
                ..StackEntryOptions::default()
            },
        );
        let frame = render_layout_frame(&mut stack, 5, 1);
        assert_eq!(visible_lines(&frame.lines), vec!["shown"]);
    }

    #[test]
    fn measures_nested_scroll_content_from_constrained_child_geometry() {
        let inner = ScrollView::new(
            Box::new(TuiText::new("1\n2\n3\n4\n5\n6", 0, 0)),
            ScrollViewOptions::default(),
        )
        .unwrap();
        let mut stack = VStack::new(0);
        stack.add_child(
            Box::new(inner),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(2)),
                ..StackEntryOptions::default()
            },
        );
        stack.add_child(
            Box::new(TuiText::new("tail", 0, 0)),
            StackEntryOptions::default(),
        );
        let mut outer = ScrollView::new(Box::new(stack), ScrollViewOptions::default()).unwrap();
        render_layout_frame(&mut outer, 10, 2);
        let inner = outer
            .child_mut()
            .as_any_mut()
            .downcast_mut::<VStack>()
            .expect("stack")
            .entries[0]
            .component
            .as_any_mut()
            .downcast_mut::<ScrollView>()
            .expect("inner");
        assert_eq!(inner.viewport_height(), 2);
        assert_eq!(outer.scroll_by(10), 9);
        assert_eq!(outer.scroll_top(), 1);
    }

    #[test]
    fn rebuilds_geometry_after_content_changes() {
        let mut root = VStack::new(0);
        root.add_child(
            Box::new(TuiText::new("one", 0, 0)),
            StackEntryOptions::default(),
        );
        let first = render_layout_frame(&mut root, 10, 4);
        assert_eq!(first.root.children[0].lines.as_ref().map(Vec::len), Some(1));
        root.entries[0]
            .component
            .as_any_mut()
            .downcast_mut::<TuiText>()
            .expect("text")
            .set_text("one\ntwo\nthree");
        let second = render_layout_frame(&mut root, 10, 4);
        assert_eq!(
            second.root.children[0].lines.as_ref().map(Vec::len),
            Some(3)
        );
    }

    #[test]
    fn thumb_height_matches_ts_rounding() {
        let scrollbar_background = "\x1b[48;5;1m";
        let style = {
            let scrollbar_background = scrollbar_background.to_string();
            Rc::new(move |text: &str| format!("{scrollbar_background}{text}\x1b[49m"))
        };
        let thumb_height_for = |content_height: usize| {
            let mut sized = ScrollView::new(
                Box::new(TuiText::new(
                    (0..content_height)
                        .map(|_| "x")
                        .collect::<Vec<_>>()
                        .join("\n"),
                    0,
                    0,
                )),
                ScrollViewOptions {
                    scrollbar: ScrollViewScrollbar::Auto,
                    scrollbar_style: style.clone(),
                    ..ScrollViewOptions::default()
                },
            )
            .unwrap();
            render_layout_frame(&mut sized, 6, 20);
            sized.scroll_by(1);
            render_layout_frame(&mut sized, 6, 20)
                .lines
                .iter()
                .filter(|line| line.contains(scrollbar_background))
                .count()
        };
        assert_eq!(thumb_height_for(21), 19);
        assert_eq!(thumb_height_for(40), 10);
        assert_eq!(thumb_height_for(100), 4);
        assert_eq!(thumb_height_for(400), 2);
    }
}
