//! Paragraph-boundary chunker for fields that exceed a model's context window
//! (per ADR-003). The entry point is [`chunk_text`]; everything else is the
//! private machinery for splitting paragraphs, padding overlap, and falling
//! back to word-level splitting when a single paragraph is too long.

use orbit_common::types::OrbitError;

use crate::Embedder;

pub fn chunk_text(
    text: &str,
    embedder: &dyn Embedder,
    target_tokens: usize,
    overlap_tokens: usize,
) -> Result<Vec<String>, OrbitError> {
    let target_tokens = target_tokens.max(1);
    if embedder.token_count(text)? <= target_tokens {
        return Ok(vec![text.trim().to_string()]);
    }

    let paragraphs = split_paragraphs(text);
    let mut chunks = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut current_tokens = 0;

    for paragraph in paragraphs {
        let paragraph_tokens = embedder.token_count(&paragraph)?;
        if paragraph_tokens > target_tokens {
            if !current.is_empty() {
                chunks.push(current.join("\n\n"));
                current = overlap_tail(&current, embedder, overlap_tokens)?;
                current_tokens = count_parts(&current, embedder)?;
            }
            for piece in split_long_paragraph(&paragraph, embedder, target_tokens, overlap_tokens)?
            {
                chunks.push(piece);
            }
            current.clear();
            continue;
        }

        if !current.is_empty() && current_tokens + paragraph_tokens > target_tokens {
            chunks.push(current.join("\n\n"));
            current = overlap_tail(&current, embedder, overlap_tokens)?;
            current_tokens = count_parts(&current, embedder)?;
        }
        current.push(paragraph);
        current_tokens += paragraph_tokens;
    }

    if !current.is_empty() {
        chunks.push(current.join("\n\n"));
    }
    Ok(chunks)
}

fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line.trim().to_string());
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }
    paragraphs
}

fn overlap_tail(
    paragraphs: &[String],
    embedder: &dyn Embedder,
    overlap_tokens: usize,
) -> Result<Vec<String>, OrbitError> {
    if overlap_tokens == 0 {
        return Ok(Vec::new());
    }
    let mut selected = Vec::new();
    let mut total = 0;
    for paragraph in paragraphs.iter().rev() {
        let tokens = embedder.token_count(paragraph)?;
        if total > 0 && total + tokens > overlap_tokens {
            break;
        }
        selected.push(paragraph.clone());
        total += tokens;
        if total >= overlap_tokens {
            break;
        }
    }
    selected.reverse();
    Ok(selected)
}

fn count_parts(parts: &[String], embedder: &dyn Embedder) -> Result<usize, OrbitError> {
    parts
        .iter()
        .try_fold(0, |sum, part| Ok(sum + embedder.token_count(part)?))
}

fn split_long_paragraph(
    paragraph: &str,
    embedder: &dyn Embedder,
    target_tokens: usize,
    overlap_tokens: usize,
) -> Result<Vec<String>, OrbitError> {
    let words = paragraph.split_whitespace().collect::<Vec<_>>();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < words.len() {
        let mut end = start + 1;
        while end <= words.len() {
            let candidate = words[start..end].join(" ");
            if embedder.token_count(&candidate)? > target_tokens {
                break;
            }
            end += 1;
        }
        let chunk_end = (end - 1).max(start + 1).min(words.len());
        chunks.push(words[start..chunk_end].join(" "));
        if chunk_end == words.len() {
            break;
        }
        let mut overlap_start = chunk_end;
        while overlap_start > start {
            let candidate = words[overlap_start - 1..chunk_end].join(" ");
            if embedder.token_count(&candidate)? > overlap_tokens {
                break;
            }
            overlap_start -= 1;
        }
        start = overlap_start.max(start + 1);
    }
    Ok(chunks)
}
