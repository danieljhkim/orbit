use comfy_table::{Attribute, Cell, ContentArrangement, Row, Table, presets};

pub fn build_table(headers: &[&str]) -> Table {
    let mut table = Table::new();
    table.load_preset(presets::UTF8_BORDERS_ONLY);
    table.set_content_arrangement(ContentArrangement::DynamicFullWidth);
    table.set_truncation_indicator("…");
    table.set_header(
        headers
            .iter()
            .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
    );
    table
}

pub fn add_single_line_row(table: &mut Table, cells: Vec<Cell>) {
    let mut row = Row::from(cells);
    row.max_height(1);
    table.add_row(row);
}

#[allow(dead_code)]
pub fn print_line(line: impl AsRef<str>) {
    println!("{}", line.as_ref());
}
