//! 조회 (§5): list_sessions / session_detail / session_series.
//! 읽기 전용 연결로 커맨드 스레드에서 직접 실행한다.

use rusqlite::Connection;
use serde::Serialize;
use vocalboard_codec::decode_compressed;

use crate::StorageError;

#[derive(Debug, Clone, Serialize)]
pub struct SessionListItem {
    pub id: String,
    pub started_at: i64,
    pub duration_ms: u32,
    pub track_id: Option<String>,
    pub track_title: Option<String>,
    pub frame_count: u32,
    pub voiced_ratio: f32,
    pub mean_score: Option<f32>,
    pub has_recording: bool,
    /// preview BLOB (§5: [버전][u8×256]) — 프론트 썸네일 렌더용.
    pub preview: Option<Vec<u8>>,
}

pub fn list_sessions(conn: &Connection) -> Result<Vec<SessionListItem>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.started_at, s.duration_ms, s.track_id, t.title,
                s.frame_count, s.voiced_ratio, s.mean_score, s.recording_path, s.preview
         FROM sessions s LEFT JOIN tracks t ON t.id = s.track_id
         ORDER BY s.started_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SessionListItem {
                id: r.get(0)?,
                started_at: r.get(1)?,
                duration_ms: r.get::<_, Option<u32>>(2)?.unwrap_or(0),
                track_id: r.get(3)?,
                track_title: r.get(4)?,
                frame_count: r.get::<_, Option<u32>>(5)?.unwrap_or(0),
                voiced_ratio: r.get::<_, Option<f32>>(6)?.unwrap_or(0.0),
                mean_score: r.get(7)?,
                has_recording: r.get::<_, Option<String>>(8)?.is_some(),
                preview: r.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackListItem {
    pub id: String,
    pub title: Option<String>,
    pub duration_ms: u32,
    pub separated: bool,
    pub sep_model: Option<String>,
    pub notes_json: Option<String>,
    pub created_at: i64,
    pub preview: Option<Vec<u8>>,
}

pub fn list_tracks(conn: &Connection) -> Result<Vec<TrackListItem>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, title, duration_ms, separated, sep_model, notes_json, created_at, preview
         FROM tracks ORDER BY created_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TrackListItem {
                id: r.get(0)?,
                title: r.get(1)?,
                duration_ms: r.get::<_, Option<u32>>(2)?.unwrap_or(0),
                separated: r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                sep_model: r.get(4)?,
                notes_json: r.get(5)?,
                created_at: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                preview: r.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 트랙 소스 파일 상대 경로 (재생/재분리용).
pub fn track_source_path(conn: &Connection, id: &str) -> Result<Option<String>, StorageError> {
    Ok(conn.query_row("SELECT source_path FROM tracks WHERE id = ?1", [id], |r| r.get(0))?)
}

/// 트랙 분리 캐시 상태: (separated, sep_model).
pub fn track_sep_state(
    conn: &Connection,
    id: &str,
) -> Result<(bool, Option<String>), StorageError> {
    Ok(conn.query_row(
        "SELECT separated, sep_model FROM tracks WHERE id = ?1",
        [id],
        |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?.unwrap_or(0) != 0,
                r.get(1)?,
            ))
        },
    )?)
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDetail {
    pub id: String,
    pub started_at: i64,
    pub duration_ms: u32,
    pub track_id: Option<String>,
    pub track_title: Option<String>,
    pub frame_count: u32,
    pub voiced_ratio: f32,
    pub midi_min: Option<f32>,
    pub midi_max: Option<f32>,
    pub mean_abs_cents: Option<f32>,
    pub mean_score: Option<f32>,
    pub octave_invariant: bool,
    pub codec: Option<String>,
    pub recording_path: Option<String>,
}

