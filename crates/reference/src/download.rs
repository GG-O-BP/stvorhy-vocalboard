//! 모델 다운로드 매니저 (스펙 §3/§8: 온디맨드, 재개 가능 + 체크섬 검증).
//!
//! `.part` 파일에 이어받기(Range) 후 SHA-256 검증 → 원자적 rename.
//! 공개 체크섬이 없는 모델은 TOFU: 최초 성공 다운로드의 해시를 `.sha256`
//! 사이드카에 박제하고 이후 다운로드에서 검증한다.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::ReferenceError;

/// 원격 모델 정의. URL·체크섬은 도입 시점에 박제 (README '모델 확보').
#[derive(Debug, Clone, Copy)]
pub struct RemoteModel {
    pub file_name: &'static str,
    pub url: &'static str,
    /// 알려진 SHA-256 (hex). None이면 TOFU.
    pub sha256: Option<&'static str>,
    /// 표시용 크기 힌트 (bytes).
    pub size_hint: Option<u64>,
}

/// SwiftF0 피치 모델 (lars76/swift-f0, MIT).
pub const SWIFTF0: RemoteModel = RemoteModel {
    file_name: "swift_f0.onnx",
    url: "https://raw.githubusercontent.com/lars76/swift-f0/main/swift_f0/model.onnx",
    sha256: Some("7e2390db8379cd9e1e2b22828e55b45b57c8559e4c8335678c717dc245c18176"),
    size_hint: Some(397_987),
};

/// 고속(기본) 분리: UVR MDX-Net Voc_FT (보컬 전용).
pub const MDX_VOC_FT: RemoteModel = RemoteModel {
    file_name: "UVR-MDX-NET-Voc_FT.onnx",
    url: "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/UVR-MDX-NET-Voc_FT.onnx",
    sha256: Some("534b2070fcc7df514b13ef660dc8cbb328679c2374d04354a5c42bb14ecce111"),
    size_hint: Some(66_800_000),
};

/// 품질 분리: HTDemucs FT vocals 스페셜리스트 ONNX (MIT).
/// 공개 체크섬 미확인 → TOFU.
pub const HTDEMUCS_VOCALS: RemoteModel = RemoteModel {
    file_name: "htdemucs_ft_vocals.onnx",
    url: "https://huggingface.co/StemSplitio/htdemucs-ft-vocals-onnx/resolve/main/htdemucs_ft_vocals.onnx",
    sha256: None,
    size_hint: Some(316_000_000),
};

/// 진행 콜백: (받은 바이트, 전체 바이트 추정).
pub type Progress<'a> = &'a mut dyn FnMut(u64, Option<u64>);

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_of_file(path: &Path) -> Result<String, ReferenceError> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

fn expected_sha(dir: &Path, model: &RemoteModel) -> Result<Option<String>, ReferenceError> {
    if let Some(s) = model.sha256 {
        return Ok(Some(s.to_lowercase()));
    }
    let sidecar = dir.join(format!("{}.sha256", model.file_name));
    if sidecar.exists() {
        let s = std::fs::read_to_string(&sidecar)?;
        return Ok(Some(s.trim().to_lowercase()));
    }
    Ok(None)
}

