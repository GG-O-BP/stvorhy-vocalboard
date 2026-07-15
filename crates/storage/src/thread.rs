//! 스토리지 스레드 (§2: DSP 스레드 → mpsc → 스토리지).
//!
//! 쓰기 연결 단독 소유. 세션 수명주기:
//! Begin → (Frame|Audio)* → End{discard} → 요약 통계·preview 계산 후
//! F0C1+zstd BLOB 트랜잭션 커밋 (+녹음 finalize, 보존 정책 적용).
//! 시작 시 [`startup_recovery`]로 고아 저널·깨진 WAV를 복구한다.

use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::thread::JoinHandle;

use rusqlite::{params, Connection};
use serde::Serialize;
use vocalboard_codec::{encode_compressed, preview::compute_preview, F0Frame, CODEC_NAME};

use crate::journal::{self, JournalWriter};
use crate::recording::{self, WavRecorder};
use crate::stats::SessionStats;
use crate::{to_rel_path, StorageError, StorageRoot};

#[derive(Debug, Clone, Copy)]
pub struct RecordingSpec {
    pub sample_rate: u32,
}

#[derive(Debug, Clone)]
pub struct BeginSession {
    pub id: String,
    pub started_at_ms: i64,
    pub track_id: Option<String>,
    pub octave_invariant: bool,
    pub recording: Option<RecordingSpec>,
    /// 보존 정책 (최근 N개). 세션 종료 시 적용.
    pub recording_keep_last: u32,
    /// 채점 정렬용 지연 캘리브레이션 (ms, 사용자 프레임 보정 §5).
    pub latency_calib_ms: i32,
}

/// 세션 종료 요약 (프론트 회신용).
#[derive(Debug, Clone, Serialize)]
pub struct FinalizedSession {
    pub id: String,
    pub started_at: i64,
    pub duration_ms: u32,
    pub frame_count: u32,
    pub voiced_ratio: f32,
    pub midi_min: Option<f32>,
    pub midi_max: Option<f32>,
    pub mean_abs_cents: Option<f32>,
    pub mean_score: Option<f32>,
    pub recording_path: Option<String>,
}

/// 트랙 행 삽입/갱신 (임포트 시).
#[derive(Debug, Clone)]
pub struct UpsertTrack {
    pub id: String,
    pub title: String,
    pub source_path: String,
    pub duration_ms: u32,
    pub created_at_ms: i64,
}

/// 분리+추출 완료 결과 반영.
#[derive(Debug, Clone)]
pub struct UpdateTrackPitch {
    pub id: String,
    pub sep_model: String,
    pub pitch_blob: Vec<u8>,
    pub pitch_codec: String,
    pub notes_json: String,
    pub preview: Vec<u8>,
}

pub enum StorageMsg {
    Begin(BeginSession),
    Frame(F0Frame),
    Audio(Vec<f32>),
    End {
        discard: bool,
        reply: SyncSender<Result<Option<FinalizedSession>, String>>,
    },
    UpsertTrack {
        track: UpsertTrack,
        reply: SyncSender<Result<(), String>>,
    },
    UpdateTrackPitch {
        update: Box<UpdateTrackPitch>,
        reply: SyncSender<Result<(), String>>,
    },
    DeleteTrack {
        id: String,
        reply: SyncSender<Result<(), String>>,
    },
    Shutdown,
}

struct ActiveSession {
    meta: BeginSession,
    journal: JournalWriter,
    frames: Vec<F0Frame>,
    recorder: Option<WavRecorder>,
}

pub struct StorageHandle {
    tx: Sender<StorageMsg>,
    join: Option<JoinHandle<()>>,
}

impl StorageHandle {
    pub fn sender(&self) -> Sender<StorageMsg> {
        self.tx.clone()
    }

    pub fn begin(&self, b: BeginSession) -> Result<(), StorageError> {
        self.tx
            .send(StorageMsg::Begin(b))
            .map_err(|_| StorageError::Other("storage thread down".into()))
    }