pub fn session_detail(conn: &Connection, id: &str) -> Result<SessionDetail, StorageError> {
    let row = conn.query_row(
        "SELECT s.id, s.started_at, s.duration_ms, s.track_id, t.title, s.frame_count,
                s.voiced_ratio, s.midi_min, s.midi_max, s.mean_abs_cents, s.mean_score,
                s.octave_invariant, s.codec, s.recording_path
         FROM sessions s LEFT JOIN tracks t ON t.id = s.track_id
         WHERE s.id = ?1",
        [id],
        |r| {
            Ok(SessionDetail {
                id: r.get(0)?,
                started_at: r.get(1)?,
                duration_ms: r.get::<_, Option<u32>>(2)?.unwrap_or(0),
                track_id: r.get(3)?,
                track_title: r.get(4)?,
                frame_count: r.get::<_, Option<u32>>(5)?.unwrap_or(0),
                voiced_ratio: r.get::<_, Option<f32>>(6)?.unwrap_or(0.0),
                midi_min: r.get(7)?,
                midi_max: r.get(8)?,
                mean_abs_cents: r.get(9)?,
                mean_score: r.get(10)?,
                octave_invariant: r.get::<_, Option<i64>>(11)?.unwrap_or(0) != 0,
                codec: r.get(12)?,
                recording_path: r.get(13)?,
            })
        },
    )?;
    Ok(row)
}

/// min/max 데시메이션된 피치 시리즈 포인트.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct SeriesPoint {
    /// 버킷 시작 시각 (ms).
    pub t: u32,
    /// 버킷 내 voiced midi 최소/최대. 무성 버킷은 null.
    pub min: Option<f32>,
    pub max: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSeries {
    pub frame_count: u32,
    pub hop_ms: u8,
    pub duration_ms: u32,
    pub points: Vec<SeriesPoint>,
}

/// 세션 프레임을 최대 `max_points` 버킷으로 데시메이션한다.
/// 프레임 수가 그보다 적으면 프레임당 1포인트.
pub fn session_series(
    conn: &Connection,
    id: &str,
    max_points: u32,
) -> Result<SessionSeries, StorageError> {
    let blob: Vec<u8> = conn.query_row(
        "SELECT data FROM session_frames WHERE session_id = ?1",
        [id],
        |r| r.get(0),
    )?;
    decimate_blob(&blob, max_points)
}

/// 트랙 레퍼런스 피치의 데시메이션 시리즈 (연습 오버레이/미리보기용).
pub fn track_series(
    conn: &Connection,
    id: &str,
    max_points: u32,
) -> Result<SessionSeries, StorageError> {
    let blob: Option<Vec<u8>> =
        conn.query_row("SELECT pitch FROM tracks WHERE id = ?1", [id], |r| r.get(0))?;
    let blob = blob.ok_or_else(|| {
        StorageError::Other("트랙에 추출된 피치가 없습니다 (분리를 먼저 실행)".into())
    })?;
    decimate_blob(&blob, max_points)
}

