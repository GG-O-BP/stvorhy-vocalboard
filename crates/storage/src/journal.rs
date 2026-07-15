//! `.f0raw` 저널: 크래시 대비 라이브 프레임 기록 + 시작 시 고아 복구.
//!
//! 파일 포맷 = F0C1 저널 모드 (frame_count=UNKNOWN, 꼬리 잘림 허용).
//! 파일명 `<session_id>.f0raw`. 주기 fsync(기본 1초)로 유실 창을 제한한다.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vocalboard_codec::{
    decode, encode_frame, encode_header, F0Frame, F0c1Header, FRAME_COUNT_UNKNOWN,
};

use crate::StorageError;

pub const SYNC_INTERVAL: Duration = Duration::from_secs(1);

pub struct JournalWriter {
    out: BufWriter<File>,
    path: PathBuf,
    last_sync: Instant,
}

impl JournalWriter {
    pub fn create(path: PathBuf, started_at_ms: i64) -> Result<Self, StorageError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().write(true).create(true).truncate(true).open(&path)?;
        let mut out = BufWriter::new(file);
        let header = F0c1Header {
            frame_count: FRAME_COUNT_UNKNOWN,
            ..F0c1Header::new(started_at_ms, 0)
        };
        out.write_all(&encode_header(&header))?;
        out.flush()?;
        out.get_ref().sync_data()?;
        Ok(Self { out, path, last_sync: Instant::now() })
    }

    pub fn append(&mut self, frame: &F0Frame) -> Result<(), StorageError> {
        self.out.write_all(&encode_frame(frame))?;
        if self.last_sync.elapsed() >= SYNC_INTERVAL {
            self.sync()?;
        }
        Ok(())
    }

    pub fn sync(&mut self) -> Result<(), StorageError> {
        self.out.flush()?;
        self.out.get_ref().sync_data()?;
        self.last_sync = Instant::now();
        Ok(())
    }

    /// 정상 종료: 저널 삭제 (데이터는 DB 트랜잭션으로 넘어간 뒤 호출).
    pub fn remove(self) -> Result<(), StorageError> {
        let path = self.path.clone();
        drop(self);
        std::fs::remove_file(&path)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// 고아 저널 하나를 읽는다. (시작 시각, 프레임들) 반환.
pub fn read_journal(path: &Path) -> Result<(i64, Vec<F0Frame>), StorageError> {
    let bytes = std::fs::read(path)?;
    let (header, frames) = decode(&bytes)?;
    Ok((header.start_unix_ms, frames))
}

/// db 디렉토리에서 고아 저널 경로들을 나열한다.
pub fn list_orphans(db_dir: &Path) -> Result<Vec<PathBuf>, StorageError> {
    let mut out = Vec::new();
    if !db_dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(db_dir)? {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "f0raw") {
            out.push(p);
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(midi: f32) -> F0Frame {
        F0Frame::quantize(midi, true, 0.95, -20.0)
    }

    #[test]
    fn journal_roundtrip_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.f0raw");
        let mut w = JournalWriter::create(path.clone(), 1234).unwrap();
        for i in 0..100 {
            w.append(&frame(50.0 + i as f32 * 0.1)).unwrap();
        }
        w.sync().unwrap();
        let (start, frames) = read_journal(&path).unwrap();
        assert_eq!(start, 1234);
        assert_eq!(frames.len(), 100);
        assert_eq!(frames[10], frame(51.0));
        w.remove().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn truncated_tail_is_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.f0raw");
        let mut w = JournalWriter::create(path.clone(), 7).unwrap();
        for _ in 0..10 {
            w.append(&frame(60.0)).unwrap();
        }
        w.sync().unwrap();
        drop(w);
        // 크래시로 마지막 프레임이 반쯤 쓰였다고 가정.
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 3]).unwrap();
        let (_, frames) = read_journal(&path).unwrap();
        assert_eq!(frames.len(), 9);
    }

    #[test]
    fn list_orphans_finds_only_f0raw() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.f0raw"), b"x").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"x").unwrap();
        let orphans = list_orphans(dir.path()).unwrap();
        assert_eq!(orphans.len(), 1);
    }
}