    /// 동기 종료: 커밋 완료 요약을 기다린다.
    pub fn end(&self, discard: bool) -> Result<Option<FinalizedSession>, StorageError> {
        let (reply, rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(StorageMsg::End { discard, reply })
            .map_err(|_| StorageError::Other("storage thread down".into()))?;
        rx.recv()
            .map_err(|_| StorageError::Other("storage thread dropped reply".into()))?
            .map_err(StorageError::Other)
    }

    pub fn upsert_track(&self, track: UpsertTrack) -> Result<(), StorageError> {
        let (reply, rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(StorageMsg::UpsertTrack { track, reply })
            .map_err(|_| StorageError::Other("storage thread down".into()))?;
        rx.recv()
            .map_err(|_| StorageError::Other("storage thread dropped reply".into()))?
            .map_err(StorageError::Other)
    }

    pub fn update_track_pitch(&self, update: UpdateTrackPitch) -> Result<(), StorageError> {
        let (reply, rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(StorageMsg::UpdateTrackPitch { update: Box::new(update), reply })
            .map_err(|_| StorageError::Other("storage thread down".into()))?;
        rx.recv()
            .map_err(|_| StorageError::Other("storage thread dropped reply".into()))?
            .map_err(StorageError::Other)
    }

    pub fn delete_track(&self, id: String) -> Result<(), StorageError> {
        let (reply, rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(StorageMsg::DeleteTrack { id, reply })
            .map_err(|_| StorageError::Other("storage thread down".into()))?;
        rx.recv()
            .map_err(|_| StorageError::Other("storage thread dropped reply".into()))?
            .map_err(StorageError::Other)
    }

    pub fn shutdown(mut self) {
        let _ = self.tx.send(StorageMsg::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for StorageHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(StorageMsg::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// 시작 시 복구: 깨진 WAV 헤더 → 고아 저널 → DB 세션 승격.
/// 반환: 복구된 세션 id 목록.
pub fn startup_recovery(conn: &Connection, root: &StorageRoot) -> Result<Vec<String>, StorageError> {
    let repaired = recording::repair_all(&root.recordings_dir())?;
    for p in &repaired {
        eprintln!("[storage] WAV 헤더 복구: {}", p.display());
    }
    let mut recovered = Vec::new();
    for path in journal::list_orphans(&root.db_dir())? {
        let id = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match journal::read_journal(&path) {
            Ok((started_at, frames)) if !frames.is_empty() => {
                let wav = root.recording_path(&id);
                let recording_rel = wav.exists().then(|| to_rel_path(root, &wav));
                insert_session(
                    conn,
                    &id,
                    started_at,
                    None,
                    false,
                    &frames,
                    recording_rel.as_deref(),
                    None,
                    None,
                )?;
                std::fs::remove_file(&path)?;
                eprintln!("[storage] 고아 세션 복구: {id} ({} frames)", frames.len());
                recovered.push(id);
            }
            Ok(_) => {
                // 빈 저널: 흔적만 지운다.
                std::fs::remove_file(&path)?;
            }
            Err(e) => {
                // 데이터 보존을 위해 삭제 대신 격리.
                eprintln!("[storage] 저널 해석 실패 {}: {e}", path.display());
                let _ = std::fs::rename(&path, path.with_extension("f0raw.corrupt"));
            }
        }
    }
    Ok(recovered)
}

#[allow(clippy::too_many_arguments)]
fn insert_session(
    conn: &Connection,
    id: &str,
    started_at: i64,
    track_id: Option<&str>,
    octave_invariant: bool,
    frames: &[F0Frame],
    recording_path: Option<&str>,
    mean_abs_cents: Option<f32>,
    mean_score: Option<f32>,
) -> Result<FinalizedSession, StorageError> {
    let stats = SessionStats::from_frames(frames);
    let preview = compute_preview(frames);
    let blob = encode_compressed(started_at, frames)?;

    conn.execute("BEGIN IMMEDIATE", [])?;
    let result = (|| -> Result<(), StorageError> {
        conn.execute(
            "INSERT INTO sessions (id, started_at, duration_ms, track_id, frame_count,
                voiced_ratio, midi_min, midi_max, mean_abs_cents, mean_score,
                octave_invariant, preview, codec, recording_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                id,
                started_at,
                stats.duration_ms,
                track_id,
                stats.frame_count,
                stats.voiced_ratio,
                stats.midi_min,
                stats.midi_max,
                mean_abs_cents,
                mean_score,
                octave_invariant as i64,
                preview,
                CODEC_NAME,
                recording_path,
            ],
        )?;
        conn.execute(
            "INSERT INTO session_frames (session_id, data) VALUES (?1, ?2)",
            params![id, blob],
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute("COMMIT", [])?;
            Ok(FinalizedSession {
                id: id.to_string(),
                started_at,
                duration_ms: stats.duration_ms,
                frame_count: stats.frame_count,
                voiced_ratio: stats.voiced_ratio,
                midi_min: stats.midi_min,
                midi_max: stats.midi_max,
                mean_abs_cents,
                mean_score,
                recording_path: recording_path.map(String::from),
            })
        }
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            Err(e)
        }
    }
}

/// 스토리지 스레드를 띄운다. 반환 전에 시작 복구를 완료한다.
pub fn spawn(root: StorageRoot) -> Result<(StorageHandle, Vec<String>), StorageError> {
    root.ensure_dirs()?;
    let conn = crate::db::open_write(&root.db_path())?;
    let recovered = startup_recovery(&conn, &root)?;

    let (tx, rx) = std::sync::mpsc::channel::<StorageMsg>();
    let join = std::thread::Builder::new()
        .name("vocalboard-storage".into())
        .spawn(move || run(conn, root, rx))
        .map_err(StorageError::Io)?;
    Ok((
        StorageHandle {
            tx,
            join: Some(join),
        },
        recovered,
    ))
}

fn run(conn: Connection, root: StorageRoot, rx: Receiver<StorageMsg>) {
    let mut active: Option<ActiveSession> = None;

    loop {
        let msg = match rx.recv() {
            Ok(m) => m,
            Err(_) => StorageMsg::Shutdown,
        };
        match msg {
            StorageMsg::Begin(meta) => {
                if active.is_some() {
                    eprintln!("[storage] Begin 무시: 이미 세션 진행 중");
                    continue;
                }
                match open_session(&root, meta) {
                    Ok(s) => active = Some(s),
                    Err(e) => eprintln!("[storage] 세션 시작 실패: {e}"),
                }
            }
            StorageMsg::Frame(f) => {
                if let Some(s) = active.as_mut() {
                    s.frames.push(f);
                    if let Err(e) = s.journal.append(&f) {
                        eprintln!("[storage] 저널 기록 실패: {e}");
                    }
                }
            }
            StorageMsg::Audio(chunk) => {
                if let Some(rec) = active.as_mut().and_then(|s| s.recorder.as_mut()) {
                    if let Err(e) = rec.write(&chunk) {
                        eprintln!("[storage] 녹음 기록 실패: {e}");
                    }
                }
            }
            StorageMsg::End { discard, reply } => {
                let outcome = match active.take() {
                    None => Ok(None),
                    Some(s) => finalize(&conn, &root, s, discard)
                        .map(Some)
                        .map_err(|e| e.to_string()),
                };
                let _ = reply.send(outcome.map(|o| o.flatten()));
            }
            StorageMsg::UpsertTrack { track, reply } => {
                let r = conn
                    .execute(
                        "INSERT INTO tracks (id, title, source_path, duration_ms, separated, created_at)
                         VALUES (?1, ?2, ?3, ?4, 0, ?5)
                         ON CONFLICT(id) DO UPDATE SET title=?2, source_path=?3, duration_ms=?4",
                        params![
                            track.id,
                            track.title,
                            track.source_path,
                            track.duration_ms,
                            track.created_at_ms
                        ],
                    )
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let _ = reply.send(r);
            }
            StorageMsg::UpdateTrackPitch { update, reply } => {
                let r = conn
                    .execute(
                        "UPDATE tracks SET separated=1, sep_model=?2, pitch_codec=?3,
                                pitch=?4, notes_json=?5, preview=?6 WHERE id=?1",
                        params![
                            update.id,
                            update.sep_model,
                            update.pitch_codec,
                            update.pitch_blob,
                            update.notes_json,
                            update.preview
                        ],
                    )
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let _ = reply.send(r);
            }
            StorageMsg::DeleteTrack { id, reply } => {
                let r = conn
                    .execute("DELETE FROM tracks WHERE id=?1", params![id])
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let _ = reply.send(r);
            }
            StorageMsg::Shutdown => {
                if let Some(s) = active.take() {
                    // 앱 종료 중에도 데이터는 살린다.
                    if let Err(e) = finalize(&conn, &root, s, false) {
                        eprintln!("[storage] 종료 중 세션 커밋 실패: {e}");
                    }
                }
                break;
            }
        }
    }
}

/// 트랙의 F0C1 프레임 로드 (채점용).
fn load_track_frames(
    conn: &Connection,
    track_id: &str,
) -> Result<Option<Vec<F0Frame>>, StorageError> {
    let blob: Option<Vec<u8>> = conn
        .query_row("SELECT pitch FROM tracks WHERE id = ?1", [track_id], |r| r.get(0))
        .map_err(|e| StorageError::Other(e.to_string()))?;
    match blob {
        Some(b) => {
            let (_, frames) = vocalboard_codec::decode_compressed(&b)?;
            Ok(Some(frames))
        }
        None => Ok(None),
    }
}

fn open_session(root: &StorageRoot, meta: BeginSession) -> Result<ActiveSession, StorageError> {
    let journal = JournalWriter::create(root.journal_path(&meta.id), meta.started_at_ms)?;
    let recorder = match meta.recording {
        Some(spec) => Some(WavRecorder::create(
            root.recording_path(&meta.id),
            spec.sample_rate,
        )?),
        None => None,
    };
    Ok(ActiveSession {
        meta,
        journal,
        frames: Vec::with_capacity(4096),
        recorder,
    })
}

fn finalize(
    conn: &Connection,
    root: &StorageRoot,
    s: ActiveSession,
    discard: bool,
) -> Result<Option<FinalizedSession>, StorageError> {
    let ActiveSession {
        meta,
        journal,
        frames,
        recorder,
    } = s;

    if discard || frames.is_empty() {
        journal.remove()?;
        if let Some(r) = recorder {
            r.discard()?;
        }
        return Ok(None);
    }

    let recording_rel = match recorder {
        Some(r) => {
            let path = r.finalize()?;
            Some(to_rel_path(root, &path))
        }
        None => None,
    };

    // 채점 (§5): 트랙 참조 세션이면 레퍼런스 프레임과 정렬해 계산.
    let (mean_abs_cents, mean_score) = match meta.track_id.as_deref() {
        Some(track_id) => match load_track_frames(conn, track_id) {
            Ok(Some(reference)) => {
                let offset = (meta.latency_calib_ms as f32 / vocalboard_codec::HOP_MS as f32)
                    .round() as i32;
                let s = vocalboard_codec::scoring::score_session(
                    &frames,
                    &reference,
                    meta.octave_invariant,
                    offset,
                );
                (s.mean_abs_cents, s.mean_score)
            }
            Ok(None) => (None, None),
            Err(e) => {
                eprintln!("[storage] 채점용 트랙 로드 실패 {track_id}: {e}");
                (None, None)
            }
        },
        None => (None, None),
    };

    let row = insert_session(
        conn,
        &meta.id,
        meta.started_at_ms,
        meta.track_id.as_deref(),
        meta.octave_invariant,
        &frames,
        recording_rel.as_deref(),
        mean_abs_cents,
        mean_score,
    )?;
    journal.remove()?;

    if let Err(e) = recording::enforce_retention(conn, &root.app_data, meta.recording_keep_last) {
        eprintln!("[storage] 보존 정책 적용 실패: {e}");
    }
    Ok(Some(row))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vocalboard_codec::decode_compressed;

    fn voiced(midi: f32) -> F0Frame {
        F0Frame::quantize(midi, true, 0.95, -20.0)
    }

    fn unvoiced() -> F0Frame {
        F0Frame::quantize(0.0, false, 0.0, -80.0)
    }

    #[test]
    fn session_roundtrip_through_thread() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::new(dir.path());
        let (handle, recovered) = spawn(root.clone()).unwrap();
        assert!(recovered.is_empty());

        handle
            .begin(BeginSession {
                id: "sess-1".into(),
                started_at_ms: 1_752_000_000_000,
                track_id: None,
                octave_invariant: false,
                recording: Some(RecordingSpec { sample_rate: 48_000 }),
                recording_keep_last: 10,
                latency_calib_ms: 0,
            })
            .unwrap();
        let sender = handle.sender();
        for i in 0..625 {
            let f = if i % 5 == 0 { unvoiced() } else { voiced(57.0 + (i % 24) as f32 * 0.5) };
            sender.send(StorageMsg::Frame(f)).unwrap();
        }
        sender.send(StorageMsg::Audio(vec![0.25; 4800])).unwrap();
        let summary = handle.end(false).unwrap().expect("finalized");
        assert_eq!(summary.id, "sess-1");
        assert_eq!(summary.frame_count, 625);
        assert_eq!(summary.duration_ms, 10_000);
        assert!((summary.voiced_ratio - 0.8).abs() < 1e-6);
        assert!(summary.recording_path.is_some());

        // DB 검증: blob 라운드트립 + 저널 삭제 + 녹음 존재.
        let conn = crate::db::open_read(&root.db_path()).unwrap();
        let blob: Vec<u8> = conn
            .query_row("SELECT data FROM session_frames WHERE session_id='sess-1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let (header, frames) = decode_compressed(&blob).unwrap();
        assert_eq!(header.start_unix_ms, 1_752_000_000_000);
        assert_eq!(frames.len(), 625);
        assert_eq!(frames[1], voiced(57.5));
        let codec: String = conn
            .query_row("SELECT codec FROM sessions WHERE id='sess-1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(codec, CODEC_NAME);
        assert!(!root.journal_path("sess-1").exists());
        assert!(root.recording_path("sess-1").exists());
        let reader = hound::WavReader::open(root.recording_path("sess-1")).unwrap();
        assert_eq!(reader.len(), 4800);

        handle.shutdown();
    }

    #[test]
    fn discard_removes_everything() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::new(dir.path());
        let (handle, _) = spawn(root.clone()).unwrap();
        handle
            .begin(BeginSession {
                id: "gone".into(),
                started_at_ms: 5,
                track_id: None,
                octave_invariant: false,
                recording: Some(RecordingSpec { sample_rate: 16_000 }),
                recording_keep_last: 10,
                latency_calib_ms: 0,
            })
            .unwrap();
        handle.sender().send(StorageMsg::Frame(voiced(60.0))).unwrap();
        let out = handle.end(true).unwrap();
        assert!(out.is_none());
        let conn = crate::db::open_read(&root.db_path()).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        assert!(!root.journal_path("gone").exists());
        assert!(!root.recording_path("gone").exists());
        handle.shutdown();
    }

    #[test]
    fn orphan_journal_recovers_on_startup() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::new(dir.path());
        root.ensure_dirs().unwrap();
        // 크래시 세션 흉내: 저널 + 헤더 미확정 WAV.
        let mut jw = JournalWriter::create(root.journal_path("crashed"), 777).unwrap();
        for i in 0..50 {
            jw.append(&voiced(60.0 + i as f32 * 0.1)).unwrap();
        }
        jw.sync().unwrap();
        std::mem::forget(jw); // remove() 없이 크래시

        let mut rec = WavRecorder::create(root.recording_path("crashed"), 48_000).unwrap();
        rec.write(&vec![0.5; 480]).unwrap();
        rec.flush().unwrap(); // 주기 flush가 지나갔다고 가정
        std::mem::forget(rec); // finalize 없이 크래시

        let (handle, recovered) = spawn(root.clone()).unwrap();
        assert_eq!(recovered, vec!["crashed".to_string()]);
        assert!(!root.journal_path("crashed").exists());

        let conn = crate::db::open_read(&root.db_path()).unwrap();
        let (count, started, rec_path): (u32, i64, Option<String>) = conn
            .query_row(
                "SELECT frame_count, started_at, recording_path FROM sessions WHERE id='crashed'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(count, 50);
        assert_eq!(started, 777);
        // 복구 패스가 WAV 헤더도 고쳐 재생 가능해야 한다.
        let rec_path = rec_path.expect("recording linked");
        let reader = hound::WavReader::open(dir.path().join(rec_path)).unwrap();
        assert_eq!(reader.len(), 480);
        handle.shutdown();
    }

    #[test]
    fn empty_session_is_dropped_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::new(dir.path());
        let (handle, _) = spawn(root.clone()).unwrap();
        handle
            .begin(BeginSession {
                id: "empty".into(),
                started_at_ms: 1,
                track_id: None,
                octave_invariant: false,
                recording: None,
                recording_keep_last: 10,
                latency_calib_ms: 0,
            })
            .unwrap();
        let out = handle.end(false).unwrap();
        assert!(out.is_none());
        assert!(!root.journal_path("empty").exists());
        handle.shutdown();
    }
}
