//! 曲库与任务队列（ADR-0009）
//!
//! 三件事挂在这里：按曲目 ID 去重、断点续传、列表中显示「已下载」标记。
//!
//! 关键约束：**索引会与磁盘不同步**——用户可能在 Finder 里删掉或移走了文件。
//! 所以「已下载」的判定不能只信索引，必须落到 `is_downloaded` 里做存在性校验。

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackRecord {
    pub track_id: i64,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub file_path: String,
    pub file_size: i64,
    pub bitrate: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub track_id: i64,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub status: String,
    pub reason: Option<String>,
    /// 封面地址随队列一起持久化。
    /// 否则下载时要么再查一次接口（多一次请求，与 ADR-0005 的节流意图相悖），
    /// 要么干脆不写封面（ADR-0003 明确不接受）。
    #[serde(default)]
    pub cover_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueStats {
    pub pending: i64,
    pub done: i64,
    pub failed: i64,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS tracks (
                track_id   INTEGER PRIMARY KEY,
                title      TEXT    NOT NULL,
                artists    TEXT    NOT NULL,
                album      TEXT    NOT NULL DEFAULT '',
                file_path  TEXT    NOT NULL,
                file_size  INTEGER NOT NULL DEFAULT 0,
                bitrate    INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS queue (
                track_id    INTEGER PRIMARY KEY,
                title       TEXT    NOT NULL,
                artists     TEXT    NOT NULL,
                album       TEXT    NOT NULL DEFAULT '',
                status      TEXT    NOT NULL,
                reason      TEXT,
                enqueued_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_queue_status ON queue(status);
            "#,
        )?;

        // 迁移：早期版本的 queue 表没有 cover_url。
        // 列已存在时 ALTER 会失败，这里正是预期情况，忽略即可。
        let _ = conn.execute(
            "ALTER TABLE queue ADD COLUMN cover_url TEXT NOT NULL DEFAULT ''",
            [],
        );

        Ok(Self { conn })
    }

    /// 返回已下载文件的路径。
    ///
    /// **索引说有还不够** —— 文件可能已被用户删掉。此时清理掉过期记录并返回 None，
    /// 让它重新进入下载队列。校验放在这里（判定时），而非启动时全量扫盘：
    /// 上千首的全量校验会明显拖慢启动。
    pub fn is_downloaded(&self, track_id: i64) -> Result<Option<String>> {
        let path: Option<String> = self
            .conn
            .query_row(
                "SELECT file_path FROM tracks WHERE track_id = ?1",
                params![track_id],
                |r| r.get(0),
            )
            .optional()?;

        match path {
            Some(p) if Path::new(&p).exists() => Ok(Some(p)),
            Some(_) => {
                self.conn
                    .execute("DELETE FROM tracks WHERE track_id = ?1", params![track_id])?;
                Ok(None)
            }
            None => Ok(None),
        }
    }

    pub fn record_track(&self, r: &TrackRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO tracks (track_id, title, artists, album, file_path, file_size, bitrate, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(track_id) DO UPDATE SET
                file_path=excluded.file_path, file_size=excluded.file_size,
                bitrate=excluded.bitrate, created_at=excluded.created_at",
            params![r.track_id, r.title, r.artists, r.album, r.file_path, r.file_size, r.bitrate, r.created_at],
        )?;
        Ok(())
    }

    /// 入队。已在队列中的曲目保持原状态不覆盖，避免重复入队时把已完成的重置成待下。
    pub fn enqueue(&mut self, items: &[QueueItem]) -> Result<usize> {
        let now = now_ts();
        let tx = self.conn.transaction()?;
        let mut added = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO queue (track_id, title, artists, album, status, reason, enqueued_at, cover_url)
                 VALUES (?1,?2,?3,?4,'pending',NULL,?5,?6)
                 ON CONFLICT(track_id) DO NOTHING",
            )?;
            for it in items {
                added += stmt.execute(params![
                    it.track_id, it.title, it.artists, it.album, now, it.cover_url
                ])?;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    pub fn next_pending(&self) -> Result<Option<QueueItem>> {
        Ok(self
            .conn
            .query_row(
                "SELECT track_id, title, artists, album, status, reason, cover_url
                 FROM queue WHERE status = 'pending' ORDER BY enqueued_at, track_id LIMIT 1",
                [],
                row_to_queue_item,
            )
            .optional()?)
    }

    pub fn mark_done(&self, track_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE queue SET status='done', reason=NULL WHERE track_id=?1",
            params![track_id],
        )?;
        Ok(())
    }

    pub fn mark_failed(&self, track_id: i64, reason: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE queue SET status='failed', reason=?2 WHERE track_id=?1",
            params![track_id, reason],
        )?;
        Ok(())
    }

    pub fn stats(&self) -> Result<QueueStats> {
        let mut s = QueueStats::default();
        let mut stmt = self
            .conn
            .prepare("SELECT status, COUNT(*) FROM queue GROUP BY status")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (status, n) = row?;
            match status.as_str() {
                "pending" => s.pending = n,
                "done" => s.done = n,
                "failed" => s.failed = n,
                _ => {}
            }
        }
        Ok(s)
    }

    /// 失败清单。ADR-0009 的用户可见行为：队列不中断，跑完统一给清单。
    pub fn failed_items(&self) -> Result<Vec<QueueItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT track_id, title, artists, album, status, reason, cover_url
             FROM queue WHERE status='failed' ORDER BY enqueued_at",
        )?;
        let rows = stmt.query_map([], row_to_queue_item)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn library(&self, limit: i64) -> Result<Vec<TrackRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT track_id, title, artists, album, file_path, file_size, bitrate, created_at
             FROM tracks ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(TrackRecord {
                track_id: r.get(0)?,
                title: r.get(1)?,
                artists: r.get(2)?,
                album: r.get(3)?,
                file_path: r.get(4)?,
                file_size: r.get(5)?,
                bitrate: r.get(6)?,
                created_at: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// 清空已完成与失败的记录，保留 pending。用于「开始新一批」。
    pub fn clear_finished(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM queue WHERE status IN ('done','failed')", [])?;
        Ok(())
    }
}

fn row_to_queue_item(r: &rusqlite::Row) -> rusqlite::Result<QueueItem> {
    Ok(QueueItem {
        track_id: r.get(0)?,
        title: r.get(1)?,
        artists: r.get(2)?,
        album: r.get(3)?,
        status: r.get(4)?,
        reason: r.get(5)?,
        cover_url: r.get(6).unwrap_or_default(),
    })
}

pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 索引说有、磁盘上没有 → 必须判定为未下载，并清理掉过期记录。
    /// 这是 ADR-0009 里明确要求的行为。
    #[test]
    fn is_downloaded_rejects_missing_file() {
        let dir = std::env::temp_dir().join(format!("ncmdb-{}", now_ts()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("t.db")).unwrap();

        let ghost = dir.join("不存在.mp3");
        db.record_track(&TrackRecord {
            track_id: 1,
            title: "t".into(),
            artists: "a".into(),
            album: String::new(),
            file_path: ghost.to_string_lossy().into(),
            file_size: 1,
            bitrate: 320000,
            created_at: now_ts(),
        })
        .unwrap();

        assert_eq!(db.is_downloaded(1).unwrap(), None, "文件不存在时不能算已下载");
        assert_eq!(db.is_downloaded(1).unwrap(), None, "过期记录应已被清理");

        let real = dir.join("存在.mp3");
        std::fs::write(&real, b"x").unwrap();
        db.record_track(&TrackRecord {
            track_id: 2,
            title: "t".into(),
            artists: "a".into(),
            album: String::new(),
            file_path: real.to_string_lossy().into(),
            file_size: 1,
            bitrate: 320000,
            created_at: now_ts(),
        })
        .unwrap();
        assert!(db.is_downloaded(2).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 重复入队不能把已完成的曲目重置为待下载。
    #[test]
    fn enqueue_does_not_reset_finished_items() {
        let dir = std::env::temp_dir().join(format!("ncmdb2-{}", now_ts()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = Db::open(&dir.join("t.db")).unwrap();

        let item = QueueItem {
            track_id: 7,
            title: "t".into(),
            artists: "a".into(),
            album: String::new(),
            status: "pending".into(),
            reason: None,
            cover_url: String::new(),
        };
        assert_eq!(db.enqueue(&[item.clone()]).unwrap(), 1);
        db.mark_done(7).unwrap();
        assert_eq!(db.enqueue(&[item]).unwrap(), 0, "已存在的不该重复插入");
        assert_eq!(db.stats().unwrap().done, 1);
        assert_eq!(db.stats().unwrap().pending, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
