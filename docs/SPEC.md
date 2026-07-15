# 보컬 피치 모니터 — 기술 스펙 (2026-07 확정)

## §1 제품 개요
- 실시간 보컬 피치 모니터: 마이크 입력의 피치를 스크롤 피아노롤로 시각화
- 세션 녹음(WAV) 및 세션 기록 저장/리뷰
- 레퍼런스 트랙: 오디오 파일 임포트 → 보컬 분리 → 레퍼런스 피치를 그래프에
  보조 표시 → 같은 피치로 따라 부르는 연습 + cents 편차 채점(옥타브 불변 옵션)
- 타겟: iOS / Android (Tauri 2 모바일). 데스크톱 빌드는 개발·검증용.

## §2 아키텍처 (데이터 흐름)
```
cpal 입력 콜백 (RT 스레드: 할당/락/IO 금지, ringbuf push만)
 └→ ringbuf (SPSC)
     └→ DSP 스레드: biquad HPF(70Hz) → 48k→16k 다운샘플 → RMS 게이트
         → SwiftF0 추론(ort) → PitchFrame 조립
         ├→ tauri::ipc::Channel (62.5Hz) → WebView
         │    ├→ Channel 콜백에서 PixiJS 직접 갱신 (프레임워크 우회)
         │    └→ Solid 신호: 판독값만 10–15Hz 스로틀
         └→ mpsc → 스토리지 스레드 (Phase 2)
재생(Phase 3.5): cpal 출력 스트림, 플레이헤드=소비 샘플 수, Channel로 동기
```

RMS 게이트 미달 hop은 SwiftF0 추론을 생략할 수 있으나, 그 경우에도 unvoiced
PitchFrame을 반드시 방출한다(62.5Hz 프레임 결번 금지 — §5 F0C1의
"타임스탬프=인덱스×16ms" 전제).

## §3 기술 스택 (확정)
| 계층 | 선택 | 비고 | 도입 단계 |
|---|---|---|---|
| 앱 프레임워크 | Tauri 2.x 최신 | Channel IPC, Swift/Kotlin 플러그인 | 0 |
| 캡처 | cpal 0.17.x + `realtime` | Android AAudio, iOS CoreAudio | 0 |
| 버퍼 | ringbuf | SPSC lock-free | 0 |
| 필터 | biquad | HPF 70Hz | 0 |
| 리샘플 | 정수비 FIR 데시메이터(48k→16k), 비정수 SR은 rubato | | 0 |
| 피치 엔진 | SwiftF0 ONNX (MIT, 16kHz/hop256, 46.875–2093.75Hz) | confidence는 voicing 아님 → RMS 게이트 필수 | 0 |
| 추론 런타임 | ort (2.0-rc, 정확 버전 고정) | `InferenceEngine` trait 뒤에 배치. CoreML/NNAPI/XNNPACK EP는 모바일 단계 | 0 |
| DSP 폴백/AB | pyin (pYIN 구현) | 검증용 A/B 기준선. viterbi 스무딩 포함이라 보컬 F0 레퍼런스로 적합 | 2 |
| 세션 DB | rusqlite 0.39 `bundled` + WAL + rusqlite_migration | 백엔드 직결. tauri-plugin-sql 금지 | 2 |
| 프레임 코덱 | 자체 F0C1 + zstd | §5 | 0(코덱)/2(저장) |
| 설정 | tauri-plugin-store + `configure` 커맨드로 백엔드 동기화 | | 1 |
| 녹음 | hound 증분 WAV + 크래시 헤더 복구 | 마이크 드라이만 | 2 |
| 임포트 | tauri-plugin-dialog, Android content://는 앱 디렉토리로 복사 | | 3.5 |
| 디코딩 | Symphonia 0.6 | AAC-LC 가능, HE-AAC/DRM 불가 안내 | 3.5 |
| 보컬 분리 | 둘 다 탑재 — 기본: MDX-Net 계열 보컬 전용(경량) / 품질 옵션: HTDemucs FT ONNX(MIT). 모두 온디맨드 다운로드 | 곡당 1회, 사용 모델을 `sep_model`에 기록, 결과 캐시. 기본값 타당성은 Phase 3.5 첫 측정으로 검증 | 3.5 |
| 재생 | cpal 출력 스트림 직접 | | 3.5 |
| 프론트 | Solid 2.0 beta(정확 버전 고정) + Vite + vite-plugin-solid@next + TS | SolidStart/라우터 없음, CSR 단일 뷰 | 0 |
| 렌더 | PixiJS v8 | WebGPU 우선 + WebGL 자동 폴백, 스크롤 텍스처 | 0 |
| 테스트 | cargo test / vitest + @solidjs/testing-library#next | | 0 |
| 모바일 플러그인 | 커스텀 Swift/Kotlin | 범위는 CLAUDE.md 가드레일 참조 | 1 |

