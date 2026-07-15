mod audio;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use vocalboard_dsp::{AcfEngine, InferenceEngine, OrtEngine, PipelineParams, PitchFrame};

use audio::{Capture, CaptureInfo};

#[derive(Default)]
struct AppState {
    capture: Mutex<Option<Capture>>,
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
    if slot.is_some() {
        return Err("이미 캡처 중입니다".into());
    }
    let engine = make_engine(&app)?;
    let sink = Box::new(move |f: &PitchFrame| {
        // Channel 송출 실패(웹뷰 리로드 등)는 세션을 죽일 이유가 아니다.
        let _ = channel.send(*f);
    });
    let capture = audio::start(engine, PipelineParams::default(), sink)?;
    let info = capture.info.clone();
    *slot = Some(capture);
    Ok(info)
}

#[tauri::command]
fn stop_capture(state: State<'_, AppState>) -> Result<(), String> {
    let mut slot = state.capture.lock().unwrap();
    match slot.take() {
        Some(c) => {
            c.stop();
            Ok(())
        }
        None => Err("캡처 중이 아닙니다".into()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![start_capture, stop_capture])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
