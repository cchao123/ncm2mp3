//! Tauri 命令层
//!
//! 设计文档见 ../../docs/，每个关键行为都对应一份 ADR。
//! 权限边界（ADR-0001）：只下载当前登录账号权限内的曲目，不绕过任何版权限制。

mod client;
mod crypto;
mod db;
mod downloader;
mod keychain;
mod naming;
mod ncm;

use client::{Availability, NcmClient, Profile, SearchResult, Track};
use db::{Db, QueueItem, TrackRecord};
use downloader::{Downloader, Progress, Throttle};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

pub struct AppState {
    client: Arc<Mutex<NcmClient>>,
    db: Arc<Mutex<Db>>,
    downloader: Downloader,
    out_dir: Mutex<PathBuf>,
    progress: Arc<Mutex<Progress>>,
}

/// anyhow::Error 不能跨 IPC 边界，统一转成字符串。
fn s<T>(r: anyhow::Result<T>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct QrStart {
    unikey: String,
    svg: String,
}

#[derive(Serialize)]
pub struct QrPoll {
    code: i64,
    message: String,
    logged_in: bool,
}

// ── 认证 ─────────────────────────────────────────────────────

#[tauri::command]
async fn auth_status(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(state.client.lock().await.is_logged_in())
}

#[tauri::command]
async fn qr_start(state: tauri::State<'_, AppState>) -> Result<QrStart, String> {
    let mut c = state.client.lock().await;
    let unikey = s(c.qr_key().await)?;
    drop(c);

    let url = format!("https://music.163.com/login?codekey={unikey}");
    let code = qrcode::QrCode::new(url.as_bytes()).map_err(|e| e.to_string())?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .build();

    Ok(QrStart { unikey, svg })
}

#[tauri::command]
async fn qr_poll(state: tauri::State<'_, AppState>, unikey: String) -> Result<QrPoll, String> {
    let mut c = state.client.lock().await;
    let code = s(c.qr_check(&unikey).await)?;

    let message = match code {
        801 => "等待扫描",
        802 => "已扫描，请在手机上确认",
        803 => "登录成功",
        800 => "二维码已过期",
        _ => "未知状态",
    }
    .to_string();

    let logged_in = code == 803 && c.is_logged_in();
    if logged_in {
        // 登录成功即写入钥匙串（ADR-0004）
        let music_u = c.music_u().cloned().unwrap_or_default();
        let csrf = c.csrf();
        drop(c);
        s(keychain::save(&music_u, &csrf))?;
    }

    Ok(QrPoll { code, message, logged_in })
}

#[tauri::command]
async fn logout(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.client.lock().await.logout();
    s(keychain::clear())
}

// ── 账号与歌单 ───────────────────────────────────────────────

#[tauri::command]
async fn get_profile(state: tauri::State<'_, AppState>) -> Result<Profile, String> {
    s(state.client.lock().await.account().await)
}

/// 搜索单曲。结果自带可用性判断（ADR-0007）。
#[tauri::command]
async fn search_songs(
    state: tauri::State<'_, AppState>,
    keyword: String,
    limit: i64,
    offset: i64,
) -> Result<SearchResult, String> {
    let kw = keyword.trim().to_string();
    if kw.is_empty() {
        return Ok(SearchResult { tracks: vec![], total: 0 });
    }
    s(state
        .client
        .lock()
        .await
        .search_songs(&kw, limit, offset)
        .await)
}

/// 试听直链。
///
/// 用标准音质而非下载用的 320k——试听是「要不要下载」的判断依据，不是最终产物，
/// 省流量也起播更快。
///
/// 注意这仍会消耗一次 song/url 请求。试听是用户主动的低频操作，不像批量下载
/// 那样需要节流（ADR-0005），但不应做成鼠标悬停自动播放之类的高频触发。
#[tauri::command]
async fn preview_url(
    state: tauri::State<'_, AppState>,
    track_id: i64,
) -> Result<String, String> {
    let song = s(state
        .client
        .lock()
        .await
        .song_url(track_id, client::PREVIEW_BITRATE)
        .await)?;
    Ok(song.url)
}

/// 返回这批曲目里哪些已经下载过（按曲目 ID，且校验文件仍在磁盘上）。
#[tauri::command]
async fn filter_downloaded(
    state: tauri::State<'_, AppState>,
    track_ids: Vec<i64>,
) -> Result<Vec<i64>, String> {
    let d = state.db.lock().await;
    Ok(track_ids
        .into_iter()
        .filter(|id| d.is_downloaded(*id).unwrap_or(None).is_some())
        .collect())
}

// ── 队列与下载 ───────────────────────────────────────────────

#[tauri::command]
async fn enqueue(state: tauri::State<'_, AppState>, tracks: Vec<Track>) -> Result<usize, String> {
    let items: Vec<QueueItem> = tracks
        .into_iter()
        // 防御性过滤：即使前端漏筛，也不把下不了的曲目塞进队列。
        // 它们只会白白消耗请求配额（ADR-0005 的风控考量），最终仍旧落入失败清单。
        // 按 ADR-0001，对这些曲目不做任何绕过尝试。
        .filter(|t| t.availability == Availability::Downloadable)
        .map(|t| QueueItem {
            track_id: t.id,
            title: t.title,
            artists: t.artists,
            album: t.album,
            status: "pending".into(),
            reason: None,
            cover_url: t.cover_url,
        })
        .collect();
    s(state.db.lock().await.enqueue(&items))
}

#[tauri::command]
async fn start_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if state.downloader.running.load(Ordering::SeqCst) {
        return Err("下载已在进行中".into());
    }
    let client = state.client.clone();
    let db = state.db.clone();
    let out_dir = state.out_dir.lock().await.clone();
    let stop = state.downloader.stop.clone();
    let running = state.downloader.running.clone();
    let progress = state.progress.clone();

    tauri::async_runtime::spawn(async move {
        downloader::run_queue(
            client,
            db,
            out_dir,
            Throttle::default(),
            stop,
            running,
            move |p| {
                // 前端靠事件驱动进度条；同时留一份快照供轮询兜底
                let _ = app.emit("download-progress", &p);
                if let Ok(mut g) = progress.try_lock() {
                    *g = p;
                }
            },
        )
        .await;
    });
    Ok(())
}