버전 표기 규칙: 표의 "최신"·"x.x"·"@next"·"#next"는 **도입 시점에** 해당 채널의
최신 버전을 확인해 정확 버전으로 박제하라는 지침이다. 매니페스트에는 정확
버전만 기재하고(^·~·범위·dist-tag 금지) lockfile을 커밋한다. 이후 업그레이드는
건별 의도적 결정으로만 한다.

## §4 파라미터
| 항목 | 값 |
|---|---|
| 캡처 SR / 분석 SR | 48kHz / 16kHz |
| hop | 256 샘플 @16k = 16ms (62.5Hz 갱신) |
| confidence 임계 | 0.9 (설정화) |
| RMS 게이트 임계 | -45 dBFS (설정화) |
| voicing | RMS 게이트 AND confidence 임계 (게이트 미달 시 추론 생략 가능, 프레임은 항상 방출) |
| E2E 지연 예산 | 60–80ms |
| DOM 판독값 | 10–15Hz 스로틀 |

"(설정화)" 표기 항목은 설정 시스템(tauri-plugin-store, Phase 1) 도입 전까지
상수로 하드코딩한다.

## §5 데이터 정의
PitchFrame (Channel, serde):
`{ t:u32(ms), f0:f32, midi:f32, cents:f32, confidence:f32, rms:f32, voiced:bool }`
— f0→midi→cents 변환과 voicing 판정은 Rust에서 완료, 프론트는 렌더만.
cents는 최근접 반음 대비 편차 [-50,+50), 기준은 12평균율 A4=440Hz 고정.
음이름 표시는 프론트가 round(midi)→이름 테이블 조회로 처리한다(조회는 렌더로
간주, 계산 아님). 프레임은 모든 hop마다 방출: 게이트 미달 시
f0/midi/cents=0·confidence=0·voiced=false에 rms는 실측값, 게이트 통과·
confidence 미달 시 추론값을 채우되 voiced=false.

F0C1 코덱 (4B/frame): `u16 midi_cents`(MIDI×100, 0=unvoiced), `u8 conf`(×255),
`u8 rms`(clamp(round(dBFS)+96, 0, 96), 1dB/step, 복원 = 값−96). 타임스탬프는
인덱스×16ms로 암시 — 무성 프레임 포함 결번 없음이 전제. 헤더: 버전, hop_ms,
시작 시각, rms_offset(=96). 저장 시 zstd. (15KB/분 비압축)

채점 (Phase 3.5): 사용자·레퍼런스 프레임을 재생 플레이헤드 기준 동일 16ms
그리드로 정렬(인덱스 = floor(플레이헤드 ms / 16)). Δcents = 1200·log2(f0_u/f0_r).
옥타브 불변 세션은 Δ ← ((Δ+600) mod 1200) − 600 으로 접는다. 프레임 점수 =
100 × clamp(1 − max(0, |Δ| − 20) / 80, 0, 1) — ±20 cents까지 만점, 반음(100
cents) 이상 0점. 집계 대상은 레퍼런스 voiced 프레임 전체: 사용자도 voiced면 위
점수, 사용자 unvoiced면 0점(안 부른 구간 감점). 사용자만 voiced인 프레임
(애드리브)은 무패널티 제외. `mean_score`는 대상 프레임 평균, `mean_abs_cents`는
양쪽 voiced 교집합의 |Δ| 평균(옥타브 불변 세션은 접은 값 기준).

