//! 커스텀 모바일 오디오 환경 플러그인 (스펙 §3, CLAUDE.md 가드레일 범위).
//!
//! 담당: 마이크 권한, 오디오 세션(iOS AVAudioSession .playAndRecord +
//! .measurement / Android 오디오 포커스), 화면 유지, 헤드폰 라우트 감지,
//! 포그라운드 서비스(FOREGROUND_SERVICE_MICROPHONE), 백업 제외 플래그.
//! 데스크톱은 개발용 스텁 (권한 항상 granted, 라우트 unknown).
//!
//! 이벤트 (모바일 → 웹뷰, `addPluginListener`):
//! - `interruption` { phase: "began"|"ended", shouldResume: bool }
//! - `routeChanged` { headphones: bool, description: string }
//! - `focusChanged` { state: "gain"|"loss"|"lossTransient"|"lossTransientCanDuck" }

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

mod commands;
mod error;
mod models;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

pub use error::{Error, Result};
pub use models::*;

#[cfg(desktop)]
pub use desktop::VocalAudio;
#[cfg(mobile)]
pub use mobile::VocalAudio;

/// `AppHandle`/`App`에서 플러그인 API에 접근하는 확장.
pub trait VocalAudioExt<R: Runtime> {
    fn vocal_audio(&self) -> &VocalAudio<R>;
}

impl<R: Runtime, T: Manager<R>> VocalAudioExt<R> for T {
    fn vocal_audio(&self) -> &VocalAudio<R> {
        self.state::<VocalAudio<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("vocal-audio")
        .invoke_handler(tauri::generate_handler![
            commands::check_permissions,
            commands::request_permissions,
            commands::configure_session,
            commands::release_session,
            commands::set_keep_screen_on,
            commands::current_route,
            commands::start_foreground_service,
            commands::stop_foreground_service,
            commands::exclude_from_backup,
        ])
        .setup(|app, api| {
            #[cfg(mobile)]
            let vocal_audio = mobile::init(app, api)?;
            #[cfg(desktop)]
            let vocal_audio = desktop::init(app, api)?;
            app.manage(vocal_audio);
            Ok(())
        })
        .build()
}
