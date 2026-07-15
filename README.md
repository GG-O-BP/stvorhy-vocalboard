# Vocalboard

실시간 보컬 피치 모니터 (Tauri 2 + Solid 2.0 beta + PixiJS v8 + Rust DSP).
단일 진실 원천은 [docs/SPEC.md](docs/SPEC.md)다.

## 개발 환경

- bun (프론트), Rust stable (백엔드), 데스크톱 빌드는 개발·검증용
- `bun install` 후:

| 명령 | 설명 |
|---|---|
| `bun run tauri dev` | 데스크톱 개발 실행 |
| `bun run test` | vitest (프론트 단위 테스트) |
| `bun run typecheck` | tsc --noEmit (JSDoc 타입 검사) |
| `cargo test` | Rust 전체 테스트 |

마이크 없는 환경에서는 `VOCALBOARD_SIM_INPUT=1`로 실행하면 A4 비브라토
시뮬레이터 입력(2초 발성/2초 무음)이 캡처 대신 공급된다.
`VOCALBOARD_ENGINE=acf`는 모델 없이 자기상관 기준선 엔진으로 돌린다(개발용).
`VOCALBOARD_SEP=stub`은 보컬 분리를 결정적 스텁(vocals=accomp=mix×0.5)으로
대체한다 — 분리 모델 없이 트랙 파이프라인을 시험할 때 사용.

## 사용 흐름

1. **라이브**: 시작 → 피아노롤에 실시간 피치, 세션은 자동 저장(+마이크 WAV
   녹음, 설정에서 보존 개수 제어).
2. **트랙**: 파일 가져오기 → 분리(모델 온디맨드 다운로드) → 연습(반주/무반주,
   스피커 감지 시 동시 모드 차단) 또는 노래방(반주만 재생). 연습 종료 시
   cents 편차 채점(설정에서 옥타브 불변 토글).
3. **세션**: 목록/상세 그래프, 녹음 재생 오버레이(클릭 시킹).

## 모델 확보 (바이너리는 저장소에 커밋하지 않는다)

### SwiftF0 피치 모델 (필수, MIT)

앱은 아래 순서로 모델을 찾는다:
`VOCALBOARD_SWIFTF0` 환경변수 → `<app_data>/models/swift_f0.onnx` →
(개발 빌드 한정) 저장소 `models/swift_f0.onnx`.

수동 확보:

```powershell
# 저장소 루트에서 (개발용)
mkdir models -Force
curl.exe -sL -o models/swift_f0.onnx `
  https://raw.githubusercontent.com/lars76/swift-f0/main/swift_f0/model.onnx
```

- 크기: 397,987 bytes
- SHA-256: `7e2390db8379cd9e1e2b22828e55b45b57c8559e4c8335678c717dc245c18176`
- 출처: [lars76/swift-f0](https://github.com/lars76/swift-f0) (MIT)

`cargo test`의 SwiftF0 통합 테스트는 모델이 없으면 SKIP 메시지를 내고
통과 처리된다 — 실검증하려면 위 파일을 놓고 다시 실행하라.

### 보컬 분리 모델 (Phase 3.5, 온디맨드)

앱 내 다운로드 매니저가 첫 사용 시 내려받는다 (재개 가능 + SHA-256 검증).
수동 배치 경로: `<app_data>/models/`.

| 모드 | 파일 | 출처 |
|---|---|---|
| 고속(기본) | `UVR-MDX-NET-Voc_FT.onnx` (66.8MB) | [TRvlvr/model_repo](https://github.com/TRvlvr/model_repo/releases/tag/all_public_uvr_models) |
| 품질 | `htdemucs_ft_vocals.onnx` (316MB) | [StemSplitio/htdemucs-ft-vocals-onnx](https://huggingface.co/StemSplitio/htdemucs-ft-vocals-onnx) (MIT) |

체크섬·URL 상수는 `crates/reference/src/download.rs`에 박제되어 있다.
