//! Tables as delimited text: CSV ([RFC 4180](https://www.rfc-editor.org/rfc/rfc4180)) and its
//! tab-separated variant.
//!
//! Every parser in the family records a merged cell once, on the cell that owns it: the
//! positions it covers — the tail of a horizontal span, and the columns a vertical span occupies
//! in the rows below — have no cell of their own. Laying such a table on its grid, and quoting
//! the fields so a spreadsheet or a CSV reader gets the values back, are the same work whatever
//! the document format was, so it is done here once.
//!
//! The layout: a merged cell's text sits at its top-left position and the positions it covers
//! are empty, so every record has the same number of fields and no value is counted twice (the
//! shape pandas reads as one value and blanks). Rows are written as given — a table's header
//! rows first, as they are.
//!
//! `std`-only, like the rest of the crate root.

/// One cell of a table row as the family's models record it: its text, and how many rows and
/// columns it owns. A span of 0 is read as 1.
///
/// The text is the cell's plain text; a cell of several paragraphs joins them with `\n`, which
/// a field keeps (quoted) rather than flattening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell<S> {
    /// The cell's plain text.
    pub text: S,
    /// Rows the cell owns, its own included.
    pub row_span: u32,
    /// Columns the cell owns, its own included.
    pub col_span: u32,
}

impl<S> Cell<S> {
    /// A cell owning `row_span` × `col_span` grid positions.
    pub fn new(text: S, row_span: u32, col_span: u32) -> Self {
        Self {
            text,
            row_span,
            col_span,
        }
    }

    /// A cell owning only its own position.
    pub fn single(text: S) -> Self {
        Self::new(text, 1, 1)
    }
}

/// The table as CSV: one record per row, fields separated by `,`, records ended by CRLF
/// (RFC 4180 §2). See [`to_delimited`].
///
/// ```
/// use unparser_shared::csv::{to_csv, Cell};
///
/// let rows = [
///     vec![Cell::single("Item"), Cell::single("Note")],
///     vec![Cell::single("Bolt, M6"), Cell::single("He said \"no\"")],
/// ];
/// assert_eq!(to_csv(rows), "Item,Note\r\n\"Bolt, M6\",\"He said \"\"no\"\"\"\r\n");
/// ```
pub fn to_csv<R, C, S>(rows: R) -> String
where
    R: IntoIterator<Item = C>,
    C: IntoIterator<Item = Cell<S>>,
    S: AsRef<str>,
{
    to_delimited(rows, ',')
}

/// The table as delimited text with `delimiter` between fields (`'\t'` for TSV), laid out on
/// its grid as the [module](self) describes.
///
/// A field holding the delimiter, a double quote or a line break is enclosed in double quotes,
/// with each quote inside doubled; every other field is written as it is.
///
/// ```
/// use unparser_shared::csv::{to_delimited, Cell};
///
/// // ┌────────┬───────────┐
/// // │ Region │   Sales   │
/// // │        ├─────┬─────┤
/// // │        │2024 │2025 │
/// // ├────────┼─────┼─────┤
/// // │ North  │ 10  │ 12  │
/// // └────────┴─────┴─────┘
/// let rows = [
///     vec![Cell::new("Region", 2, 1), Cell::new("Sales", 1, 2)],
///     vec![Cell::single("2024"), Cell::single("2025")],
///     vec![Cell::single("North"), Cell::single("10"), Cell::single("12")],
/// ];
/// assert_eq!(
///     to_delimited(rows, '\t'),
///     "Region\tSales\t\r\n\t2024\t2025\r\nNorth\t10\t12\r\n"
/// );
/// ```
pub fn to_delimited<R, C, S>(rows: R, delimiter: char) -> String
where
    R: IntoIterator<Item = C>,
    C: IntoIterator<Item = Cell<S>>,
    S: AsRef<str>,
{
    let mut out = String::new();
    for record in grid(rows) {
        for (i, field) in record.iter().enumerate() {
            if i > 0 {
                out.push(delimiter);
            }
            push_field(&mut out, field, delimiter);
        }
        out.push_str("\r\n");
    }
    out
}

