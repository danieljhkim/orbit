//! Unit tests for `schema` — sibling layout under store/tests/.

use rusqlite::Connection;

use super::super::schema::{ensure_vector_schema, legacy_task_fts_table, table_exists};

#[test]
fn legacy_task_fts_rows_backfill_into_corpus_fts_once() {
    let conn = Connection::open_in_memory().expect("open db");
    conn.execute_batch(&format!(
        r#"
                    CREATE VIRTUAL TABLE {} USING fts5(
                        source_id UNINDEXED,
                        field UNINDEXED,
                        content,
                        tokenize = 'porter unicode61 remove_diacritics 2'
                    );
                    INSERT INTO {}(source_id, field, content)
                    VALUES ('T1', 'title', 'alpha'), ('T2', 'plan', 'beta');
                "#,
        legacy_task_fts_table(),
        legacy_task_fts_table()
    ))
    .expect("create legacy table");

    ensure_vector_schema(&conn).expect("migrate schema");
    ensure_vector_schema(&conn).expect("migrate schema idempotently");

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM corpus_fts", [], |row| row.get(0))
        .expect("count corpus rows");
    let task_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM corpus_fts WHERE source_kind = 'task'",
            [],
            |row| row.get(0),
        )
        .expect("count task rows");
    assert_eq!(rows, 2);
    assert_eq!(task_rows, 2);
    assert!(!table_exists(&conn, legacy_task_fts_table()).expect("legacy lookup"));
}
