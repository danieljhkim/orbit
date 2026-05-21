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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_embedding_fields_use_stable_names() {
        let source = DocEmbeddingSource {
            path: "docs/example.md".to_string(),
            title: "Example Summary".to_string(),
            tags: vec!["search".to_string(), "docs".to_string()],
            body: "Body text".to_string(),
        };

        let field_names = doc_embedding_fields(&source)
            .into_iter()
            .map(|field| field.field)
            .collect::<Vec<_>>();

        assert_eq!(field_names, vec!["path", "title", "tags", "body"]);
    }
}
