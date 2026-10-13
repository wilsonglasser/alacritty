//! Rows held across the grid's own motion.
//!
//! A [`Line`] names a row only until the next scroll: output pushes every
//! row up, a full scrollback drops the oldest, a scroll region moves some
//! rows and not others. Something outside the terminal that has to find a
//! row again later (where a command's output began) cannot keep a `Line`.
//!
//! An anchor is moved by the terminal itself, on exactly the paths that
//! move the selection, so it stays on its row's content. When that
//! content stops existing (scrolled out of a region, past the end of the
//! history, cleared with it, reflowed by a width change) the anchor is
//! dropped and answers `None`: it can miss, it never lands on other text.
//!
//! Anchors belong to the primary screen. The alternate screen has no
//! history and is thrown away on exit, so nothing there is worth holding,
//! and its scrolls leave the primary rows where they are.

use std::ops::Range;

use crate::index::Line;

/// A held row. Copyable, and meaningless once released.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowAnchor(u64);

/// The rows currently held.
#[derive(Debug, Default, Clone)]
pub struct RowAnchors {
    next: u64,
    held: Vec<(u64, Line)>,
}

impl RowAnchors {
    /// Hold `line` (primary screen coordinates, negative in the history).
    pub fn hold(&mut self, line: Line) -> RowAnchor {
        let id = self.next;
        self.next += 1;
        self.held.push((id, line));
        RowAnchor(id)
    }

    /// Where the held row is now, or `None` once its content is gone.
    pub fn line(&self, anchor: RowAnchor) -> Option<Line> {
        self.held.iter().find(|(id, _)| *id == anchor.0).map(|(_, line)| *line)
    }

    /// Stop holding a row.
    pub fn release(&mut self, anchor: RowAnchor) {
        self.held.retain(|(id, _)| *id != anchor.0);
    }

    /// Drop every held row.
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// Text moved up by `lines` inside `region`. A region that starts at
    /// the top of the screen pushes its rows into the history, which keeps
    /// rows down to `topmost`; any other region loses the rows that leave
    /// its top. Rows outside the region stay.
    pub(crate) fn scroll_up(&mut self, region: &Range<Line>, lines: usize, topmost: Line) {
        let lines = lines as i32;
        self.held.retain_mut(|(_, line)| {
            let inside = (*line >= region.start || region.start == 0) && *line < region.end;
            if !inside {
                return true;
            }
            *line -= lines;
            if region.start == 0 { *line >= topmost } else { *line >= region.start }
        });
    }

    /// Text moved down by `lines` inside `region`: rows pushed past its
    /// bottom are gone. The history never moves down.
    pub(crate) fn scroll_down(&mut self, region: &Range<Line>, lines: usize) {
        let lines = lines as i32;
        self.held.retain_mut(|(_, line)| {
            if *line < region.start || *line >= region.end {
                return true;
            }
            *line += lines;
            *line < region.end
        });
    }

    /// Every row moved by `delta` (a height change), kept while inside
    /// `topmost..=bottommost`.
    pub(crate) fn shift(&mut self, delta: i32, topmost: Line, bottommost: Line) {
        self.held.retain_mut(|(_, line)| {
            *line += delta;
            *line >= topmost && *line <= bottommost
        });
    }

    /// The history was cleared: rows in it are gone.
    pub(crate) fn drop_history(&mut self) {
        self.held.retain(|(_, line)| *line >= 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_screen_scroll_carries_rows_into_the_history() {
        let mut anchors = RowAnchors::default();
        let a = anchors.hold(Line(3));
        anchors.scroll_up(&(Line(0)..Line(10)), 5, Line(-100));
        assert_eq!(anchors.line(a), Some(Line(-2)));
    }

    #[test]
    fn a_row_past_the_end_of_the_history_is_gone() {
        let mut anchors = RowAnchors::default();
        let a = anchors.hold(Line(0));
        anchors.scroll_up(&(Line(0)..Line(10)), 5, Line(-4));
        assert_eq!(anchors.line(a), None);
    }

    #[test]
    fn a_partial_region_moves_its_rows_and_leaves_the_rest() {
        let mut anchors = RowAnchors::default();
        let inside = anchors.hold(Line(5));
        let above = anchors.hold(Line(1));
        let below = anchors.hold(Line(9));
        let leaving = anchors.hold(Line(3));
        anchors.scroll_up(&(Line(2)..Line(8)), 2, Line(-100));
        assert_eq!(anchors.line(inside), Some(Line(3)));
        assert_eq!(anchors.line(above), Some(Line(1)));
        assert_eq!(anchors.line(below), Some(Line(9)));
        // Row 3 moved to 1, above the region's top: scrolled away.
        assert_eq!(anchors.line(leaving), None);
    }

    #[test]
    fn a_region_at_the_top_keeps_its_fixed_bottom_rows() {
        // `apt` keeps its progress bar on the last row this way.
        let mut anchors = RowAnchors::default();
        let output = anchors.hold(Line(4));
        let bar = anchors.hold(Line(9));
        anchors.scroll_up(&(Line(0)..Line(9)), 1, Line(-100));
        assert_eq!(anchors.line(output), Some(Line(3)));
        assert_eq!(anchors.line(bar), Some(Line(9)));
    }

    #[test]
    fn a_reverse_scroll_pushes_rows_off_the_bottom() {
        let mut anchors = RowAnchors::default();
        let stays = anchors.hold(Line(2));
        let goes = anchors.hold(Line(8));
        let history = anchors.hold(Line(-3));
        anchors.scroll_down(&(Line(0)..Line(10)), 3);
        assert_eq!(anchors.line(stays), Some(Line(5)));
        assert_eq!(anchors.line(goes), None);
        assert_eq!(anchors.line(history), Some(Line(-3)));
    }

    #[test]
    fn clearing_the_history_drops_only_its_rows() {
        let mut anchors = RowAnchors::default();
        let old = anchors.hold(Line(-1));
        let screen = anchors.hold(Line(0));
        anchors.drop_history();
        assert_eq!(anchors.line(old), None);
        assert_eq!(anchors.line(screen), Some(Line(0)));
    }

    #[test]
    fn a_released_anchor_answers_none() {
        let mut anchors = RowAnchors::default();
        let a = anchors.hold(Line(2));
        anchors.release(a);
        assert_eq!(anchors.line(a), None);
    }
}