fn push_field(out: &mut String, field: &str, delimiter: char) {
    let quote = field.contains(delimiter)
        || field.contains('"')
        || field.contains('\n')
        || field.contains('\r');
    if quote {
        out.push('"');
        out.push_str(&field.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(field);
    }
}

/// The rows laid out on their grid: each merged cell's text at its top-left position, the
/// positions it covers empty, every record as wide as the widest.
fn grid<R, C, S>(rows: R) -> Vec<Vec<String>>
where
    R: IntoIterator<Item = C>,
    C: IntoIterator<Item = Cell<S>>,
    S: AsRef<str>,
{
    // For each column, how many further rows a cell merged down from above still covers.
    let mut covered: Vec<usize> = Vec::new();
    let mut grid: Vec<Vec<String>> = Vec::new();
    for row in rows {
        let mut record: Vec<String> = Vec::new();
        let mut col = 0;
        for cell in row {
            skip_covered(&mut covered, &mut record, &mut col);
            record.push(cell.text.as_ref().to_string());
            let span = cell.col_span.max(1) as usize;
            record.extend(std::iter::repeat_n(String::new(), span - 1));
            if covered.len() < col + span {
                covered.resize(col + span, 0);
            }
            for c in &mut covered[col..col + span] {
                *c = cell.row_span.max(1) as usize - 1;
            }
            col += span;
        }
        // Columns past the row's last cell that a cell above still covers.
        while col < covered.len() {
            if covered[col] > 0 {
                covered[col] -= 1;
            }
            record.push(String::new());
            col += 1;
        }
        grid.push(record);
    }
    let width = grid.iter().map(Vec::len).max().unwrap_or(0);
    for record in &mut grid {
        record.resize(width, String::new());
    }
    grid
}

/// Pass the columns, from `col` on, that a cell merged down from a row above still covers,
/// leaving each one empty in `record`.
fn skip_covered(covered: &mut [usize], record: &mut Vec<String>, col: &mut usize) {
    while covered.get(*col).is_some_and(|&n| n > 0) {
        covered[*col] -= 1;
        record.push(String::new());
        *col += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv(rows: &[&[Cell<&str>]]) -> String {
        to_csv(rows.iter().map(|row| row.iter().copied()))
    }

    #[test]
    fn plain_fields_are_written_as_they_are() {
        assert_eq!(
            csv(&[&[Cell::single("a"), Cell::single("b c")]]),
            "a,b c\r\n"
        );
    }

    #[test]
    fn a_field_with_a_delimiter_quote_or_line_break_is_quoted() {
        assert_eq!(
            csv(&[&[
                Cell::single("1,5"),
                Cell::single("say \"hi\""),
                Cell::single("two\nlines"),
                Cell::single("cr\rhere"),
            ]]),
            "\"1,5\",\"say \"\"hi\"\"\",\"two\nlines\",\"cr\rhere\"\r\n"
        );
    }

    #[test]
    fn the_delimiter_decides_what_needs_quoting() {
        let row = [Cell::single("a,b"), Cell::single("c\td")];
        assert_eq!(to_delimited([row], '\t'), "a,b\t\"c\td\"\r\n");
    }

    #[test]
    fn a_horizontal_span_leaves_the_positions_it_covers_empty() {
        assert_eq!(
            csv(&[
                &[Cell::new("wide", 1, 3)],
                &[Cell::single("a"), Cell::single("b"), Cell::single("c")],
            ]),
            "wide,,\r\na,b,c\r\n"
        );
    }

    #[test]
    fn a_vertical_span_pushes_the_rows_below_rightward() {
        // The second and third rows have no cell for column 0: the merge owns it.
        assert_eq!(
            csv(&[
                &[Cell::new("tall", 3, 1), Cell::single("a")],
                &[Cell::single("b")],
                &[Cell::single("c")],
                &[Cell::single("d"), Cell::single("e")],
            ]),
            "tall,a\r\n,b\r\n,c\r\nd,e\r\n"
        );
    }

    #[test]
    fn a_cell_merged_down_at_the_row_end_leaves_the_next_row_aligned() {
        assert_eq!(
            csv(&[
                &[Cell::single("a"), Cell::new("tall", 2, 1)],
                &[Cell::single("b")],
                &[Cell::single("c"), Cell::single("d")],
            ]),
            "a,tall\r\nb,\r\nc,d\r\n"
        );
    }

    #[test]
    fn a_block_merge_covers_rows_and_columns() {
        assert_eq!(
            csv(&[
                &[Cell::new("block", 2, 2), Cell::single("x")],
                &[Cell::single("y")],
                &[Cell::single("p"), Cell::single("q"), Cell::single("r")],
            ]),
            "block,,x\r\n,,y\r\np,q,r\r\n"
        );
    }

    #[test]
    fn short_rows_are_padded_to_the_widest() {
        assert_eq!(
            csv(&[
                &[Cell::single("a")],
                &[Cell::single("b"), Cell::single("c")]
            ]),
            "a,\r\nb,c\r\n"
        );
    }

    #[test]
    fn a_zero_span_is_read_as_one() {
        assert_eq!(
            csv(&[
                &[Cell::new("z", 0, 0), Cell::single("a")],
                &[Cell::single("b"), Cell::single("c")]
            ]),
            "z,a\r\nb,c\r\n"
        );
    }

    #[test]
    fn an_empty_table_is_empty_text() {
        assert_eq!(csv(&[]), "");
    }

    #[test]
    fn owned_text_works_as_well_as_borrowed() {
        let rows = vec![vec![Cell::single(String::from("own"))]];
        assert_eq!(to_csv(rows), "own\r\n");
    }
}