fn decimate_blob(blob: &[u8], max_points: u32) -> Result<SessionSeries, StorageError> {
    let (header, frames) = decode_compressed(blob)?;
    let n = frames.len();
    let max_points = max_points.max(1) as usize;
    let hop_ms = header.hop_ms as u32;

    let mut points = Vec::with_capacity(n.min(max_points));
    if n <= max_points {
        for (i, f) in frames.iter().enumerate() {
            let (min, max) = if f.voiced() {
                (Some(f.midi()), Some(f.midi()))
            } else {
                (None, None)
            };
            points.push(SeriesPoint { t: (i as u32) * hop_ms, min, max });
        }
    } else {
        for b in 0..max_points {
            let lo = b * n / max_points;
            let hi = ((b + 1) * n / max_points).max(lo + 1);
            let mut min = f32::INFINITY;
            let mut max = f32::NEG_INFINITY;
            let mut any = false;
            for f in &frames[lo..hi] {
                if f.voiced() {
                    any = true;
                    let m = f.midi();
                    min = min.min(m);
                    max = max.max(m);
                }
            }
            points.push(SeriesPoint {
                t: (lo as u32) * hop_ms,
                min: any.then_some(min),
                max: any.then_some(max),
            });
        }
    }
    Ok(SessionSeries {
        frame_count: n as u32,
        hop_ms: header.hop_ms,
        duration_ms: (n as u32) * hop_ms,
        points,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread::{spawn, BeginSession, StorageMsg};
    use crate::StorageRoot;
    use vocalboard_codec::F0Frame;

    fn seed_session(root: &StorageRoot, id: &str, frames: Vec<F0Frame>) {
        let (handle, _) = spawn(root.clone()).unwrap();
        handle
            .begin(BeginSession {
                id: id.into(),
                started_at_ms: 42,
                track_id: None,
                octave_invariant: false,
                recording: None,
                recording_keep_last: 10,
                latency_calib_ms: 0,
            })
            .unwrap();
        let tx = handle.sender();
        for f in frames {
            tx.send(StorageMsg::Frame(f)).unwrap();
        }
        handle.end(false).unwrap().unwrap();
        handle.shutdown();
    }

    #[test]
    fn list_and_detail() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::new(dir.path());
        seed_session(
            &root,
            "q1",
            (0..100).map(|_| F0Frame::quantize(64.0, true, 0.99, -22.0)).collect(),
        );
        let conn = crate::db::open_read(&root.db_path()).unwrap();
        let list = list_sessions(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "q1");
        assert_eq!(list[0].frame_count, 100);
        assert_eq!(list[0].preview.as_ref().map(|p| p.len()), Some(257));
        let detail = session_detail(&conn, "q1").unwrap();
        assert_eq!(detail.midi_min, Some(64.0));
        assert!(!detail.octave_invariant);
    }

    #[test]
    fn series_decimation_minmax() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::new(dir.path());
        // 10_000 프레임: 버킷마다 [60, 60+Δ] 범위가 나오도록 톱니 패턴,
        // 매 100프레임 블록의 앞 10프레임은 unvoiced.
        let frames: Vec<F0Frame> = (0..10_000)
            .map(|i| {
                if i % 100 < 10 {
                    F0Frame::quantize(0.0, false, 0.0, -80.0)
                } else {
                    F0Frame::quantize(60.0 + (i % 50) as f32 * 0.2, true, 0.95, -20.0)
                }
            })
            .collect();
        seed_session(&root, "big", frames);
        let conn = crate::db::open_read(&root.db_path()).unwrap();

        let s = session_series(&conn, "big", 200).unwrap();
        assert_eq!(s.frame_count, 10_000);
        assert_eq!(s.points.len(), 200);
        assert_eq!(s.hop_ms, 16);
        for p in &s.points {
            let (min, max) = (p.min.unwrap(), p.max.unwrap());
            assert!(min <= max);
            assert!((60.0..=69.8).contains(&min));
            assert!(max <= 69.8 + 1e-3);
        }
        // 버킷(50프레임) 안에 톱니 반주기가 들어가므로 min<max.
        assert!(s.points.iter().filter(|p| p.min < p.max).count() > 150);

        // 프레임 수보다 큰 max_points → 프레임당 1포인트, 무성 null.
        let small = session_series(&conn, "big", 20_000).unwrap();
        assert_eq!(small.points.len(), 10_000);
        assert!(small.points[0].min.is_none()); // 첫 프레임은 unvoiced
        assert_eq!(small.points[1].t, 16);
    }

    #[test]
    fn missing_session_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::new(dir.path());
        root.ensure_dirs().unwrap();
        drop(crate::db::open_write(&root.db_path()).unwrap());
        let conn = crate::db::open_read(&root.db_path()).unwrap();
        assert!(session_detail(&conn, "nope").is_err());
        assert!(session_series(&conn, "nope", 100).is_err());
    }
}
