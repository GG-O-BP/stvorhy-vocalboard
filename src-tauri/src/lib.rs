mod audio;
mod config;

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::Mutex;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;
use vocalboard_codec::F0Frame;
use vocalboard_dsp::{AcfEngine, InferenceEngine, OrtEngine, PitchFrame};
use vocalboard_storage::queries::{SessionDetail, SessionListItem, SessionSeries};
use vocalboard_storage::thread::BeginSession;
use vocalboard_storage::{db, FinalizedSession, RecordingSpec, StorageHandle, StorageMsg, StorageRoot};

use audio::{Capture, CaptureInfo, FrameSink};
use config::AppConfig;

struct AppState {
    capture: Mutex<Option<Capture>>,
    config: Mutex<AppConfig>,
    storage: Mutex<Option<StorageHandle>>,
    root: StorageRoot,
}

/// SwiftF0 모델 탐색: 환경변수 → app_data/models → (dev 빌드) 저장소 models/.
fn resolve_model_path(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("VOCALBOARD_SWIFTF0") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(dir) = app.path().app_data_dir() {
        let p = dir.join("models/swift_f0.onnx");
        if p.exists() {
            return Some(p);
        }
    }
    #[cfg(debug_assertions)]
    {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/swift_f0.onnx");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn make_engine(app: &AppHandle) -> Result<Box<dyn InferenceEngine>, String> {
    if std::env::var("VOCALBOARD_ENGINE").is_ok_and(|v| v == "acf") {
        return Ok(Box::new(AcfEngine::new()));
    }
    let path = resolve_model_path(app).ok_or_else(|| {
        "SwiftF0 모델(swift_f0.onnx)을 찾을 수 없습니다. README의 '모델 확보' 절차를 따르거나 \
         VOCALBOARD_SWIFTF0 환경변수를 설정하세요."
            .to_string()
    })?;
    Ok(Box::new(
        OrtEngine::from_file(&path).map_err(|e| format!("SwiftF0 로드 실패: {e}"))?,
    ))
}

/// Channel + 스토리지로 팬아웃하는 DSP 싱크.
struct CaptureSink {
    channel: Channel<PitchFrame>,
    storage: Sender<StorageMsg>,
    record_audio: bool,
}

impl FrameSink for CaptureSink {
    fn on_frame(&mut self, frame: &PitchFrame) {
        // Channel 송출 실패(웹뷰 리로드 등)는 세션을 죽일 이유가 아니다.
        let _ = self.channel.send(*frame);
        let quantized = F0Frame::quantize(frame.midi, frame.voiced, frame.confidence, frame.rms);
        let _ = self.storage.send(StorageMsg::Frame(quantized));
    }

    fn on_audio(&mut self, mono: &[f32]) {
        if self.record_audio {
            let _ = self.storage.send(StorageMsg::Audio(mono.to_vec()));
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[tauri::command]
fn start_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    channel: Channel<PitchFrame>,
    track_id: Option<String>,
) -> Result<CaptureInfo, String> {
    let mut slot = state.capture.lock().unwrap();
    // 웹뷰 리로드 등으로 프론트가 상태를 잃어도 재시작이 막히지 않도록
    // 기존 캡처는 교체한다 (이전 세션은 정상 마감·저장).
    if let Some(old) = slot.take() {
        old.stop();
        end_session(&state, false).ok();
    }

    let engine = make_engine(&app)?;
    let cfg = *state.config.lock().unwrap();
    let info = audio::probe()?;

    let storage_guard = state.storage.lock().unwrap();
    let storage = storage_guard.as_ref().ok_or("스토리지 미초기화")?;
    storage
        .begin(BeginSession {
            id: uuid::Uuid::new_v4().to_string(),
            started_at_ms: now_ms(),
            track_id,
            octave_invariant: cfg.octave_invariant,
            recording: cfg
                .recording_enabled
                .then_some(RecordingSpec { sample_rate: info.sample_rate }),
            recording_keep_last: cfg.recording_keep_last,
        })
        .map_err(|e| e.to_string())?;

    let sink = Box::new(CaptureSink {
        channel,
        storage: storage.sender(),
        record_audio: cfg.recording_enabled,
    });
    let capture = audio::start(engine, cfg.pipeline_params(), sink)?;
    let info = capture.info.clone();
    *slot = Some(capture);
    Ok(info)
}

/// 진행 중 세션을 마감한다. discard=true면 저장하지 않는다.
fn end_session(state: &State<'_, AppState>, discard: bool) -> Result<Option<FinalizedSession>, String> {
    let storage_guard = state.storage.lock().unwrap();
    let storage = storage_guard.as_ref().ok_or("스토리지 미초기화")?;
    storage.end(discard).map_err(|e| e.to_string())
}

#[tauri::command]
fn stop_capture(
    state: State<'_, AppState>,
    discard: Option<bool>,
) -> Result<Option<FinalizedSession>, String> {
    let mut slot = state.capture.lock().unwrap();
    // 멱등: 이미 정지 상태여도 성공 (리로드 후 정리 호출 허용).
    if let Some(c) = slot.take() {
        c.stop();
    }
    drop(slot);
    end_session(&state, discard.unwrap_or(false))
}

/// DSP 관련 설정을 백엔드에 동기화한다. 실행 중인 캡처에는 즉시 적용.
#[tauri::command]
fn configure(state: State<'_, AppState>, config: AppConfig) -> Result<(), String> {
    let config = config.sanitized();
    *state.config.lock().unwrap() = config;
    if let Some(capture) = state.capture.lock().unwrap().as_ref() {
        capture.set_params(config.pipeline_params());
    }
    Ok(())
}

#[tauri::command]
fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionListItem>, String> {
    let conn = db::open_read(&state.root.db_path()).map_err(|e| e.to_string())?;
    vocalboard_storage::queries::list_sessions(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn session_detail(state: State<'_, AppState>, id: String) -> Result<SessionDetail, String> {
    let conn = db::open_read(&state.root.db_path()).map_err(|e| e.to_string())?;
    vocalboard_storage::queries::session_detail(&conn, &id).map_err(|e| e.to_string())
}

#[tauri::command]
fn session_series(
    state: State<'_, AppState>,
    id: String,
    max_points: u32,
) -> Result<SessionSeries, String> {
    let conn = db::open_read(&state.root.db_path()).map_err(|e| e.to_string())?;
    vocalboard_storage::queries::session_series(&conn, &id, max_points).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_vocal_audio::init())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let root = StorageRoot::new(&app_data);
            let (storage, recovered) =
                vocalboard_storage::thread::spawn(root.clone()).map_err(|e| e.to_string())?;
            for id in &recovered {
                eprintln!("[app] 복구된 세션: {id}");
            }
            app.manage(AppState {
                capture: Mutex::new(None),
                config: Mutex::new(AppConfig::default()),
                storage: Mutex::new(Some(storage)),
                root,
            });
            // store에 저장된 설정을 초기 로드 (프론트 configure 이전의 기본).
            let store = app.store("settings.json")?;
            if let Some(v) = store.get("config") {
                if let Ok(cfg) = serde_json::from_value::<AppConfig>(v) {
                    *app.state::<AppState>().config.lock().unwrap() = cfg.sanitized();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 창이 닫혀도 진행 중 세션은 커밋되도록 스토리지를 정리한다.
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.app_handle().state::<AppState>();
                let capture = state.capture.lock().unwrap().take();
                if let Some(c) = capture {
                    c.stop();
                }
                let storage = state.storage.lock().unwrap().take();
                if let Some(s) = storage {
                    s.shutdown();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            start_capture,
            stop_capture,
            configure,
            list_sessions,
            session_detail,
            session_series
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
