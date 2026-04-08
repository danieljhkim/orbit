use comfy_table::{Attribute, Cell, Table, presets};

pub fn build_table(headers: &[&str]) -> Table {
    let mut table = Table::new();
    table.load_preset(presets::UTF8_BORDERS_ONLY);
    table.set_header(
        headers
            .iter()
            .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
    );
    table
}

#[allow(dead_code)]
pub fn print_line(line: impl AsRef<str>) {
    println!("{}", line.as_ref());
}
