//! 串行下载器（ADR-0003 / 0005 / 0008 / 0009）
//!
//! 核心行为，每一条都对应一份 ADR：
//!   - 串行 + 随机间隔，绝不并发（ADR-0005，账号风控）
//!   - 逐首现取直链，绝不批量预取（ADR-0005，直链会过期）
//!   - 下载即完成，无转码阶段（ADR-0003，320k 直出 mp3）
//!   - 失败不中断队列，跑完给清单（ADR-0009）
//!   - 瞬时失败重试，终局失败直接跳过（ADR-0009）

use crate::client::NcmClient;
use crate::db::{Db, QueueItem, TrackRecord};
use crate::naming;
use anyhow::{anyhow, Result};
use rand::Rng;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// 节流参数。ADR-0005 明确要求可配置而非硬编码——
/// 「多保守才算保守」无法先验确定，只能实际跑过再调。
#[derive(Debug, Clone)]
pub struct Throttle {
    pub min_delay_ms: u64,
    pub max_delay_ms: u64,
    pub max_retries: u32,
}

impl Default for Throttle {
    fn default() -> Self {
        Self {
            min_delay_ms: 900,
            max_delay_ms: 2600,
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub current: Option<String>,
    pub done: i64,
    pub failed: i64,
    pub pending: i64,
    pub running: bool,
    pub last_message: String,
}

/// 失败分类决定重试语义——两者混为一谈，一次 WiFi 抖动就能让几十首歌
/// 无谓地进入失败清单（ADR-0009）。
enum Failure {
    /// 网络抖动、限流、直链过期 —— 重试有意义
    Transient(String),
    /// 无版权 / 需单独购买 / 地区限制 —— 重试无意义
    Terminal(String),
}

fn classify(err: &anyhow::Error) -> Failure {
    let msg = err.to_string();
    if msg.contains("无下载权限") || msg.contains("song/url 无数据") {
        Failure::Terminal("无下载权限（无版权/需单独购买/地区限制）".into())
    } else {
        Failure::Transient(msg)
    }
}

pub struct Downloader {
    pub stop: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
}

impl Downloader {
    pub fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// 跑完整个队列。每首完成后立刻落库——
/// 这样即便进程被强杀，最坏也只丢失当前这一首（ADR-0009）。
pub async fn run_queue(
    client: Arc<Mutex<NcmClient>>,
    db: Arc<Mutex<Db>>,
    out_dir: PathBuf,
    throttle: Throttle,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    on_progress: impl Fn(Progress) + Send + 'static,
) {
    running.store(true, Ordering::SeqCst);
    stop.store(false, Ordering::SeqCst);
    std::fs::create_dir_all(&out_dir).ok();

    loop {
        if stop.load(Ordering::SeqCst) {
            emit(&db, &on_progress, None, "已暂停", false).await;
            break;
        }

        let item = {
            let d = db.lock().await;
            match d.next_pending() {
                Ok(Some(i)) => i,
                Ok(None) => {
                    drop(d);
                    emit(&db, &on_progress, None, "全部完成", false).await;
                    break;
                }
                Err(e) => {
                    drop(d);
                    emit(&db, &on_progress, None, &format!("读取队列失败: {e}"), false).await;
                    break;
                }
            }
        };

        let label = format!("{} - {}", item.artists, item.title);
        emit(&db, &on_progress, Some(label.clone()), "下载中", true).await;

        // 已经下过就跳过。注意 is_downloaded 内部会校验文件真的还在磁盘上。
        let already = {
            let d = db.lock().await;
            d.is_downloaded(item.track_id).unwrap_or(None)
        };
        if already.is_some() {
            let d = db.lock().await;
            let _ = d.mark_done(item.track_id);
            drop(d);
            emit(&db, &on_progress, Some(label), "已存在，跳过", true).await;
            continue;
        }

        match download_one(&client, &item, &out_dir, &throttle, &stop).await {
            Ok(rec) => {
                let d = db.lock().await;
                let _ = d.record_track(&rec);
                let _ = d.mark_done(item.track_id);
                drop(d);
                emit(&db, &on_progress, Some(label), "完成", true).await;
            }
            Err(e) => {
                let reason = match classify(&e) {
                    Failure::Terminal(r) => r,
                    Failure::Transient(r) => format!("网络或服务端问题：{r}"),
                };
                let d = db.lock().await;
                let _ = d.mark_failed(item.track_id, &reason);
                drop(d);
                emit(&db, &on_progress, Some(label), &reason, true).await;
            }
        }

        // 随机间隔（ADR-0005）
        if !stop.load(Ordering::SeqCst) {
            let ms = rand::thread_rng().gen_range(throttle.min_delay_ms..=throttle.max_delay_ms);
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
    }

    running.store(false, Ordering::SeqCst);
}

async fn download_one(
    client: &Arc<Mutex<NcmClient>>,
    item: &QueueItem,
    out_dir: &Path,
    throttle: &Throttle,
    stop: &Arc<AtomicBool>,
) -> Result<TrackRecord> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match try_download(client, item, out_dir).await {
            Ok(rec) => return Ok(rec),
            Err(e) => match classify(&e) {
                // 终局失败重试没有意义，立刻返回
                Failure::Terminal(_) => return Err(e),
                Failure::Transient(_) if attempt >= throttle.max_retries => return Err(e),
                Failure::Transient(_) => {
                    if stop.load(Ordering::SeqCst) {
                        return Err(e);
                    }
                    // 指数退避
                    tokio::time::sleep(Duration::from_millis(700 * 2u64.pow(attempt))).await;
                }
            },
        }
    }
}

async fn try_download(
    client: &Arc<Mutex<NcmClient>>,
    item: &QueueItem,
    out_dir: &Path,
) -> Result<TrackRecord> {
    // 逐首现取。ADR-0005 硬约束：不可改成批量预取，直链会过期。
    let song = {
        let mut c = client.lock().await;
        c.song_url(item.track_id, crate::client::TARGET_BITRATE).await?
    };

    let bytes = {
        let c = client.lock().await;
        let http = c.http().clone();
        drop(c);
        http.get(&song.url)
            .header("Referer", "https://music.163.com")
            .send()
            .await?
            .bytes()
            .await?
    };

    if bytes.len() < 1024 {
        return Err(anyhow!("下载内容异常（仅 {} 字节）", bytes.len()));
    }

    // 接口声称的字节数是校验完整性的现成依据——spike 中实测两者一致。
    // 截断的下载在文件头上看不出任何异常，只有比对长度才能发现。
    // 归类为瞬时失败，会被重试。
    if song.size > 0 && bytes.len() as i64 != song.size {
        return Err(anyhow!(
            "下载不完整：收到 {} 字节，接口声称 {} 字节",
            bytes.len(),
            song.size
        ));
    }

    // ADR-0003：零转码。落盘扩展名以实际内容为准，绝不撒谎——
    // 原实现在缺 ffmpeg 时把 flac 改名成 .mp3，那比失败更糟。
    let ext = detect_ext(&bytes).unwrap_or(&song.kind).to_string();
    let stem = naming::track_stem(&item.artists, &item.title);
    let path = naming::dedupe_path(out_dir, &stem, &ext);

    tokio::fs::write(&path, &bytes).await?;

    // 补全 ID3。音乐库软件读的是标签而非文件名，缺了它产物就是「未知专辑」。
    if ext == "mp3" {
        let cover = fetch_cover(&item.cover_url).await;
        if let Err(e) = write_id3(&path, item, cover) {
            // 标签失败不该让整首歌算作下载失败——音频本身是好的
            eprintln!("ID3 写入失败（不影响音频本身）: {e}");
        }
    }

    Ok(TrackRecord {
        track_id: item.track_id,
        title: item.title.clone(),
        artists: item.artists.clone(),
        album: item.album.clone(),
        file_path: path.to_string_lossy().to_string(),
        file_size: bytes.len() as i64,
        bitrate: song.bitrate,
        created_at: crate::db::now_ts(),
    })
}

/// 以文件头为准判定容器，不信接口声称的 type，也不信扩展名。
fn detect_ext(b: &[u8]) -> Option<&'static str> {
    if b.len() < 4 {
        return None;
    }
    if &b[..3] == b"ID3" || (b[0] == 0xFF && (b[1] & 0xE0) == 0xE0) {
        Some("mp3")
    } else if &b[..4] == b"fLaC" {
        Some("flac")
    } else if &b[..4] == b"RIFF" {
        Some("wav")
    } else {
        None
    }
}

/// 拉取专辑封面。失败一律返回 None——封面缺失不该让一首歌算作下载失败。
///
/// 用独立的短超时客户端：封面服务器偶发慢响应时，不应拖住整个串行队列。
async fn fetch_cover(url: &str) -> Option<(String, Vec<u8>)> {
    if url.is_empty() {
        return None;
    }
    // 网易云的图片地址支持尺寸参数，取 500x500 足够做专辑封面，
    // 免得把原图（可能上千像素）整个塞进每个 mp3。
    let sized = format!("{url}?param=500y500");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .ok()?;
    let resp = client
        .get(&sized)
        .header("Referer", "https://music.163.com")
        .send()
        .await
        .ok()?;

    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();

    let data = resp.bytes().await.ok()?.to_vec();
    if data.len() < 512 {
        return None; // 多半是错误页而非图片
    }
    Some((mime, data))
}

pub fn write_id3(path: &Path, item: &QueueItem, cover: Option<(String, Vec<u8>)>) -> Result<()> {
    use id3::{frame::Picture, frame::PictureType, Tag, TagLike, Version};

    let mut tag = Tag::read_from_path(path).unwrap_or_else(|_| Tag::new());
    tag.set_title(&item.title);
    tag.set_artist(&item.artists);
    if !item.album.is_empty() {
        tag.set_album(&item.album);
    }
    if let Some((mime, data)) = cover {
        tag.add_frame(Picture {
            mime_type: mime,
            picture_type: PictureType::CoverFront,
            description: String::new(),
            data,
        });
    }
    tag.write_to_path(path, Version::Id3v24)
        .map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

async fn emit(
    db: &Arc<Mutex<Db>>,
    on_progress: &impl Fn(Progress),
    current: Option<String>,
    msg: &str,
    running: bool,
) {
    let stats = {
        let d = db.lock().await;
        d.stats().unwrap_or_default()
    };
    on_progress(Progress {
        current,
        done: stats.done,
        failed: stats.failed,
        pending: stats.pending,
        running,
        last_message: msg.to_string(),
    });
}