preview BLOB: `[u8 버전=1][u8×256]` — 전체 길이를 256 버킷으로 등분, 버킷당
voiced 프레임 midi 중앙값을 `clamp(round((midi−20)×2), 1, 255)`로 양자화, 무성
버킷은 0. (sessions.preview / tracks.preview 공통, 목록 썸네일용)

SQLite (Phase 2+, WAL, synchronous=NORMAL, foreign_keys=ON(연결마다 PRAGMA),
쓰기 연결은 스토리지 스레드 단독 소유):
```sql
sessions(id TEXT PK, started_at INTEGER, duration_ms INTEGER,
  track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL,
  frame_count INTEGER, voiced_ratio REAL,
  midi_min REAL, midi_max REAL, mean_abs_cents REAL, mean_score REAL,
  octave_invariant INTEGER, preview BLOB, codec TEXT, recording_path TEXT);
session_frames(session_id TEXT PK REFERENCES sessions(id) ON DELETE CASCADE, data BLOB);
tracks(id TEXT PK, title TEXT, source_path TEXT, duration_ms INTEGER,
  separated INTEGER, sep_model TEXT, pitch_codec TEXT, pitch BLOB,
  notes_json TEXT, preview BLOB, created_at INTEGER);
```
파일 배치: DB `app_data/db/app.sqlite3`, 녹음 `app_data/recordings/`,
스템 `app_data/tracks/{id}/`(FLAC). 녹음·스템은 iOS/Android 백업 제외.

## §6 로드맵
| 단계 | 내용 | 통과 게이트 |
|---|---|---|
| 0 | 사전 정비(기존 스캐폴드 의존성 정확 버전 고정, git init + lockfile 커밋) + 스파이크(데스크톱): 골격 + cpal→SwiftF0→Channel→PixiJS 수직 관통 + F0C1 코덱 모듈(단위 테스트까지, 저장 연동은 Phase 2) | 관통 성공, 실패 시 스택 재결정 |
| 1 | 커스텀 모바일 플러그인(권한·세션·포커스·화면유지·헤드폰 라우트 감지) + ort 모바일 정적 링크 검증 | 양 플랫폼 실기기 캡처 안정 |
| 2 | DSP 확정 + 영속화(스토리지 스레드, F0C1 저장, 녹음, 크래시 복구) + 엔진 A/B | 정확도·지연 예산 충족 |
| 3 | 렌더·UX(피아노롤 완성, 세션 목록/리뷰) | 저사양 기기 60fps |
| 3.5 | 레퍼런스 트랙(임포트→디코드→분리→추출→오버레이→동기재생→채점, 모델 다운로드 매니저). 첫 작업: 실기기 분리 처리시간 측정 | 이어폰 게이트 포함 E2E 동작 |
| 4 | 모바일 경화(인터럽션/라우트/포그라운드 서비스/BT/배터리, 기기 매트릭스) | 매트릭스 통과 |
| 5 | 릴리스(Solid RC/정식 승격 시도, 버전 고정, 스토어 제출) | — |

## 개정 이력
- 2026-07-15: 초판 전사 시 발견된 모순·모호점 해소 — RMS 게이트 의미와 프레임
  연속성 명시(§2·§4·§5), F0C1 코덱 Phase 0 범위 확정(§6), 버전 표기 규칙
  추가(§3), DSP 폴백 pyin 확정·보컬 분리 이중 탑재(기본 MDX-Net) 확정(§3),
  cents 기준·음이름 처리 명시(§5), 채점 공식·preview 포맷·rms 인코딩 정의(§5),
  track_id ON DELETE SET NULL + foreign_keys=ON(§5), Phase 0 사전 정비 추가(§6).
