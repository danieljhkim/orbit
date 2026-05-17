use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use orbit_common::types::{LearningVoteRow, LearningVoteSummary, OrbitError};

use super::layout::{validate_learning_id, votes_jsonl_path};

pub(super) fn append_vote_row(path: &Path, row: &LearningVoteRow) -> Result<(), OrbitError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| OrbitError::Io(err.to_string()))?;
    }

    let mut encoded = serde_json::to_vec(row).map_err(|err| OrbitError::Store(err.to_string()))?;
    encoded.push(b'\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| OrbitError::Io(err.to_string()))?;
    let written = file
        .write(&encoded)
        .map_err(|err| OrbitError::Io(err.to_string()))?;
    if written != encoded.len() {
        return Err(OrbitError::Io(format!(
            "short write appending learning vote: wrote {written} of {} bytes",
            encoded.len()
        )));
    }
    Ok(())
}

pub(super) fn read_vote_rows(path: &Path) -> Result<Vec<LearningVoteRow>, OrbitError> {
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(OrbitError::Io(err.to_string())),
    };

    let reader = BufReader::new(file);
    let mut rows = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line_no = idx + 1;
        let line = line.map_err(|err| OrbitError::Io(err.to_string()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let row = serde_json::from_str::<LearningVoteRow>(trimmed).map_err(|err| {
            OrbitError::Store(format!(
                "invalid learning vote file {} line {line_no}: {err}",
                path.display()
            ))
        })?;
        rows.push(row);
    }
    Ok(rows)
}

pub(super) fn summarize_votes(rows: &[LearningVoteRow]) -> LearningVoteSummary {
    let mut by_key = BTreeMap::new();
    for row in rows {
        let key = (
            row.learning_id.clone(),
            row.voter_model.clone(),
            row.task_id.clone(),
        );
        by_key
            .entry(key)
            .and_modify(|existing: &mut chrono::DateTime<chrono::Utc>| {
                if row.voted_at < *existing {
                    *existing = row.voted_at;
                }
            })
            .or_insert(row.voted_at);
    }

    LearningVoteSummary {
        vote_count: by_key.len(),
        last_voted_at: by_key.values().max().cloned(),
    }
}

pub(super) fn deduped_vote_times(rows: &[LearningVoteRow]) -> Vec<chrono::DateTime<chrono::Utc>> {
    let mut by_key = BTreeMap::new();
    for row in rows {
        let key = (
            row.learning_id.clone(),
            row.voter_model.clone(),
            row.task_id.clone(),
        );
        by_key
            .entry(key)
            .and_modify(|existing: &mut chrono::DateTime<chrono::Utc>| {
                if row.voted_at < *existing {
                    *existing = row.voted_at;
                }
            })
            .or_insert(row.voted_at);
    }
    by_key.into_values().collect()
}

pub(super) fn validate_vote_files(root: &Path) -> Result<(), OrbitError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|err| OrbitError::Io(err.to_string()))? {
        let entry = entry.map_err(|err| OrbitError::Io(err.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|err| OrbitError::Io(err.to_string()))?;
        if !file_type.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if validate_learning_id(&id).is_err() {
            continue;
        }
        read_vote_rows(&votes_jsonl_path(root, &id))?;
    }
    Ok(())
}
