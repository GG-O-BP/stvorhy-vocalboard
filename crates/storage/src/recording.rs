//! 세션 녹음 (§3: hound 증분 WAV, 마이크 드라이만) + 크래시 헤더 복구 +
//! 보존 정책 (최근 N개).

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hound::{SampleFormat, WavSpec, WavWriter};
use rusqlite::Connection;

use crate::StorageError;

/// 주기 flush 간격: 크래시 시 유실 창 상한 (저널 fsync와 동일 철학).
/// hound의 flush는 헤더 길이도 갱신하므로 flush 시점까지는 유효한 WAV다.
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// 드라이 mono 입력을 16-bit PCM으로 증분 기록한다.
pub struct WavRecorder {
    writer: WavWriter<std::io::BufWriter<std::fs::File>>,
    path: PathBuf,
    last_flush: Instant,
}

impl WavRecorder {
    pub fn create(path: PathBuf, sample_rate: u32) -> Result<Self, StorageError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        Ok(Self {
            writer: WavWriter::create(&path, spec)?,
            path,
            last_flush: Instant::now(),
        })
    }

    pub fn write(&mut self, mono: &[f32]) -> Result<(), StorageError> {
        for s in mono {
            let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            self.writer.write_sample(v)?;
        }
        if self.last_flush.elapsed() >= FLUSH_INTERVAL {
            self.flush()?;
        }
        Ok(())
    }

    /// 버퍼를 내리고 헤더 길이를 현재까지로 갱신한다.
    pub fn flush(&mut self) -> Result<(), StorageError> {
        self.writer.flush()?;
        self.last_flush = Instant::now();
        Ok(())
    }

    /// 헤더 길이를 확정하고 닫는다.
    pub fn finalize(self) -> Result<PathBuf, StorageError> {
        let path = self.path.clone();
        self.writer.finalize()?;
        Ok(path)
    }

    /// 세션 폐기: 파일 삭제.
    pub fn discard(self) -> Result<(), StorageError> {
        let path = self.path.clone();
        drop(self.writer); // finalize 없이 닫힘 → 파일 삭제하므로 무관
        std::fs::remove_file(&path)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// 크래시로 RIFF/data 청크 길이가 확정되지 않은 WAV의 헤더를 복구한다.
///
/// 반환: 복구 수행 여부. 헤더 길이가 이미 파일 크기와 일치하면 false.
/// RIFF 컨테이너를 순회해 data 청크를 찾고, 실제 파일 길이로 RIFF/data
/// 크기 필드를 다시 쓴다 (뒤따르는 청크가 없다는 전제 — hound 출력 형태).
pub fn repair_wav_header(path: &Path) -> Result<bool, StorageError> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let file_len = file.metadata()?.len();
    if file_len < 44 {
        return Err(StorageError::Other(format!(
            "{}: WAV라기엔 너무 작음 ({file_len}B)",
            path.display()
        )));
    }
    let mut head = [0u8; 12];
    file.read_exact(&mut head)?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return Err(StorageError::Other(format!("{}: RIFF/WAVE 아님", path.display())));
    }

    // 청크 순회로 data 청크 위치를 찾는다.
    let mut pos: u64 = 12;
    let data_pos = loop {
        if pos + 8 > file_len {
            return Err(StorageError::Other(format!("{}: data 청크 없음", path.display())));
        }
        file.seek(SeekFrom::Start(pos))?;
        let mut chunk_head = [0u8; 8];
        file.read_exact(&mut chunk_head)?;
        let size = u32::from_le_bytes(chunk_head[4..8].try_into().unwrap()) as u64;
        if &chunk_head[0..4] == b"data" {
            break pos;
        }
        // 크기 필드가 깨졌으면(파일 끝 초과) 더 진행할 수 없다.
        let next = pos + 8 + size + (size & 1);
        if size == 0 || next > file_len {
            return Err(StorageError::Other(format!(
                "{}: 손상된 중간 청크 (pos {pos})",
                path.display()
            )));
        }
        pos = next;
    };

    let true_data_len = file_len - (data_pos + 8);
    let true_riff_len = file_len - 8;

    // 현재 기록된 값과 비교.
    file.seek(SeekFrom::Start(4))?;
    let mut b4 = [0u8; 4];
    file.read_exact(&mut b4)?;
    let riff_len = u32::from_le_bytes(b4) as u64;
    file.seek(SeekFrom::Start(data_pos + 4))?;
    file.read_exact(&mut b4)?;
    let data_len = u32::from_le_bytes(b4) as u64;

    if riff_len == true_riff_len && data_len == true_data_len {
        return Ok(false);
    }

    file.seek(SeekFrom::Start(4))?;
    file.write_all(&(true_riff_len as u32).to_le_bytes())?;
    file.seek(SeekFrom::Start(data_pos + 4))?;
    file.write_all(&(true_data_len as u32).to_le_bytes())?;
    file.sync_data()?;
    Ok(true)
}

