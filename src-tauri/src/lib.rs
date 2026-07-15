mod audio;
mod config;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;
use vocalboard_dsp::{AcfEngine, InferenceEngine, OrtEngine, PitchFrame};

use audio::{Capture, CaptureInfo};
use config::AppConfig;

#[derive(Default)]
struct AppState {
    capture: Mutex<Option<Capture>>,
    config: Mutex<AppConfig>,
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

#[tauri::command]
fn start_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    channel: Channel<PitchFrame>,
) -> Result<CaptureInfo, String> {
    let mut slot = state.capture.lock().unwrap();
    // 웹뷰 리로드 등으로 프론트가 상태를 잃어도 재시작이 막히지 않도록
    // 기존 캡처는 교체한다.
    if let Some(old) = slot.take() {
        old.stop();
    }
    let engine = make_engine(&app)?;
    let sink = Box::new(move |f: &PitchFrame| {
        // Channel 송출 실패(웹뷰 리로드 등)는 세션을 죽일 이유가 아니다.
        let _ = channel.send(*f);
    });
    let params = state.config.lock().unwrap().pipeline_params();
    let capture = audio::start(engine, params, sink)?;
    let info = capture.info.clone();
    *slot = Some(capture);
    Ok(info)
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
fn stop_capture(state: State<'_, AppState>) -> Result<(), String> {
    let mut slot = state.capture.lock().unwrap();
    // 멱등: 이미 정지 상태여도 성공 (리로드 후 정리 호출 허용).
    if let Some(c) = slot.take() {
        c.stop();
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(AppState::default())
        .setup(|app| {
            // store에 저장된 설정을 초기 로드 (프론트 configure 이전의 기본).
            let store = app.store("settings.json")?;
            if let Some(v) = store.get("config") {
                if let Ok(cfg) = serde_json::from_value::<AppConfig>(v) {
                    *app.state::<AppState>().config.lock().unwrap() = cfg.sanitized();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_capture,
            stop_capture,
            configure
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
