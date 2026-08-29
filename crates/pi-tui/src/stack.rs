//! VStack / HStack matching `vendor/pi/packages/tui/src/components/{stack,v-stack,h-stack}.ts`.

use crate::ansi::visible_width;
use crate::overlay::composite_tui_line;
use crate::render::Component;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackBasis {
    Auto,
    Fixed(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackAlign {
    Stretch,
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutViewport {
    pub width: usize,
    pub height: usize,
}

#[derive(Default)]
pub struct StackEntryOptions {
    pub basis: Option<StackBasis>,
    pub grow: Option<usize>,
    pub shrink: Option<usize>,
    pub min_size: Option<usize>,
    pub max_size: Option<usize>,
    pub visible: Option<Box<dyn Fn(LayoutViewport) -> bool>>,
}

pub struct StackLayoutEntry {
    pub component: Box<dyn Component>,
    pub basis: Option<StackBasis>,
    pub grow: Option<usize>,
    pub shrink: Option<usize>,
    pub min_size: Option<usize>,
    pub max_size: Option<usize>,
    pub visible: Option<Box<dyn Fn(LayoutViewport) -> bool>>,
}

fn normalize_size(value: Option<usize>, fallback: usize) -> usize {
    value.unwrap_or(fallback)
}

fn clamp_size(size: usize, entry: &StackLayoutEntry) -> usize {
    let min = entry.min_size.unwrap_or(0);
    let max = entry.max_size.unwrap_or(usize::MAX).max(min);
    size.min(max).max(min)
}

fn visible_stack_entries(
    entries: &[StackLayoutEntry],
    viewport: LayoutViewport,
) -> Vec<(usize, &StackLayoutEntry)> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry
                .visible
                .as_ref()
                .map_or(true, |visible| visible(viewport))
        })
        .collect()
}