/// 暂停。ADR-0009：关窗口即暂停，进度留在 SQLite，重开可续传。
#[tauri::command]
async fn stop_download(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.downloader.stop.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
async fn get_progress(state: tauri::State<'_, AppState>) -> Result<Progress, String> {
    let stats = s(state.db.lock().await.stats())?;
    let mut p = state.progress.lock().await.clone();
    p.done = stats.done;
    p.failed = stats.failed;
    p.pending = stats.pending;
    p.running = state.downloader.running.load(Ordering::SeqCst);
    Ok(p)
}

/// 失败清单（ADR-0009：队列不中断，跑完统一给清单）
#[tauri::command]
async fn get_failed(state: tauri::State<'_, AppState>) -> Result<Vec<QueueItem>, String> {
    s(state.db.lock().await.failed_items())
}

#[tauri::command]
async fn clear_finished(state: tauri::State<'_, AppState>) -> Result<(), String> {
    s(state.db.lock().await.clear_finished())
}

// ── 曲库与文件管理器 ─────────────────────────────────────────

#[tauri::command]
async fn get_library(state: tauri::State<'_, AppState>) -> Result<Vec<TrackRecord>, String> {
    s(state.db.lock().await.library(5000))
}

#[tauri::command]
async fn get_output_dir(state: tauri::State<'_, AppState>) -> Result<String, String> {
    Ok(state.out_dir.lock().await.to_string_lossy().to_string())
}

#[tauri::command]
async fn set_output_dir(state: tauri::State<'_, AppState>, dir: String) -> Result<(), String> {
    let p = PathBuf::from(&dir);
    std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
    *state.out_dir.lock().await = p;
    Ok(())
}

/// 「打开目录」——原始需求第三条。ADR-0008 将其定位为逃生口，
/// 日常使用走应用内曲库页。
#[tauri::command]
async fn open_output_dir(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = state.out_dir.lock().await.clone();
    std::fs::create_dir_all(&dir).ok();
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// 在文件管理器中定位并选中某个文件。
/// 与「打开所在目录」是不同操作，术语表为此立了 Reveal 条目。
#[tauri::command]
async fn reveal_file(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .reveal_item_in_dir(PathBuf::from(path))
        .map_err(|e| e.to_string())
}

// ── 历史 NCM 兼容分支（ADR-0001）────────────────────────────

#[tauri::command]
async fn decrypt_ncm(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let out_dir = state.out_dir.lock().await.clone();
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    let r = s(ncm::decrypt(&path))?;
    let meta = r.metadata.clone().unwrap_or_default();
    let title = meta["musicName"].as_str().unwrap_or("").to_string();
    let artists = meta["artist"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_array().and_then(|p| p.first()).and_then(|n| n.as_str()))
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();

    let stem = if title.is_empty() {
        naming::sanitize_stem(
            std::path::Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "未命名".into())
                .as_str(),
        )
    } else {
        naming::track_stem(&artists, &title)
    };

    // ADR-0003：flac 原样输出，不改扩展名冒充 mp3
    let out = naming::dedupe_path(&out_dir, &stem, &r.format);
    std::fs::write(&out, &r.audio).map_err(|e| e.to_string())?;

    // NCM 容器里内嵌了专辑封面。原 Python 实现把它读出来就直接丢弃
    // （docs/README.md 的缺陷清单有记），这里写进 ID3。
    if r.format == "mp3" {
        let item = QueueItem {
            track_id: 0,
            title: title.clone(),
            artists: artists.clone(),
            album: meta["album"].as_str().unwrap_or("").to_string(),
            status: "done".into(),
            reason: None,
            cover_url: String::new(),
        };
        let cover = r.cover.clone().map(|d| ("image/jpeg".to_string(), d));
        if let Err(e) = downloader::write_id3(&out, &item, cover) {
            eprintln!("ID3 写入失败（不影响音频本身）: {e}");
        }
    }

    Ok(out.to_string_lossy().to_string())
}

// ── 启动 ─────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let base = app.path().app_data_dir()?;
            let db = Db::open(&base.join("library.db"))?;

            // 默认输出到 ~/Music/网易云下载
            let out_dir = app
                .path()
                .audio_dir()
                .unwrap_or_else(|_| base.clone())
                .join("网易云下载");
            std::fs::create_dir_all(&out_dir).ok();

            let mut client = NcmClient::new()?;
            // 有钥匙串凭据就直接恢复登录态，免去每次扫码（ADR-0004）
            if let Some((music_u, csrf)) = keychain::load() {
                client.restore(&music_u, &csrf);
            }

            app.manage(AppState {
                client: Arc::new(Mutex::new(client)),
                db: Arc::new(Mutex::new(db)),
                downloader: Downloader::new(),
                out_dir: Mutex::new(out_dir),
                progress: Arc::new(Mutex::new(Progress {
                    current: None,
                    done: 0,
                    failed: 0,
                    pending: 0,
                    running: false,
                    last_message: String::new(),
                })),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            auth_status,
            qr_start,
            qr_poll,
            logout,
            get_profile,
            search_songs,
            preview_url,
            filter_downloaded,
            enqueue,
            start_download,
            stop_download,
            get_progress,
            get_failed,
            clear_finished,
            get_library,
            get_output_dir,
            set_output_dir,
            open_output_dir,
            reveal_file,
            decrypt_ncm,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}
