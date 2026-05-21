//! Maps a project-learning record to the per-field rows embedded for learning search.

use super::EmbeddingField;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningEmbeddingSource {
    pub id: String,
    pub summary: String,
    pub body: String,
    pub tags: Vec<String>,
}

impl From<&orbit_common::types::Learning> for LearningEmbeddingSource {
    fn from(learning: &orbit_common::types::Learning) -> Self {
        Self {
            id: learning.id.clone(),
            summary: learning.summary.clone(),
            body: learning.body.clone(),
            tags: learning.scope.tags.clone(),
        }
    }
}

pub fn learning_embedding_fields(learning: &LearningEmbeddingSource) -> Vec<EmbeddingField> {
    let mut fields = Vec::new();
    push_field(&mut fields, "summary", &learning.summary);

    let mut section_count = 0;
    if let Some(rule) = markdown_section(&learning.body, "rule") {
        push_field(&mut fields, "rule", &rule);
        section_count += 1;
    }
    if let Some(why) = markdown_section(&learning.body, "why") {
        push_field(&mut fields, "why", &why);
        section_count += 1;
    }
    if let Some(how_to_apply) = markdown_section(&learning.body, "how to apply") {
        push_field(&mut fields, "how_to_apply", &how_to_apply);
        section_count += 1;
    }
    if section_count == 0 {
        push_field(&mut fields, "details", &learning.body);
    }
    if !learning.tags.is_empty() {
        push_field(&mut fields, "tags", &learning.tags.join("\n"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_embedding_fields_use_stable_section_names() {
        let source = LearningEmbeddingSource {
            id: "L-0001".to_string(),
            summary: "Use async-aware locks".to_string(),
            body: "## Rule\nNever hold std Mutex across await.\n\n## Why\nIt can deadlock.\n\n## How to apply\nDrop the guard before await.\n".to_string(),
            tags: vec!["rust".to_string(), "async".to_string()],
        };

        let field_names = learning_embedding_fields(&source)
            .into_iter()
            .map(|field| field.field)
            .collect::<Vec<_>>();

        assert_eq!(
            field_names,
            vec!["summary", "rule", "why", "how_to_apply", "tags"]
        );
    }

    #[test]
    fn learning_embedding_fields_fallback_to_details_for_plain_body() {
        let source = LearningEmbeddingSource {
            id: "L-0002".to_string(),
            summary: "Keep fixtures tracked".to_string(),
            body: "Avoid .orbit fixture paths.".to_string(),
            tags: Vec::new(),
        };

        let field_names = learning_embedding_fields(&source)
            .into_iter()
            .map(|field| field.field)
            .collect::<Vec<_>>();

        assert_eq!(field_names, vec!["summary", "details"]);
    }
}
