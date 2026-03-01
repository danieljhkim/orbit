use std::io::{BufRead, Write};

use orbit_core::OrbitError;
use serde::Serialize;

const HEADER_CONTENT_LENGTH: &str = "content-length";

pub fn read_framed_json<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>, OrbitError> {
    let mut content_length: Option<usize> = None;
    let mut saw_any_header = false;

    loop {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|e| OrbitError::Io(e.to_string()))?;

        if bytes == 0 {
            if saw_any_header {
                return Err(OrbitError::Execution(
                    "unexpected EOF while reading MCP headers".to_string(),
                ));
            }
            return Ok(None);
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        saw_any_header = true;
        let Some((name, value)) = trimmed.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(HEADER_CONTENT_LENGTH) {
            let parsed = value.trim().parse::<usize>().map_err(|_| {
                OrbitError::Execution(format!("invalid Content-Length header value: {value}"))
            })?;
            content_length = Some(parsed);
        }
    }

    let length = content_length.ok_or_else(|| {
        OrbitError::Execution("missing Content-Length header in MCP frame".to_string())
    })?;

    let mut payload = vec![0u8; length];
    reader
        .read_exact(&mut payload)
        .map_err(|e| OrbitError::Io(e.to_string()))?;

    Ok(Some(payload))
}

pub fn write_framed_json<W: Write, T: Serialize>(
    writer: &mut W,
    payload: &T,
) -> Result<(), OrbitError> {
    let body = serde_json::to_vec(payload).map_err(|e| OrbitError::Execution(e.to_string()))?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())
        .map_err(|e| OrbitError::Io(e.to_string()))?;
    writer
        .write_all(&body)
        .map_err(|e| OrbitError::Io(e.to_string()))?;
    writer.flush().map_err(|e| OrbitError::Io(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use super::{read_framed_json, write_framed_json};

    #[test]
    fn round_trip_framed_json() {
        let mut buf = Vec::new();
        write_framed_json(&mut buf, &json!({"a": 1})).expect("write frame");

        let mut cursor = Cursor::new(buf);
        let payload = read_framed_json(&mut cursor)
            .expect("read frame")
            .expect("payload present");
        let value: serde_json::Value = serde_json::from_slice(&payload).expect("valid json");
        assert_eq!(value["a"], 1);
    }
}
