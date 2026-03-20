use colored::Colorize;
use comfy_table::{Attribute, Cell, Color};

pub fn status_color_cell(status: &str) -> Cell {
    let cell = Cell::new(status);
    match status {
        "proposed" => cell.fg(Color::Yellow),
        "in-progress" => cell.fg(Color::Cyan),
        "review" => cell.fg(Color::Magenta),
        "done" => cell.fg(Color::Green).add_attribute(Attribute::Bold),
        "rejected" | "blocked" => cell.fg(Color::Red),
        "archived" => cell.add_attribute(Attribute::Dim),
        _ => cell,
    }
}

pub fn priority_color_cell(priority: &str) -> Cell {
    let cell = Cell::new(priority);
    match priority {
        "high" => cell.fg(Color::Red),
        "medium" => cell.fg(Color::Yellow),
        "low" => cell.add_attribute(Attribute::Dim),
        _ => cell,
    }
}

pub fn job_state_color_cell(state: &str) -> Cell {
    let cell = Cell::new(state);
    match state {
        "success" | "active" => cell.fg(Color::Green),
        "failed" | "error" | "disabled" => cell.fg(Color::Red),
        "running" => cell.fg(Color::Cyan),
        "pending" => cell.fg(Color::Yellow),
        _ => cell,
    }
}

pub fn doctor_status_color_cell(status: &str) -> Cell {
    let cell = Cell::new(status);
    match status {
        "ok" => cell.fg(Color::Green),
        "warning" => cell.fg(Color::Yellow),
        "ERROR" | "error" => cell.fg(Color::Red).add_attribute(Attribute::Bold),
        _ => cell,
    }
}

pub fn status_color(status: &str) -> String {
    match status {
        "proposed" => status.yellow().to_string(),
        "backlog" => status.to_string(),
        "in-progress" => status.cyan().to_string(),
        "review" => status.magenta().to_string(),
        "done" => status.green().bold().to_string(),
        "rejected" | "blocked" => status.red().to_string(),
        "archived" => status.dimmed().to_string(),
        _ => status.to_string(),
    }
}

pub fn priority_color(priority: &str) -> String {
    match priority {
        "high" => priority.red().to_string(),
        "medium" => priority.yellow().to_string(),
        "low" => priority.dimmed().to_string(),
        _ => priority.to_string(),
    }
}

pub fn job_state_color(state: &str) -> String {
    match state {
        "success" | "active" => state.green().to_string(),
        "failed" | "error" | "disabled" => state.red().to_string(),
        "running" => state.cyan().to_string(),
        "pending" => state.yellow().to_string(),
        _ => state.to_string(),
    }
}

pub fn bold(text: &str) -> String {
    text.bold().to_string()
}

pub fn dimmed(text: &str) -> String {
    text.dimmed().to_string()
}
