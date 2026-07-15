# 프로젝트 규칙

단일 진실 원천은 `docs/SPEC.md`다. 세션 시작 시 반드시 읽어라.
스펙 변경이 필요하면 구현하지 말고 먼저 질문하라.

## 가드레일
- 오디오 콜백에서 할당/락/로그/IPC 금지. ringbuf push만.
- 62.5Hz 스트림을 Solid 반응성에 태우지 말 것. Channel 콜백 → PixiJS 직행,
  신호에는 스로틀된 판독값만.
- Solid 2.0 beta 규칙: 정확 버전 고정(자동 업데이트 금지), createSignal/
  createMemo/Show/For 등 정착 프리미티브만 사용, action/낙관적 업데이트/
  async memo 금지, 쓰기는 마이크로태스크 배칭되어 flush 전 읽기가 이전 값임에
  유의, 비즈니스 로직은 프레임워크 무관 TS 모듈로 분리하고 컴포넌트는 얇은 뷰로.
- 추론은 `InferenceEngine` trait 뒤에 두고 ort 구현체로 주입.
- SwiftF0 confidence는 voicing 판단이 아니므로 RMS 게이트를 항상 병행.
  게이트 미달 hop은 추론 생략 가능하나 unvoiced PitchFrame은 반드시 방출
  (62.5Hz 결번 금지 — F0C1 시간축 전제).
- 영속화는 백엔드 rusqlite. 프론트에 SQL 노출 금지.
- 반주 재생+캡처 동시 모드는 헤드폰 라우트 감지 시에만 (스피커=에코 오염).
- iOS AVAudioSession은 .playAndRecord + .measurement를 플러그인에서 직접 설정.
- Android: RECORD_AUDIO 런타임 권한, FOREGROUND_SERVICE_MICROPHONE(백그라운드),
  오디오 포커스, FLAG_KEEP_SCREEN_ON. (이상 Phase 1 플러그인 범위)
- 모델 바이너리를 저장소에 커밋 금지. 확보 절차만 README에 문서화.
- 의존성은 정확 버전 고정(^·~·범위·dist-tag 금지, 도입 시점 최신을 박제),
  lockfile 커밋. 스펙 §3의 "최신/@next" 표기는 도입 시점 해석 지침일 뿐이다.
