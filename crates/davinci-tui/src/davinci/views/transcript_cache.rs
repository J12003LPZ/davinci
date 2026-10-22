//! Height metadata is retained; styled rows are retained only for one viewport.
use ratatui::text::Line;
use std::{cell::RefCell, ops::Range};

#[derive(Clone)]
pub(super) struct BlockRows {
    pub entries: Range<usize>,
    pub rows: Range<usize>,
}
#[derive(Clone, Default)]
pub(super) struct RowIndex {
    pub blocks: Vec<BlockRows>,
    pub len: usize,
    pub cached: RefCell<Option<(Range<usize>, Vec<Line<'static>>)>>,
}
impl RowIndex {
    pub fn push(&mut self, entries: Range<usize>, rows: usize) {
        let start = self.len;
        self.len = self.len.saturating_add(rows);
        self.blocks.push(BlockRows {
            entries,
            rows: start..self.len,
        });
    }
    pub fn slice(
        &self,
        range: Range<usize>,
        mut render: impl FnMut(&BlockRows, Range<usize>) -> Vec<Line<'static>>,
    ) -> Vec<Line<'static>> {
        if let Some((cached, rows)) = self
            .cached
            .borrow()
            .as_ref()
            .filter(|(cached, _)| *cached == range)
        {
            let _ = cached;
            return rows.clone();
        }
        let first = self
            .blocks
            .partition_point(|block| block.rows.end <= range.start);
        let mut rows = Vec::new();
        let mut touched = 0;
        for block in self.blocks[first..]
            .iter()
            .take_while(|block| block.rows.start < range.end)
        {
            let start = range.start.saturating_sub(block.rows.start);
            let end = range
                .end
                .min(block.rows.end)
                .saturating_sub(block.rows.start);
            rows.extend(render(block, start..end));
            touched += 1;
        }
        let bytes: usize = rows
            .iter()
            .map(|row| {
                std::mem::size_of::<Line>()
                    + row
                        .spans
                        .iter()
                        .map(|span| std::mem::size_of_val(span) + span.content.len())
                        .sum::<usize>()
            })
            .sum();
        if touched <= 128 && rows.len() <= 4096 && bytes <= 8 * 1024 * 1024 {
            *self.cached.borrow_mut() = Some((range, rows.clone()));
        }
        rows
    }
}
