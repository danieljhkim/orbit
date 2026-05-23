//! Maps an ADR record to the per-field rows embedded for ADR search.

use super::EmbeddingField;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdrEmbeddingSource {
    pub id: String,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
}

pub fn adr_embedding_fields(adr: &AdrEmbeddingSource) -> Vec<EmbeddingField> {
    let mut fields = Vec::new();
    push_field(&mut fields, "title", &adr.title);
    if let Some(decision) = markdown_section(&adr.body, "decision") {
        push_field(&mut fields, "decision", &decision);
    }
    if let Some(context) = markdown_section(&adr.body, "context") {
        push_field(&mut fields, "context", &context);
    }
    if let Some(consequences) = markdown_section(&adr.body, "consequences") {
        push_field(&mut fields, "consequences", &consequences);
    }
    if !adr.tags.is_empty() {
        push_field(&mut fields, "tags", &adr.tags.join("\n"));
    }
    fields
}

fn push_field(fields: &mut Vec<EmbeddingField>, field: impl Into<String>, text: &str) {
    if !text.trim().is_empty() {
        fields.push(EmbeddingField::new(field, text.trim().to_string()));
    }
}

fn markdown_section(body: &str, wanted: &str) -> Option<String> {
    let mut active = false;
    let mut lines = Vec::new();
    for line in body.lines() {
        if let Some(title) = markdown_heading_title(line) {
            if active {
                break;
            }
            active = normalize_heading(title) == normalize_heading(wanted);
            continue;
        }
        if active {
            lines.push(line);
        }
    }
    let section = lines.join("\n");
    if section.trim().is_empty() {
        None
    } else {
        Some(section.trim().to_string())
    }
}

fn markdown_heading_title(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    Some(rest.trim())
}

fn normalize_heading(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', " ")
}