fn distribute(sizes: &mut [usize], entries: &[&StackLayoutEntry], amount: usize, grow: bool) {
    let mut remaining = amount;
    while remaining > 0 {
        let candidates: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(index, entry)| {
                if grow {
                    (entry.grow.unwrap_or(0) > 0)
                        && sizes[*index] < entry.max_size.unwrap_or(usize::MAX)
                } else {
                    (entry.shrink.unwrap_or(1) > 0) && sizes[*index] > entry.min_size.unwrap_or(0)
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
                    entries[index].grow.unwrap_or(0)
                } else {
                    entries[index].shrink.unwrap_or(1) * sizes[index].max(1)
                }
            })
            .sum();
        let mut distributed = 0usize;
        for index in candidates {
            if remaining == 0 {
                break;
            }
            let weight = if grow {
                entries[index].grow.unwrap_or(0)
            } else {
                entries[index].shrink.unwrap_or(1) * sizes[index].max(1)
            };
            let proposed = ((remaining * weight) / total_weight).max(1);
            let capacity = if grow {
                entry_max(entries[index]).saturating_sub(sizes[index])
            } else {
                sizes[index].saturating_sub(entries[index].min_size.unwrap_or(0))
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

fn entry_max(entry: &StackLayoutEntry) -> usize {
    entry.max_size.unwrap_or(usize::MAX)
}

pub fn allocate_stack_sizes(
    entries: &[&StackLayoutEntry],
    intrinsic_sizes: &[usize],
    available_size: Option<usize>,
    gap: usize,
) -> Vec<usize> {
    let mut sizes: Vec<usize> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let raw = match entry.basis {
                None | Some(StackBasis::Auto) => intrinsic_sizes.get(index).copied().unwrap_or(0),
                Some(StackBasis::Fixed(basis)) => basis,
            };
            clamp_size(raw, entry)
        })
        .collect();
    let Some(available) = available_size else {
        return sizes;
    };
    let content_size = available.saturating_sub(entries.len().saturating_sub(1) * gap);
    let total: usize = sizes.iter().sum();
    match total.cmp(&content_size) {
        std::cmp::Ordering::Less => distribute(&mut sizes, entries, content_size - total, true),
        std::cmp::Ordering::Greater => distribute(&mut sizes, entries, total - content_size, false),
        std::cmp::Ordering::Equal => {}
    }
    sizes
}

fn add_entry(
    entries: &mut Vec<StackLayoutEntry>,
    component: Box<dyn Component>,
    options: StackEntryOptions,
) {
    entries.push(StackLayoutEntry {
        component,
        basis: options.basis,
        grow: options.grow.map(|value| normalize_size(Some(value), 0)),
        shrink: options.shrink.map(|value| normalize_size(Some(value), 1)),
        min_size: options.min_size.map(|value| normalize_size(Some(value), 0)),
        max_size: options
            .max_size
            .map(|value| normalize_size(Some(value), usize::MAX)),
        visible: options.visible,
    });
}

pub struct VStack {
    pub entries: Vec<StackLayoutEntry>,
    pub gap: usize,
}

impl VStack {
    pub fn new(gap: usize) -> Self {
        Self {
            entries: Vec::new(),
            gap,
        }
    }

    pub fn add_child(&mut self, component: Box<dyn Component>, options: StackEntryOptions) {
        add_entry(&mut self.entries, component, options);
    }
}

impl Component for VStack {
    fn render(&self, width: usize) -> Vec<String> {
        let viewport = LayoutViewport {
            width: width.max(1),
            height: usize::MAX,
        };
        let visible = visible_stack_entries(&self.entries, viewport);
        let rendered: Vec<Vec<String>> = visible
            .iter()
            .map(|(_, entry)| entry.component.render(viewport.width))
            .collect();
        let refs: Vec<&StackLayoutEntry> = visible.iter().map(|(_, entry)| *entry).collect();
        let intrinsic: Vec<usize> = rendered.iter().map(Vec::len).collect();
        let sizes = allocate_stack_sizes(&refs, &intrinsic, None, self.gap);
        let mut lines = Vec::new();
        for (index, child_lines) in rendered.iter().enumerate() {
            if index > 0 {
                for _ in 0..self.gap {
                    lines.push(String::new());
                }
            }
            let size = sizes.get(index).copied().unwrap_or(0);
            let kept: Vec<String> = child_lines.iter().take(size).cloned().collect();
            lines.extend(kept.iter().cloned());
            for _ in kept.len()..size {
                lines.push(String::new());
            }
        }
        lines
    }

    fn invalidate(&mut self) {
        for entry in &mut self.entries {
            entry.component.invalidate();
        }
    }
}

pub struct HStack {
    pub entries: Vec<StackLayoutEntry>,
    pub gap: usize,
    pub align: StackAlign,
}

impl HStack {
    pub fn new(gap: usize, align: StackAlign) -> Self {
        Self {
            entries: Vec::new(),
            gap,
            align,
        }
    }

    pub fn add_child(&mut self, component: Box<dyn Component>, options: StackEntryOptions) {
        add_entry(&mut self.entries, component, options);
    }
}

impl Component for HStack {
    fn render(&self, width: usize) -> Vec<String> {
        let safe_width = width.max(1);
        let viewport = LayoutViewport {
            width: safe_width,
            height: usize::MAX,
        };
        let visible = visible_stack_entries(&self.entries, viewport);
        if visible.is_empty() {
            return Vec::new();
        }
        let intrinsic_widths: Vec<usize> = visible
            .iter()
            .map(|(_, entry)| {
                entry
                    .component
                    .render(safe_width)
                    .iter()
                    .map(|line| visible_width(line))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let refs: Vec<&StackLayoutEntry> = visible.iter().map(|(_, entry)| *entry).collect();
        let widths = allocate_stack_sizes(&refs, &intrinsic_widths, Some(safe_width), self.gap);
        let rendered: Vec<Vec<String>> = visible
            .iter()
            .enumerate()
            .map(|(index, (_, entry))| {
                if widths[index] == 0 {
                    Vec::new()
                } else {
                    entry.component.render(widths[index])
                }
            })
            .collect();
        let height = rendered.iter().map(Vec::len).max().unwrap_or(0);
        let mut result = vec![String::new(); height];
        let mut x = 0usize;
        for (index, lines) in rendered.iter().enumerate() {
            let child_width = widths[index];
            let offset = match self.align {
                StackAlign::Center => height.saturating_sub(lines.len()) / 2,
                StackAlign::End => height.saturating_sub(lines.len()),
                StackAlign::Stretch | StackAlign::Start => 0,
            };
            for (row, line) in lines.iter().enumerate() {
                let target = row + offset;
                if target < result.len() {
                    result[target] =
                        composite_tui_line(&result[target], line, x, child_width, safe_width);
                }
            }
            x += child_width + self.gap;
        }
        result
    }

    fn invalidate(&mut self) {
        for entry in &mut self.entries {
            entry.component.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_text::TuiText;

    fn trim_lines(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                crate::render::strip_terminal_sequences(line)
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn vstack_omits_gaps_around_invisible_entries() {
        let mut stack = VStack::new(1);
        stack.add_child(
            Box::new(TuiText::new("one", 0, 0)),
            StackEntryOptions::default(),
        );
        stack.add_child(
            Box::new(TuiText::new("hidden", 0, 0)),
            StackEntryOptions {
                visible: Some(Box::new(|_| false)),
                ..StackEntryOptions::default()
            },
        );
        stack.add_child(
            Box::new(TuiText::new("two", 0, 0)),
            StackEntryOptions::default(),
        );
        assert_eq!(trim_lines(&stack.render(10)), vec!["one", "", "two"]);
    }

    #[test]
    fn hstack_composes_children_at_allocated_widths() {
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
        assert_eq!(trim_lines(&stack.render(12)), vec!["left  right"]);
    }

    #[test]
    fn hstack_does_not_paint_zero_width_children() {
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
        assert_eq!(trim_lines(&stack.render(5)), vec!["shown"]);
    }

    #[test]
    fn allocate_grows_and_shrinks_like_ts() {
        let top = StackLayoutEntry {
            component: Box::new(TuiText::new("top", 0, 0)),
            basis: Some(StackBasis::Fixed(1)),
            grow: Some(0),
            shrink: Some(0),
            min_size: None,
            max_size: None,
            visible: None,
        };
        let body = StackLayoutEntry {
            component: Box::new(TuiText::new("body", 0, 0)),
            basis: Some(StackBasis::Fixed(0)),
            grow: Some(1),
            shrink: None,
            min_size: None,
            max_size: None,
            visible: None,
        };
        let refs = [&top, &body];
        assert_eq!(allocate_stack_sizes(&refs, &[1, 1], Some(4), 0), vec![1, 3]);
    }
}
