//! 세션 영속화 (스펙 §5).
//!
//! - 쓰기 연결은 스토리지 스레드가 단독 소유 (mpsc 수신).
//! - 라이브 중에는 `.f0raw` 저널(F0C1 저널 모드)에 추가 + 주기 fsync,
//!   세션 종료 시 요약 통계·preview 계산 후 F0C1+zstd BLOB을 트랜잭션 커밋.
//! - 시작 시 고아 저널/깨진 WAV 헤더 복구.
//! - 조회는 읽기 전용 연결로 커맨드 스레드에서 직접 (WAL 동시 읽기).

use std::path::{Path, PathBuf};

use thiserror::Error;

pub mod db;
pub mod journal;
pub mod queries;
pub mod recording;
pub mod stats;
pub mod thread;

pub use thread::{FinalizedSession, RecordingSpec, StorageHandle, StorageMsg};

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("codec: {0}")]
    Codec(#[from] vocalboard_codec::CodecError),
    #[error("wav: {0}")]
    Wav(#[from] hound::Error),
    #[error("{0}")]
    Other(String),
}

/// app_data 하위 파일 배치 (§5): DB `db/app.sqlite3`, 녹음 `recordings/`,
/// 스템 `tracks/{id}/`. 저널(.f0raw)은 db/ 옆에 둔다.
#[derive(Debug, Clone)]
pub struct StorageRoot {
    pub app_data: PathBuf,
}

impl StorageRoot {
    pub fn new(app_data: impl Into<PathBuf>) -> Self {
        Self { app_data: app_data.into() }
    }

    pub fn db_dir(&self) -> PathBuf {
        self.app_data.join("db")
    }

    pub fn db_path(&self) -> PathBuf {
        self.db_dir().join("app.sqlite3")
    }

    pub fn journal_path(&self, session_id: &str) -> PathBuf {
        self.db_dir().join(format!("{session_id}.f0raw"))
    }

    pub fn recordings_dir(&self) -> PathBuf {
        self.app_data.join("recordings")
    }

    pub fn recording_path(&self, session_id: &str) -> PathBuf {
        self.recordings_dir().join(format!("{session_id}.wav"))
    }

    pub fn tracks_dir(&self) -> PathBuf {
        self.app_data.join("tracks")
    }

    pub fn track_dir(&self, track_id: &str) -> PathBuf {
        self.tracks_dir().join(track_id)
    }

    pub fn models_dir(&self) -> PathBuf {
        self.app_data.join("models")
    }

    pub fn ensure_dirs(&self) -> Result<(), StorageError> {
        for d in [self.db_dir(), self.recordings_dir(), self.tracks_dir(), self.models_dir()] {
            std::fs::create_dir_all(&d)?;
        }
        Ok(())
    }
}

/// 경로를 DB 저장용 상대 문자열로 (app_data 기준, 슬래시 통일).
pub fn to_rel_path(root: &StorageRoot, p: &Path) -> String {
    p.strip_prefix(&root.app_data)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}