/// recordings 디렉토리의 모든 WAV에 대해 복구 패스를 돌린다 (시작 시).
pub fn repair_all(recordings_dir: &Path) -> Result<Vec<PathBuf>, StorageError> {
    let mut repaired = Vec::new();
    if !recordings_dir.exists() {
        return Ok(repaired);
    }
    for entry in std::fs::read_dir(recordings_dir)? {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "wav") {
            match repair_wav_header(&p) {
                Ok(true) => repaired.push(p),
                Ok(false) => {}
                Err(e) => eprintln!("[storage] WAV 복구 실패 {}: {e}", p.display()),
            }
        }
    }
    Ok(repaired)
}

/// 보존 정책: recording_path가 있는 세션 중 최신 keep_last개만 남기고
/// 파일 삭제 + recording_path NULL. 삭제된 세션 id 목록 반환.
pub fn enforce_retention(
    conn: &Connection,
    app_data: &Path,
    keep_last: u32,
) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, recording_path FROM sessions
         WHERE recording_path IS NOT NULL
         ORDER BY started_at DESC
         LIMIT -1 OFFSET ?1",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map([keep_last], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut removed = Vec::new();
    for (id, rel) in rows {
        let path = app_data.join(&rel);
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                eprintln!("[storage] 녹음 삭제 실패 {}: {e}", path.display());
                continue;
            }
        }
        conn.execute("UPDATE sessions SET recording_path = NULL WHERE id = ?1", [&id])?;
        removed.push(id);
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_writes_readable_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.wav");
        let mut rec = WavRecorder::create(path.clone(), 48_000).unwrap();
        let chunk: Vec<f32> = (0..4800).map(|i| (i as f32 / 100.0).sin() * 0.5).collect();
        rec.write(&chunk).unwrap();
        rec.write(&chunk).unwrap();
        rec.finalize().unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.len(), 9600);
    }

    /// 크래시 시나리오: 길이 필드가 0인 헤더 + 유효한 PCM 꼬리.
    #[test]
    fn repairs_crashed_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.wav");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 미확정 RIFF 크기
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&(48_000u32 * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 미확정 data 크기
        for i in 0..1000i16 {
            bytes.extend_from_slice(&i.to_le_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();

        assert!(hound::WavReader::open(&path).is_err() || {
            // 일부 리더는 0길이로 열리므로 샘플 수 0으로 취급된다.
            hound::WavReader::open(&path).unwrap().len() == 0
        });

        assert!(repair_wav_header(&path).unwrap());
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.len(), 1000);
        let samples: Vec<i16> = reader.into_samples().map(|s| s.unwrap()).collect();
        assert_eq!(samples[999], 999);

        // 두 번째 패스는 no-op.
        assert!(!repair_wav_header(&path).unwrap());
    }

    #[test]
    fn healthy_wav_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ok.wav");
        let mut rec = WavRecorder::create(path.clone(), 16_000).unwrap();
        rec.write(&vec![0.1; 1600]).unwrap();
        rec.finalize().unwrap();
        assert!(!repair_wav_header(&path).unwrap());
    }

    #[test]
    fn retention_deletes_oldest_beyond_n() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::StorageRoot::new(dir.path());
        root.ensure_dirs().unwrap();
        let conn = crate::db::open_write(&root.db_path()).unwrap();
        for i in 0..5 {
            let id = format!("s{i}");
            let wav = root.recording_path(&id);
            std::fs::write(&wav, b"RIFFxxxxWAVE").unwrap();
            conn.execute(
                "INSERT INTO sessions (id, started_at, recording_path) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, 1000 + i, crate::to_rel_path(&root, &wav)],
            )
            .unwrap();
        }
        let removed = enforce_retention(&conn, &root.app_data, 2).unwrap();
        assert_eq!(removed.len(), 3);
        // 최신 2개(s4, s3)만 파일과 경로가 남는다.
        assert!(root.recording_path("s4").exists());
        assert!(root.recording_path("s3").exists());
        assert!(!root.recording_path("s2").exists());
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE recording_path IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
    }
}
