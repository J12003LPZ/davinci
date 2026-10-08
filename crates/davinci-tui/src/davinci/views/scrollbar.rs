//! The conversation's scrollbar, drawn in the window's last column.
//!
//! DaVinci runs on the alternate screen, where the terminal shows no
//! scrollbar of its own (Windows Terminal hides it there), so the
//! conversation draws one. The geometry follows pi's alt-screen scroll view
//! (`crate::layout::get_scrollbar_geometry`): the thumb is `track² / content`
//! rows, never under two, and a drag keeps the row the thumb was grabbed by
//! under the pointer.
//!
//! Positions here are counted as `offset` rows up from the newest row, the
//! way the conversation is scrolled; `0` follows new output.

use ratatui::text::Span;

use crate::davinci::theme::Theme;
use crate::davinci::ui::span;

pub const TRACK: &str = "│";
pub const THUMB: &str = "┃";
/// Narrower than this the bar would cost too much of the line.
pub const MIN_WIDTH: u16 = 20;
/// Rows one wheel notch moves, as most terminals scroll.
pub const WHEEL_ROWS: isize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scrollbar {
    /// Screen column of the bar.
    pub column: u16,
    /// Screen row of the track's first cell.
    pub top: u16,
    /// Track rows, the same as the visible conversation rows.
    pub height: u16,
    /// Rows the whole conversation takes.
    pub total: usize,
    /// Rows scrolled up from the newest.
    pub offset: usize,
    /// Whether the bar is on screen. Too narrow a window scrolls without one.
    pub drawn: bool,
}

impl Scrollbar {
    /// The furthest the conversation scrolls up.
    pub fn max_offset(&self) -> usize {
        self.total.saturating_sub(usize::from(self.height))
    }

    /// The thumb's first track row and its length.
    pub fn thumb(&self) -> (u16, u16) {
        let track = usize::from(self.height);
        let min = 2.min(track);
        let len = min
            .max(track.min(((track * track) as f64 / self.total.max(1) as f64).round() as usize));
        let room = track - len;
        let max = self.max_offset();
        let top = if max == 0 {
            room
        } else {
            let from_top = max - self.offset.min(max);
            ((from_top as f64 / max as f64) * room as f64).round() as usize
        };
        (top as u16, len as u16)
    }

    /// Whether a screen cell is on the track.
    pub fn contains(&self, column: u16, row: u16) -> bool {
        self.drawn && column == self.column && row >= self.top && row < self.top + self.height
    }

    /// Where the thumb was grabbed, in rows from its first row. A press off
    /// the thumb grabs its middle, so the thumb jumps to center on the press.
    pub fn grab(&self, row: u16) -> u16 {
        let (top, len) = self.thumb();
        let at = row.saturating_sub(self.top);
        if at >= top && at < top + len {
            at - top
        } else {
            len / 2
        }
    }

    /// The offset that puts the thumb's grabbed row under screen row `row`.
    pub fn offset_at(&self, row: u16, grab: u16) -> usize {
        let (_, len) = self.thumb();
        let room = self.height.saturating_sub(len);
        let max = self.max_offset();
        if room == 0 {
            return self.offset.min(max);
        }
        let thumb_top = i32::from(row) - i32::from(self.top) - i32::from(grab);
        let thumb_top = thumb_top.clamp(0, i32::from(room)) as f64;
        let from_top = (thumb_top / f64::from(room) * max as f64).round() as usize;
        max - from_top.min(max)
    }

    /// The cell for track row `row`.
    pub fn cell(&self, theme: &Theme, row: u16) -> Span<'static> {
        let cc = theme.cc();
        let (top, len) = self.thumb();
        if row >= top && row < top + len {
            span(THUMB, cc.inactive)
        } else {
            span(TRACK, cc.subtle)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(total: usize, offset: usize) -> Scrollbar {
        Scrollbar {
            column: 79,
            top: 0,
            height: 20,
            total,
            offset,
            drawn: true,
        }
    }

    #[test]
    fn following_the_newest_puts_the_thumb_at_the_bottom() {
        let (top, len) = bar(200, 0).thumb();
        assert_eq!(len, 2); // 20 * 20 / 200
        assert_eq!(top + len, 20);
    }

    #[test]
    fn scrolled_all_the_way_up_puts_the_thumb_at_the_top() {
        let b = bar(200, 180);
        assert_eq!(b.max_offset(), 180);
        assert_eq!(b.thumb().0, 0);
    }

    #[test]
    fn the_thumb_is_the_visible_share_of_the_track() {
        assert_eq!(bar(40, 0).thumb().1, 10);
        assert_eq!(bar(10_000, 0).thumb().1, 2);
    }

    #[test]
    fn dragging_the_thumb_to_an_end_reaches_that_end() {
        let b = bar(200, 50);
        let grab = b.grab(b.thumb().0);
        assert_eq!(grab, 0);
        assert_eq!(b.offset_at(0, 0), 180);
        assert_eq!(b.offset_at(19, 0), 0);
        // Far past either end clamps.
        assert_eq!(b.offset_at(500, 0), 0);
    }

    #[test]
    fn a_drag_round_trips_the_offset_it_started_from() {
        for offset in [0, 1, 37, 90, 179, 180] {
            let b = bar(200, offset);
            let (top, _) = b.thumb();
            let back = b.offset_at(top, 0);
            // The thumb has 18 positions for 180 offsets: within one step.
            assert!(back.abs_diff(offset) <= 10, "{offset} -> {back}");
        }
    }

    #[test]
    fn a_press_off_the_thumb_grabs_its_middle() {
        let b = bar(200, 0);
        assert_eq!(b.grab(0), 1);
    }

    #[test]
    fn only_the_track_column_and_rows_are_hit() {
        let b = Scrollbar {
            top: 3,
            ..bar(200, 0)
        };
        assert!(b.contains(79, 3));
        assert!(b.contains(79, 22));
        assert!(!b.contains(79, 23));
        assert!(!b.contains(79, 2));
        assert!(!b.contains(78, 10));
    }

    #[test]
    fn a_bar_that_is_not_drawn_is_never_hit() {
        let b = Scrollbar {
            drawn: false,
            ..bar(200, 0)
        };
        assert!(!b.contains(79, 5));
    }
}
