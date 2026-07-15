//! 스키마(§5)·마이그레이션·연결 헬퍼.

use std::path::Path;

use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};

use crate::StorageError;

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(
        r#"
        CREATE TABLE tracks (
            id TEXT PRIMARY KEY,
            title TEXT,
            source_path TEXT,
            duration_ms INTEGER,
            separated INTEGER,
            sep_model TEXT,
            pitch_codec TEXT,
            pitch BLOB,
            notes_json TEXT,
            preview BLOB,
            created_at INTEGER
        );
        CREATE TABLE sessions (
            id TEXT PRIMARY KEY,
            started_at INTEGER,
            duration_ms INTEGER,
            track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL,
            frame_count INTEGER,
            voiced_ratio REAL,
            midi_min REAL,
            midi_max REAL,
            mean_abs_cents REAL,
            mean_score REAL,
            octave_invariant INTEGER,
            preview BLOB,
            codec TEXT,
            recording_path TEXT
        );
        CREATE TABLE session_frames (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            data BLOB
        );
        CREATE INDEX idx_sessions_started_at ON sessions(started_at DESC);
        "#,
    )])
}

/// 연결마다 적용해야 하는 PRAGMA (§5: WAL, synchronous=NORMAL,
/// foreign_keys=ON은 연결 단위 설정).
fn apply_pragmas(conn: &Connection) -> Result<(), StorageError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

/// 쓰기 연결 (스토리지 스레드 단독 소유). 마이그레이션 수행.
pub fn open_write(path: &Path) -> Result<Connection, StorageError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut conn = Connection::open(path)?;
    apply_pragmas(&conn)?;
    migrations().to_latest(&mut conn)?;
    Ok(conn)
}

/// 읽기 전용 연결 (조회 커맨드용, WAL 동시 읽기).
pub fn open_read(path: &Path) -> Result<Connection, StorageError> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // 읽기 연결에도 foreign_keys는 일관성 위해 켠다 (조인 의미엔 영향 없음).
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_validate() {
        assert!(migrations().validate().is_ok());
    }

    #[test]
    fn schema_applies_and_fk_cascades() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite3");
        let conn = open_write(&path).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, started_at, frame_count) VALUES ('s1', 0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_frames (session_id, data) VALUES ('s1', x'00')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM sessions WHERE id='s1'", []).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_frames", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "ON DELETE CASCADE");
    }

    #[test]
    fn track_delete_sets_session_track_null() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_write(&dir.path().join("t.sqlite3")).unwrap();
        conn.execute("INSERT INTO tracks (id, title) VALUES ('t1', 'x')", []).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, started_at, track_id) VALUES ('s1', 0, 't1')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM tracks WHERE id='t1'", []).unwrap();
        let track: Option<String> = conn
            .query_row("SELECT track_id FROM sessions WHERE id='s1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(track, None, "ON DELETE SET NULL");
    }
}
