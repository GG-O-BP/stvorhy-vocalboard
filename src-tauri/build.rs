use std::io::Read;
use std::{env, fs, path::PathBuf};

use sha2::{Digest, Sha256};

// SwiftF0 (lars76/swift-f0, MIT, ~398KB). 단일 진실 원천:
// crates/dsp/src/ort_engine.rs(SWIFTF0_URL/SWIFTF0_SHA256),
// crates/reference/src/download.rs(SWIFTF0). 값 변경 시 함께 갱신 — SHA-256으로 검증.
const SWIFTF0_URL: &str = "https://raw.githubusercontent.com/lars76/swift-f0/main/swift_f0/model.onnx";
const SWIFTF0_SHA256: &str = "7e2390db8379cd9e1e2b22828e55b45b57c8559e4c8335678c717dc245c18176";

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// SwiftF0를 빌드 시점에 확보해 `OUT_DIR/swift_f0.onnx`에 둔다.
/// lib.rs가 `include_bytes!`로 앱 바이너리에 임베드한다(모델 바이너리 저장소 커밋 금지 가드레일 유지).
fn embed_swiftf0() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=VOCALBOARD_SWIFTF0");

    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("swift_f0.onnx");

    // 캐시 히트: 이미 받아둔 파일이 해시 일치하면 재사용(오프라인 재빌드 가능).
    if let Ok(existing) = fs::read(&dest) {
        if sha256_hex(&existing) == SWIFTF0_SHA256 {
            return;
        }
    }

    // 오프라인/CI 오버라이드: VOCALBOARD_SWIFTF0=<로컬 swift_f0.onnx 경로>이면 그걸 임베드.
    let bytes: Vec<u8> = if let Ok(path) = env::var("VOCALBOARD_SWIFTF0") {
        fs::read(&path).unwrap_or_else(|e| panic!("VOCALBOARD_SWIFTF0={path} 읽기 실패: {e}"))
    } else {
        let resp = ureq::get(SWIFTF0_URL).call().unwrap_or_else(|e| {
            panic!(
                "SwiftF0 다운로드 실패({SWIFTF0_URL}): {e}\n\
                 오프라인이면 VOCALBOARD_SWIFTF0=<로컬 swift_f0.onnx 경로>로 지정하세요."
            )
        });
        let mut buf = Vec::new();
        resp.into_body()
            .into_reader()
            .read_to_end(&mut buf)
            .unwrap_or_else(|e| panic!("SwiftF0 본문 읽기 실패: {e}"));
        buf
    };

    let actual = sha256_hex(&bytes);
    assert_eq!(
        actual, SWIFTF0_SHA256,
        "SwiftF0 SHA-256 불일치: expected {SWIFTF0_SHA256}, got {actual}"
    );
    fs::write(&dest, &bytes).unwrap_or_else(|e| panic!("{}: 쓰기 실패: {e}", dest.display()));
}

fn main() {
    embed_swiftf0();
    tauri_build::build();
}
