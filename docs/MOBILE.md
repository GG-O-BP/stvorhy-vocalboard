# 모바일 빌드 가이드 (Phase 1)

이 저장소의 개발 환경(Windows, Android SDK 미설치)에서는 `tauri android
init`/`tauri ios init`을 실행할 수 없어 `gen/` 프로젝트가 커밋되어 있지
않다. 실기기 빌드는 아래 절차를 따른다. 플러그인 소스
(`crates/tauri-plugin-vocal-audio`)는 완성되어 있으며 init 후 자동으로
링크된다.

## 공통

- 플러그인 커맨드: 권한(check/request), 오디오 세션(configure/release),
  화면 유지, 라우트 조회, 포그라운드 서비스(start/stop), 백업 제외.
- 이벤트: `interruption`(iOS), `routeChanged`, `focusChanged`(Android).
- JS 래퍼: `src/lib/vocalAudio.js`.

## Android

1. Android Studio + SDK(34+) + NDK + `ANDROID_HOME`/`NDK_HOME` 설정
   (https://tauri.app/start/prerequisites/#android).
2. `bun run tauri android init` — `gen/android` 생성. 플러그인의
   AndroidManifest(권한: RECORD_AUDIO, FOREGROUND_SERVICE,
   FOREGROUND_SERVICE_MICROPHONE, POST_NOTIFICATIONS, BLUETOOTH_CONNECT
   + microphone 타입 서비스)는 gradle manifest merger로 앱에 병합된다.
3. `gen/android/app/src/main/AndroidManifest.xml`의 `<application>`에
   추가 권장:
   - `android:allowBackup="false"` — 녹음·스템 백업 제외
     (또는 `dataExtractionRules`로 `recordings/`·`tracks/` 경로만 제외).
4. `bun run tauri android dev|build`.

## iOS (macOS 필요)

1. Xcode 15+.
2. `bun run tauri ios init` — `gen/apple` 생성.
3. `gen/apple/<app>_iOS/Info.plist`에 추가:
   ```xml
   <key>NSMicrophoneUsageDescription</key>
   <string>피치 분석을 위해 마이크를 사용합니다.</string>
   <key>UIBackgroundModes</key>
   <array><string>audio</string></array>
   ```
4. AVAudioSession(.playAndRecord + .measurement)은 플러그인
   `configureSession`이 직접 설정한다. 백업 제외는 플러그인
   `excludeFromBackup(paths)`가 NSURLIsExcludedFromBackupKey로 처리.
5. `bun run tauri ios dev|build`.

## ort(onnxruntime) 모바일 링크

`crates/dsp`의 피처로 제어한다:

| 타깃 | 권장 구성 |
|---|---|
| iOS | 기본(`download-binaries`, 정적 링크) + `--features vocalboard-dsp/ep-coreml` |
| Android | `--features vocalboard-dsp/ep-xnnpack` (또는 `ep-nnapi`). 정적 바이너리 미제공 시 `ort-load-dynamic` + `libonnxruntime.so`를 `gen/android/app/src/main/jniLibs/<abi>/`에 동봉 |

- pyke(ort) CDN이 해당 rc 버전의 iOS/Android 프리빌드를 제공하지 않으면
  onnxruntime을 소스 빌드해 `ORT_LIB_LOCATION`으로 지정한다
  (https://ort.pyke.io/setup/linking).
- **사람 확인 필요**: 실기기에서 (a) 정적 링크 성공, (b) EP 활성화 로그,
  (c) `.measurement` 모드 실효(AGC 비활성), (d) E2E 지연 60–80ms 충족.

## 백그라운드 동작

- Android: 캡처 시작 시 `startForegroundService()` 호출됨
  (microphone 타입). API 33+ 알림 표시는 POST_NOTIFICATIONS 런타임 승인
  필요 — 미승인이어도 서비스는 동작한다.
- iOS: UIBackgroundModes audio + 활성 AVAudioSession으로 유지된다.
