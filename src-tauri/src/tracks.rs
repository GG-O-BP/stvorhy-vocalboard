//! 레퍼런스 트랙 커맨드 (§8): 임포트 → 분리(이중 모드, 곡당 1회 캐시)
//! → SwiftF0 추출 → 노트 → tracks 저장.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use vocalboard_codec::preview::compute_preview;
use vocalboard_reference::decode::decode_file;
use vocalboard_reference::download::ensure_model;
use vocalboard_reference::extract::{
    bridge_gaps, extract_frames, segment_notes, stem_to_16k, ExtractParams, SegmentParams,
};
use vocalboard_reference::flac::write_flac_stereo;
use vocalboard_reference::separate::{
    MdxSeparator, OrtSpectroModel, OrtWaveModel, SepMode, SeparationEngine, StereoPcm,
    StubSeparationEngine, WaveformSeparator, SEP_SR, STUB_SEP_ID,
};
use vocalboard_storage::thread::{UpdateTrackPitch, UpsertTrack};
use vocalboard_storage::{db, StorageRoot};

use crate::AppState;

/// 분리 잡 진행 이벤트.
#[derive(Debug, Clone, Serialize)]
pub struct SepProgress {
    /// download | decode | separate | extract | save | done | error
    pub stage: String,
    /// [0,1]
    pub progress: f32,
    pub message: Option<String>,
}

/// 실행 중 분리 잡 (트랙당 1개).
#[derive(Default)]
pub struct TrackJobs {
    running: Mutex<std::collections::HashSet<String>>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Android content:// 를 포함한 소스에서 앱 디렉토리로 복사한다.
fn copy_source(app: &AppHandle, src: &str, dest: &Path) -> Result<(), String> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_fs::FsExt;
        let path: tauri_plugin_fs::FilePath = src.parse().map_err(|e| format!("{e}"))?;
        let mut reader = app
            .fs()
            .open(path, tauri_plugin_fs::OpenOptions::new().read(true).clone())
            .map_err(|e| e.to_string())?;
        let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
        std::io::copy(&mut reader, &mut out).map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        std::fs::copy(src, dest).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportedTrack {
    pub id: String,
    pub title: String,
    pub duration_ms: u32,
}

#[tauri::command]
pub fn import_track(
    app: AppHandle,
    state: State<'_, AppState>,
    src_path: String,
) -> Result<ImportedTrack, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let title = Path::new(&src_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "제목 없음".into());
    let ext = Path::new(&src_path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "bin".into());
    let dest = state.root.track_dir(&id).join(format!("source.{ext}"));
    copy_source(&app, &src_path, &dest)?;

    // 임포트 시점 디코드로 포맷 검증 + duration 확정 (HE-AAC/DRM 조기 안내).
    let decoded = decode_file(&dest).map_err(|e| {
        let _ = std::fs::remove_dir_all(state.root.track_dir(&id));
        e.to_string()
    })?;
    let duration_ms = decoded.duration_ms();

    let storage = state.storage.lock().unwrap();
    let storage = storage.as_ref().ok_or("스토리지 미초기화")?;
    storage
        .upsert_track(UpsertTrack {
            id: id.clone(),
            title: title.clone(),
            source_path: vocalboard_storage::to_rel_path(&state.root, &dest),
            duration_ms,
            created_at_ms: now_ms(),
        })
        .map_err(|e| e.to_string())?;
    Ok(ImportedTrack { id, title, duration_ms })
}

