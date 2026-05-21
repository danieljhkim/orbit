//! Maps a docs corpus record to the per-field rows embedded for doc search.

use super::EmbeddingField;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocEmbeddingSource {
    pub path: String,
    pub title: String,
    pub tags: Vec<String>,
    pub body: String,
}

pub fn doc_embedding_fields(doc: &DocEmbeddingSource) -> Vec<EmbeddingField> {
    let mut fields = Vec::new();
    push_field(&mut fields, "path", &doc.path);
    push_field(&mut fields, "title", &doc.title);
    if !doc.tags.is_empty() {
        push_field(&mut fields, "tags", &doc.tags.join("\n"));
    }
    push_field(&mut fields, "body", &doc.body);
    fields
}

fn push_field(fields: &mut Vec<EmbeddingField>, field: impl Into<String>, text: &str) {
    if !text.trim().is_empty() {
        fields.push(EmbeddingField::new(field, text.trim().to_string()));
    }
}