/// 모델 파일을 보장한다: 있으면 검증 후 반환, 없으면 (이어)다운로드.
pub fn ensure_model(
    dir: &Path,
    model: &RemoteModel,
    progress: Progress<'_>,
) -> Result<PathBuf, ReferenceError> {
    std::fs::create_dir_all(dir)?;
    let target = dir.join(model.file_name);
    let expected = expected_sha(dir, model)?;

    if target.exists() {
        match &expected {
            Some(exp) => {
                let actual = sha256_of_file(&target)?;
                if &actual == exp {
                    return Ok(target);
                }
                eprintln!(
                    "[download] 체크섬 불일치, 재다운로드: {} (expected {exp}, got {actual})",
                    model.file_name
                );
                std::fs::remove_file(&target)?;
            }
            None => return Ok(target),
        }
    }

    let part = dir.join(format!("{}.part", model.file_name));
    let mut offset = part.metadata().map(|m| m.len()).unwrap_or(0);

    let mut request = ureq::get(model.url);
    if offset > 0 {
        request = request.header("Range", format!("bytes={offset}-"));
    }
    let response = request
        .call()
        .map_err(|e| ReferenceError::Download(format!("{}: {e}", model.url)))?;

    let status = response.status().as_u16();
    let mut file = match status {
        206 => std::fs::OpenOptions::new().append(true).create(true).open(&part)?,
        200 => {
            // 서버가 Range를 무시 → 처음부터.
            offset = 0;
            std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&part)?
        }
        s => {
            return Err(ReferenceError::Download(format!(
                "{}: HTTP {s}",
                model.file_name
            )))
        }
    };

    let content_len: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let total = content_len.map(|c| c + offset).or(model.size_hint);

    let mut reader = response.into_body().into_reader();
    let mut buf = [0u8; 65536];
    let mut received = offset;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| ReferenceError::Download(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        received += n as u64;
        progress(received, total);
    }
    file.flush()?;
    file.sync_data()?;
    drop(file);

    let actual = sha256_of_file(&part)?;
    match &expected {
        Some(exp) => {
            if &actual != exp {
                std::fs::remove_file(&part)?;
                return Err(ReferenceError::Download(format!(
                    "{}: 체크섬 불일치 (expected {exp}, got {actual}) — 파일 폐기, 다시 시도하세요",
                    model.file_name
                )));
            }
        }
        None => {
            // TOFU: 최초 해시 박제.
            std::fs::write(dir.join(format!("{}.sha256", model.file_name)), &actual)?;
            eprintln!("[download] TOFU 체크섬 기록: {} = {actual}", model.file_name);
        }
    }
    std::fs::rename(&part, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::net::TcpListener;

    /// Range를 지원하는 초소형 테스트 HTTP 서버 (단일 스레드, N회 응답).
    fn serve(data: Vec<u8>, requests: usize, honor_range: bool) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut range: Option<u64> = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let l = line.trim().to_lowercase();
                    if let Some(r) = l.strip_prefix("range: bytes=") {
                        range = r.split('-').next().and_then(|v| v.parse().ok());
                    }
                    if line == "\r\n" || line == "\n" {
                        break;
                    }
                }
                match range.filter(|_| honor_range) {
                    Some(from) => {
                        let body = &data[(from as usize).min(data.len())..];
                        let head = format!(
                            "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {}-{}/{}\r\nconnection: close\r\n\r\n",
                            body.len(), from, data.len().saturating_sub(1), data.len()
                        );
                        stream.write_all(head.as_bytes()).unwrap();
                        stream.write_all(body).unwrap();
                    }
                    None => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            data.len()
                        );
                        stream.write_all(head.as_bytes()).unwrap();
                        stream.write_all(&data).unwrap();
                    }
                }
            }
        });
        format!("http://{addr}/model.bin")
    }

    fn test_model(url: &str, sha: Option<&'static str>) -> RemoteModel {
        // 테스트 전용: url을 leak해 'static 확보.
        RemoteModel {
            file_name: "test_model.bin",
            url: Box::leak(url.to_string().into_boxed_str()),
            sha256: sha,
            size_hint: None,
        }
    }

    fn hex_sha(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        to_hex(&h.finalize())
    }

    #[test]
    fn fresh_download_verifies_and_renames() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let sha = hex_sha(&data);
        let url = serve(data.clone(), 1, true);
        let model = test_model(&url, Some(Box::leak(sha.into_boxed_str())));
        let dir = tempfile::tempdir().unwrap();
        let mut seen = Vec::new();
        let path = ensure_model(dir.path(), &model, &mut |r, t| seen.push((r, t))).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert!(!dir.path().join("test_model.bin.part").exists());
        assert_eq!(seen.last().unwrap().0, 100_000);
        assert_eq!(seen.last().unwrap().1, Some(100_000));
    }

    #[test]
    fn resume_appends_remaining_bytes() {
        let data: Vec<u8> = (0..80_000u32).map(|i| (i % 37) as u8).collect();
        let sha = hex_sha(&data);
        let url = serve(data.clone(), 1, true);
        let model = test_model(&url, Some(Box::leak(sha.into_boxed_str())));
        let dir = tempfile::tempdir().unwrap();
        // 앞 30k가 이미 받아졌다고 가정.
        std::fs::write(dir.path().join("test_model.bin.part"), &data[..30_000]).unwrap();
        let mut max_seen = 0;
        let path = ensure_model(dir.path(), &model, &mut |r, _| max_seen = r).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert_eq!(max_seen, 80_000);
    }

    #[test]
    fn range_ignoring_server_restarts_from_zero() {
        let data: Vec<u8> = (0..50_000u32).map(|i| (i % 13) as u8).collect();
        let sha = hex_sha(&data);
        let url = serve(data.clone(), 1, false); // 200만 반환
        let model = test_model(&url, Some(Box::leak(sha.into_boxed_str())));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test_model.bin.part"), vec![9u8; 10_000]).unwrap();
        let path = ensure_model(dir.path(), &model, &mut |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
    }

    #[test]
    fn checksum_mismatch_discards_part() {
        let data = vec![1u8; 20_000];
        let url = serve(data, 1, true);
        let model = test_model(&url, Some("deadbeef"));
        let dir = tempfile::tempdir().unwrap();
        let err = ensure_model(dir.path(), &model, &mut |_, _| {}).unwrap_err();
        assert!(err.to_string().contains("체크섬"), "{err}");
        assert!(!dir.path().join("test_model.bin.part").exists());
        assert!(!dir.path().join("test_model.bin").exists());
    }

    #[test]
    fn tofu_records_then_verifies() {
        let data = vec![7u8; 10_000];
        let url = serve(data.clone(), 1, true);
        let model = test_model(&url, None);
        let dir = tempfile::tempdir().unwrap();
        let path = ensure_model(dir.path(), &model, &mut |_, _| {}).unwrap();
        let sidecar = dir.path().join("test_model.bin.sha256");
        assert!(sidecar.exists());
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap().trim(), hex_sha(&data));
        // 이미 존재 + 사이드카 일치 → 네트워크 없이 즉시 반환.
        let again = ensure_model(dir.path(), &model, &mut |_, _| {}).unwrap();
        assert_eq!(again, path);
    }

    #[test]
    fn existing_valid_file_short_circuits() {
        let data = vec![3u8; 5_000];
        let sha = hex_sha(&data);
        let model = test_model("http://127.0.0.1:1/unreachable", Some(Box::leak(sha.into_boxed_str())));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test_model.bin"), &data).unwrap();
        // 네트워크에 닿지 않고 성공해야 한다.
        let path = ensure_model(dir.path(), &model, &mut |_, _| {}).unwrap();
        assert!(path.ends_with("test_model.bin"));
    }
}