#[tauri::command]
pub fn separate_track(
    app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
    channel: Channel<SepProgress>,
) -> Result<(), String> {
    let jobs = app.state::<TrackJobs>();
    {
        let mut running = jobs.running.lock().unwrap();
        if !running.insert(track_id.clone()) {
            return Err("이미 분리 작업이 진행 중입니다".into());
        }
    }

    let quality = state.config.lock().unwrap().separation_quality;
    let root = state.root.clone();
    let storage_tx = {
        let guard = state.storage.lock().unwrap();
        guard.as_ref().ok_or("스토리지 미초기화")?.sender()
    };
    let app2 = app.clone();
    let track_id2 = track_id.clone();

    std::thread::Builder::new()
        .name("vocalboard-separate".into())
        .spawn(move || {
            let emit = |stage: &str, progress: f32, message: Option<String>| {
                let _ = channel.send(SepProgress {
                    stage: stage.into(),
                    progress,
                    message,
                });
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_separation_job(&app2, &root, &storage_tx, &track_id2, quality, &emit)
            }))
            .unwrap_or_else(|p| {
                let msg = p
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| p.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".into());
                Err(format!("내부 패닉: {msg}"))
            });
            match result {
                Ok(()) => {
                    eprintln!("[separate] 완료: {track_id2}");
                    emit("done", 1.0, None);
                }
                Err(e) => {
                    // 채널 수신자가 없어도(탭 이탈) 원인 추적이 가능해야 한다.
                    eprintln!("[separate] 실패 {track_id2}: {e}");
                    emit("error", 0.0, Some(e));
                }
            }
            let jobs = app2.state::<TrackJobs>();
            jobs.running.lock().unwrap().remove(&track_id2);
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn run_separation_job(
    app: &AppHandle,
    root: &StorageRoot,
    storage_tx: &std::sync::mpsc::Sender<vocalboard_storage::StorageMsg>,
    track_id: &str,
    quality: bool,
    emit: &dyn Fn(&str, f32, Option<String>),
) -> Result<(), String> {
    let conn = db::open_read(&root.db_path()).map_err(|e| e.to_string())?;

    let use_stub = std::env::var("VOCALBOARD_SEP").is_ok_and(|v| v == "stub");
    let mode = if quality { SepMode::Quality } else { SepMode::Fast };
    let sep_id = if use_stub { STUB_SEP_ID } else { mode.id() };

    // 곡당 1회 캐시: 같은 모델로 이미 분리됨 → 즉시 완료.
    let (separated, prev_model) =
        vocalboard_storage::queries::track_sep_state(&conn, track_id).map_err(|e| e.to_string())?;
    if separated && prev_model.as_deref() == Some(sep_id) {
        emit("done", 1.0, Some("캐시 사용 (이미 분리됨)".into()));
        return Ok(());
    }

    let source_rel = vocalboard_storage::queries::track_source_path(&conn, track_id)
        .map_err(|e| e.to_string())?
        .ok_or("트랙 소스가 없습니다")?;
    drop(conn);

    // 1) 디코드 + 44.1k 스테레오 정규화.
    emit("decode", 0.0, None);
    let decoded = decode_file(&root.app_data.join(&source_rel)).map_err(|e| e.to_string())?;
    let (mut left, mut right) = decoded.to_stereo();
    if decoded.sample_rate != SEP_SR {
        left = vocalboard_dsp::resample::resample_all(&left, decoded.sample_rate, SEP_SR)
            .map_err(|e| e.to_string())?;
        right = vocalboard_dsp::resample::resample_all(&right, decoded.sample_rate, SEP_SR)
            .map_err(|e| e.to_string())?;
    }
    emit("decode", 1.0, None);

    // 2) 분리 엔진 준비 (모델 온디맨드 다운로드).
    let mut engine: Box<dyn SeparationEngine> = if use_stub {
        Box::new(StubSeparationEngine)
    } else {
        let remote = mode.remote_model();
        let mut last = 0u64;
        let model_path = ensure_model(&root.models_dir(), remote, &mut |got, total| {
            // 과도한 이벤트 방지: 1MB 단위.
            if got - last > 1_000_000 || Some(got) == total {
                last = got;
                let p = total.map(|t| got as f32 / t as f32).unwrap_or(0.0);
                emit("download", p.min(1.0), None);
            }
        })
        .map_err(|e| e.to_string())?;
        match mode {
            SepMode::Fast => Box::new(MdxSeparator::new(
                OrtSpectroModel::from_file(&model_path).map_err(|e| e.to_string())?,
            )),
            SepMode::Quality => Box::new(WaveformSeparator::new(
                OrtWaveModel::from_file(&model_path).map_err(|e| e.to_string())?,
            )),
        }
    };

    // 3) 분리.
    let mix = StereoPcm { left, right };
    let stems = engine
        .separate(&mix, &mut |p| emit("separate", p, None))
        .map_err(|e| e.to_string())?;

    // 4) 스템 FLAC 저장 (§5: tracks/{id}/, FLAC).
    emit("save", 0.2, None);
    let dir = root.track_dir(track_id);
    write_flac_stereo(&dir.join("vocals.flac"), &stems.vocals.left, &stems.vocals.right, SEP_SR)
        .map_err(|e| e.to_string())?;
    write_flac_stereo(
        &dir.join("accompaniment.flac"),
        &stems.accompaniment.left,
        &stems.accompaniment.right,
        SEP_SR,
    )
    .map_err(|e| e.to_string())?;
    emit("save", 1.0, None);

    // 5) SwiftF0 추출 + 후처리 + 노트 (F0C1 재사용).
    // resolve 경로 우선, 없으면 임베드 모델(lib.rs::SWIFTF0_MODEL) 사용.
    let mut pitch_engine = match crate::resolve_model_path(app) {
        Some(p) => vocalboard_dsp::OrtEngine::from_file(&p),
        None => vocalboard_dsp::OrtEngine::from_memory(crate::SWIFTF0_MODEL),
    }
    .map_err(|e| e.to_string())?;
    let stem16k = stem_to_16k(&stems.vocals.left, &stems.vocals.right, SEP_SR)
        .map_err(|e| e.to_string())?;
    let params = ExtractParams::default();
    let mut frames = extract_frames(&mut pitch_engine, &stem16k, &params, &mut |p| {
        emit("extract", p, None)
    })
    .map_err(|e| e.to_string())?;
    bridge_gaps(&mut frames, &params);
    let notes = segment_notes(&frames, &SegmentParams::default());

    let preview = compute_preview(&frames);
    let blob = vocalboard_codec::encode_compressed(0, &frames).map_err(|e| e.to_string())?;
    let notes_json = serde_json::to_string(&notes).map_err(|e| e.to_string())?;

    // 6) 스토리지 스레드로 커밋.
    let (reply, rx) = std::sync::mpsc::sync_channel(1);
    storage_tx
        .send(vocalboard_storage::StorageMsg::UpdateTrackPitch {
            update: Box::new(UpdateTrackPitch {
                id: track_id.to_string(),
                sep_model: sep_id.to_string(),
                pitch_blob: blob,
                pitch_codec: vocalboard_codec::CODEC_NAME.to_string(),
                notes_json,
                preview,
            }),
            reply,
        })
        .map_err(|_| "storage thread down".to_string())?;
    rx.recv().map_err(|_| "storage reply lost".to_string())??;
    Ok(())
}

#[tauri::command]
pub fn list_tracks(
    state: State<'_, AppState>,
) -> Result<Vec<vocalboard_storage::queries::TrackListItem>, String> {
    let conn = db::open_read(&state.root.db_path()).map_err(|e| e.to_string())?;
    vocalboard_storage::queries::list_tracks(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn track_series(
    state: State<'_, AppState>,
    id: String,
    max_points: u32,
) -> Result<vocalboard_storage::queries::SessionSeries, String> {
    let conn = db::open_read(&state.root.db_path()).map_err(|e| e.to_string())?;
    vocalboard_storage::queries::track_series(&conn, &id, max_points).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_track(state: State<'_, AppState>, id: String) -> Result<(), String> {
    {
        let storage = state.storage.lock().unwrap();
        let storage = storage.as_ref().ok_or("스토리지 미초기화")?;
        storage.delete_track(id.clone()).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(state.root.track_dir(&id));
    Ok(())
}

/// 재생 소스 경로 (연습/노래방): 반주 스템.
pub fn accompaniment_path(root: &StorageRoot, track_id: &str) -> PathBuf {
    root.track_dir(track_id).join("accompaniment.flac")
}

/// 분리 잡 활성 여부 (연습 시작 가드).
pub fn job_running(app: &AppHandle, track_id: &str) -> bool {
    app.state::<TrackJobs>()
        .running
        .lock()
        .unwrap()
        .contains(track_id)
}

/// TrackJobs를 state로 등록 (run()에서 호출).
pub fn init(app: &AppHandle) {
    app.manage(TrackJobs::default());
}
