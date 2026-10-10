//! Where a table's cells sit on its grid.
//!
//! Every parser in the family records a merged cell once, on the cell that owns it: the
//! positions it covers — the tail of a horizontal span, and the columns a vertical span occupies
//! in the rows below — have no cell of their own. So a cell's index in its row is not its grid
//! column once a vertical span from a row above sits to its left, and a row's cell count is not
//! the table's width once a cell spans columns. [`place`] walks the spans to recover both — the
//! same walk whatever the document format was, done here once.
//!
//! `std`-only, like the rest of the crate root.

/// How many grid rows and columns a cell owns, its own position included. A span of 0 is read
/// as 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// Rows the cell owns.
    pub rows: u32,
    /// Columns the cell owns.
    pub cols: u32,
}

impl Span {
    /// A cell owning `rows` × `cols` grid positions.
    pub fn new(rows: u32, cols: u32) -> Self {
        Self { rows, cols }
    }

    /// A cell owning only its own position.
    pub fn single() -> Self {
        Self::new(1, 1)
    }
}

/// Where each cell of a table sits: see [`place`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Placement {
    /// The grid column each cell starts in, one `Vec` per row, parallel to the row's cells.
    pub columns: Vec<Vec<usize>>,
    /// How many grid columns the table occupies: the furthest any cell reaches.
    pub width: usize,
}

/// The grid column each cell starts in, and the table's width, for rows given as their cells'
/// spans in reading order.
///
/// A cell starts at the first column, from where the row has got to, that no cell merged down
/// from a row above still covers. The width is the furthest any cell reaches — which can be
/// further than a row's own spans add up to, since a vertical span from above pushes the row's
/// cells rightward.
///
/// ```
/// use unparser_shared::grid::{place, Span};
///
/// // ┌────────┬───────────┐
/// // │ Region │   Sales   │
/// // │        ├─────┬─────┤
/// // │        │2024 │2025 │
/// // └────────┴─────┴─────┘
/// let placement = place([
///     vec![Span::new(2, 1), Span::new(1, 2)],
///     vec![Span::single(), Span::single()],
/// ]);
/// assert_eq!(placement.columns, vec![vec![0, 1], vec![1, 2]]);
/// assert_eq!(placement.width, 3);
/// ```
pub fn place<R, C>(rows: R) -> Placement
where
    R: IntoIterator<Item = C>,
    C: IntoIterator<Item = Span>,
{
    // For each column, how many further rows a cell merged down from above still covers.
    let mut covered: Vec<usize> = Vec::new();
    let mut placement = Placement::default();
    for row in rows {
        let mut columns = Vec::new();
        let mut col = 0;
        for span in row {
            // Pass the columns a cell from above still covers; this row uses them up once.
            while covered.get(col).is_some_and(|&n| n > 0) {
                covered[col] -= 1;
                col += 1;
            }
            let (rows, cols) = (span.rows.max(1) as usize, span.cols.max(1) as usize);
            columns.push(col);
            if covered.len() < col + cols {
                covered.resize(col + cols, 0);
            }
            for c in &mut covered[col..col + cols] {
                *c = rows - 1;
            }
            col += cols;
            placement.width = placement.width.max(col);
        }
        // Columns past the row's last cell that a cell above still covers belong to this row.
        for c in covered.iter_mut().skip(col) {
            *c = c.saturating_sub(1);
        }
        placement.columns.push(columns);
    }
    placement
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(rows: &[&[(u32, u32)]]) -> Vec<Vec<Span>> {
        rows.iter()
            .map(|r| {
                r.iter()
                    .map(|&(rows, cols)| Span::new(rows, cols))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn plain_cells_sit_at_their_index() {
        let p = place(spans(&[&[(1, 1), (1, 1)], &[(1, 1), (1, 1)]]));
        assert_eq!(p.columns, vec![vec![0, 1], vec![0, 1]]);
        assert_eq!(p.width, 2);
    }

    /// A first row that merges across: its cell count is not the table's width.
    #[test]
    fn a_horizontal_span_in_the_first_row_widens_the_table() {
        let p = place(spans(&[&[(2, 1), (1, 2)], &[(1, 1), (1, 1)]]));
        assert_eq!(p.width, 3);
    }

    #[test]
    fn a_vertical_span_pushes_the_rows_below_rightward() {
        let p = place(spans(&[
            &[(3, 1), (1, 1)],
            &[(1, 1)],
            &[(1, 1)],
            &[(1, 1), (1, 1)],
        ]));
        assert_eq!(p.columns, vec![vec![0, 1], vec![1], vec![1], vec![0, 1]]);
    }

    #[test]
    fn a_cell_merged_down_at_the_row_end_is_used_up_by_the_rows_it_covers() {
        let p = place(spans(&[&[(1, 1), (2, 1)], &[(1, 1)], &[(1, 1), (1, 1)]]));
        assert_eq!(p.columns, vec![vec![0, 1], vec![0], vec![0, 1]]);
        assert_eq!(p.width, 2);
    }

    #[test]
    fn a_block_merge_covers_rows_and_columns() {
        let p = place(spans(&[
            &[(2, 2), (1, 1)],
            &[(1, 1)],
            &[(1, 1), (1, 1), (1, 1)],
        ]));
        assert_eq!(p.columns, vec![vec![0, 2], vec![2], vec![0, 1, 2]]);
        assert_eq!(p.width, 3);
    }

    #[test]
    fn a_zero_span_is_read_as_one() {
        let p = place(spans(&[&[(0, 0), (0, 0)]]));
        assert_eq!(p.columns, vec![vec![0, 1]]);
        assert_eq!(p.width, 2);
    }

    #[test]
    fn an_empty_table_has_no_columns() {
        assert_eq!(place(Vec::<Vec<Span>>::new()), Placement::default());
    }
}
