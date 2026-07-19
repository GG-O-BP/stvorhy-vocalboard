mod audio;
mod config;
mod player;
mod tracks;

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

pub(crate) struct AppState {
    pub(crate) capture: Mutex<Option<Capture>>,
    pub(crate) config: Mutex<AppConfig>,
    pub(crate) storage: Mutex<Option<StorageHandle>>,
    pub(crate) playback: Mutex<Option<player::Player>>,
    pub(crate) root: StorageRoot,
}

/// 빌드 시점(build.rs)에 확보해 앱 바이너리에 임베드한 SwiftF0 모델(~398KB).
/// resolve 경로가 모두 비면(모바일/신규 설치) 이것을 사용한다.
pub(crate) static SWIFTF0_MODEL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/swift_f0.onnx"));

/// SwiftF0 모델 탐색: 환경변수 → app_data/models → (dev 빌드) 저장소 models/.
pub(crate) fn resolve_model_path(app: &AppHandle) -> Option<PathBuf> {
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

/// cpal(AAudio 백엔드)의 기기·설정 조회는 JNI를 거치는데, 그 진입점인
/// ndk_context를 Tauri(tao/wry)는 초기화하지 않는다. 미초기화 상태에서 캡처를
/// 시작하면 `ndk_context::android_context()`가 패닉해 앱이 그대로 죽으므로,
/// 웹뷰 JNI 핸들에서 JavaVM과 애플리케이션 컨텍스트를 얻어 1회 초기화한다.
#[cfg(target_os = "android")]
fn init_ndk_context(app: &AppHandle) -> tauri::Result<()> {
    let window = app
        .get_webview_window("main")
        .expect("main 웹뷰 윈도우가 setup 전에 생성되어 있어야 한다");
    window.with_webview(|webview| {
        webview.jni_handle().exec(|env, activity, _webview| {
            let mut init = || -> Result<(), jni::errors::Error> {
                let vm = env.get_java_vm()?;
                // 액티비티 재생성과 무관하게 유효하도록 앱 컨텍스트를 쓴다.
                let ctx = env
                    .call_method(
                        activity,
                        "getApplicationContext",
                        "()Landroid/content/Context;",
                        &[],
                    )?
                    .l()?;
                let ctx = env.new_global_ref(ctx)?;
                unsafe {
                    ndk_context::initialize_android_context(
                        vm.get_java_vm_pointer().cast(),
                        ctx.as_raw().cast(),
                    );
                }
                // ndk_context가 프로세스 수명 동안 참조하므로 해제하지 않는다.
                std::mem::forget(ctx);
                Ok(())
            };
            if let Err(e) = init() {
                eprintln!("[app] ndk_context 초기화 실패 — 마이크 캡처 불가: {e}");
            }
        });
    })
}

fn make_engine(app: &AppHandle) -> Result<Box<dyn InferenceEngine>, String> {
    if std::env::var("VOCALBOARD_ENGINE").is_ok_and(|v| v == "acf") {
        return Ok(Box::new(AcfEngine::new()));
    }
    // resolve 경로(환경변수/app_data/dev 저장소)가 있으면 그 파일을, 없으면 빌드 시점에
    // 임베드된 모델(build.rs + include_bytes!)을 쓴다 — 모바일/신규 설치는 후자.
    let engine = match resolve_model_path(app) {
        Some(p) => OrtEngine::from_file(&p),
        None => OrtEngine::from_memory(SWIFTF0_MODEL),
    }
    .map_err(|e| format!("SwiftF0 로드 실패: {e}"))?;
    Ok(Box::new(engine))
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

/// 세션+캡처 공통 시작 경로 (라이브/연습 겸용).
fn begin_capture_session(
    app: &AppHandle,
    state: &State<'_, AppState>,
    channel: Channel<PitchFrame>,
    track_id: Option<String>,
) -> Result<CaptureInfo, String> {
    let mut slot = state.capture.lock().unwrap();
    // 웹뷰 리로드 등으로 프론트가 상태를 잃어도 재시작이 막히지 않도록
    // 기존 캡처는 교체한다 (이전 세션은 정상 마감·저장).
    if let Some(old) = slot.take() {
        old.stop();
        end_session(state, false).ok();
    }

    let engine = make_engine(app)?;
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
            latency_calib_ms: cfg.latency_calib_ms,
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

#[tauri::command]
fn start_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    channel: Channel<PitchFrame>,
    track_id: Option<String>,
) -> Result<CaptureInfo, String> {
    begin_capture_session(&app, &state, channel, track_id)
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

/// 세션 녹음 재생 시작. 플레이헤드는 Channel로 ~20Hz 동기.
#[tauri::command]
fn play_recording(
    state: State<'_, AppState>,
    session_id: String,
    start_ms: Option<u32>,
    channel: Channel<player::PlayheadEvent>,
) -> Result<u32, String> {
    let conn = db::open_read(&state.root.db_path()).map_err(|e| e.to_string())?;
    let detail =
        vocalboard_storage::queries::session_detail(&conn, &session_id).map_err(|e| e.to_string())?;
    let rel = detail
        .recording_path
        .ok_or("이 세션에는 녹음이 없습니다 (보존 정책으로 삭제되었을 수 있음)")?;
    let path = state.root.app_data.join(rel);
    if !path.exists() {
        return Err("녹음 파일이 없습니다".into());
    }
    let clip = player::Clip::from_file(&path)?;
    let duration_ms = clip.duration_ms();

    let mut slot = state.playback.lock().unwrap();
    if let Some(old) = slot.take() {
        old.stop();
    }
    let p = player::play(
        clip,
        start_ms.unwrap_or(0),
        Box::new(move |e| {
            let _ = channel.send(e);
        }),
    )?;
    *slot = Some(p);
    Ok(duration_ms)
}

#[tauri::command]
fn playback_pause(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(p) = state.playback.lock().unwrap().as_ref() {
        p.pause();
    }
    Ok(())
}

#[tauri::command]
fn playback_resume(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(p) = state.playback.lock().unwrap().as_ref() {
        p.resume();
    }
    Ok(())
}

#[tauri::command]
fn playback_seek(state: State<'_, AppState>, t_ms: u32) -> Result<(), String> {
    if let Some(p) = state.playback.lock().unwrap().as_ref() {
        p.seek_ms(t_ms);
    }
    Ok(())
}

#[tauri::command]
fn playback_stop(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(p) = state.playback.lock().unwrap().take() {
        p.stop();
    }
    Ok(())
}

/// 헤드폰 라우트 게이트 (가드레일: 반주 재생+캡처 동시 모드는 헤드폰
/// 라우트 감지 시에만 — 스피커는 에코 오염).
fn check_headphone_gate(app: &AppHandle, headphone_override: bool) -> Result<(), String> {
    use tauri_plugin_vocal_audio::VocalAudioExt;
    let route = app
        .vocal_audio()
        .current_route()
        .map_err(|e| e.to_string())?;
    match route.headphones {
        Some(true) => Ok(()),
        Some(false) if headphone_override => Ok(()),
        Some(false) => Err(format!(
            "HEADPHONE_GATE:스피커 출력이 감지되었습니다 ({}) — 에코가 분석을 오염시킵니다. \
             헤드폰을 연결하세요.",
            route.description
        )),
        None if headphone_override => Ok(()),
        None => Err(
            "HEADPHONE_GATE:출력 라우트를 판별할 수 없습니다. 헤드폰 사용을 확인해 주세요."
                .into(),
        ),
    }
}

#[derive(serde::Serialize)]
struct PracticeInfo {
    capture: CaptureInfo,
    duration_ms: u32,
}

/// 연습 모드: 반주 재생 + 캡처 + 채점 세션을 한 번에 시작한다.
/// `playback=false`면 무반주 연습 (라우트 게이트 없음).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn practice_start(
    app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
    playback: bool,
    headphone_override: bool,
    pitch: Channel<PitchFrame>,
    playhead: Channel<player::PlayheadEvent>,
) -> Result<PracticeInfo, String> {
    if tracks::job_running(&app, &track_id) {
        return Err("이 트랙은 분리 작업이 진행 중입니다".into());
    }
    let clip = if playback {
        check_headphone_gate(&app, headphone_override)?;
        let path = tracks::accompaniment_path(&state.root, &track_id);
        if !path.exists() {
            return Err("반주 스템이 없습니다 — 트랙 분리를 먼저 실행하세요".into());
        }
        Some(player::Clip::from_file(&path)?)
    } else {
        None
    };

    let capture = begin_capture_session(&app, &state, pitch, Some(track_id))?;

    let duration_ms = match clip {
        Some(clip) => {
            let d = clip.duration_ms();
            let mut slot = state.playback.lock().unwrap();
            if let Some(old) = slot.take() {
                old.stop();
            }
            let p = player::play(
                clip,
                0,
                Box::new(move |e| {
                    let _ = playhead.send(e);
                }),
            )?;
            *slot = Some(p);
            d
        }
        None => 0,
    };
    Ok(PracticeInfo { capture, duration_ms })
}

/// 연습 종료: 재생·캡처를 멈추고 세션을 마감(채점 포함)한다.
#[tauri::command]
fn practice_stop(state: State<'_, AppState>) -> Result<Option<FinalizedSession>, String> {
    if let Some(p) = state.playback.lock().unwrap().take() {
        p.stop();
    }
    let mut slot = state.capture.lock().unwrap();
    if let Some(c) = slot.take() {
        c.stop();
    }
    drop(slot);
    end_session(&state, false)
}

/// 노래방 모드: 반주만 재생 (캡처 없음 — 스피커 허용).
#[tauri::command]
fn karaoke_start(
    state: State<'_, AppState>,
    track_id: String,
    playhead: Channel<player::PlayheadEvent>,
) -> Result<u32, String> {
    let path = tracks::accompaniment_path(&state.root, &track_id);
    if !path.exists() {
        return Err("반주 스템이 없습니다 — 트랙 분리를 먼저 실행하세요".into());
    }
    let clip = player::Clip::from_file(&path)?;
    let duration_ms = clip.duration_ms();
    let mut slot = state.playback.lock().unwrap();
    if let Some(old) = slot.take() {
        old.stop();
    }
    let p = player::play(
        clip,
        0,
        Box::new(move |e| {
            let _ = playhead.send(e);
        }),
    )?;
    *slot = Some(p);
    Ok(duration_ms)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_vocal_audio::init())
        .setup(|app| {
            #[cfg(target_os = "android")]
            init_ndk_context(app.handle())?;
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
                playback: Mutex::new(None),
                root,
            });
            tracks::init(app.handle());
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
            session_series,
            play_recording,
            playback_pause,
            playback_resume,
            playback_seek,
            playback_stop,
            tracks::import_track,
            tracks::separate_track,
            tracks::list_tracks,
            tracks::track_series,
            tracks::delete_track,
            practice_start,
            practice_stop,
            karaoke_start
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
