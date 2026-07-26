//! 网易云 HTTP 客户端（异步）
//!
//! 只用 weapi。eapi 的 song/url/v1 稳定返回 200 + 空 body，已排查并放弃（ADR-0006）。
//!
//! 权限边界（ADR-0001 硬约束）：只请求当前登录账号权限范围内的资源。
//! 无版权 / 需单独购买 / 地区受限的曲目，服务端会返回空直链，
//! 此处如实上报为「无下载权限」并跳过，不做任何绕过尝试。

use crate::crypto;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// ADR-0003：下载固定 320k。目标格式恒为 mp3，因此不需要任何转码。
pub const TARGET_BITRATE: i64 = 320_000;

/// 试听用标准音质：省流量、起播快。
/// 试听是「要不要下载」的判断依据，不是最终产物，没必要拉 320k。
pub const PREVIEW_BITRATE: i64 = 128_000;

/// 曲目可用性（ADR-0007 / 术语表）
///
/// 搜索会大量遇到账号权限之外的曲目。区分它们不是为了绕过——按 ADR-0001 的硬约束
/// 这些一律不下载——而是为了在 UI 上**提前说清楚**，而不是让用户点了之后
/// 收到一个语焉不详的失败。
///
/// 同时这也是重试语义的分界：不可用属于终局失败，重试没有意义（ADR-0009）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// 当前账号可下载
    Downloadable,
    /// 需要会员
    VipOnly,
    /// 需单独购买（数字专辑）
    PayAlbum,
    /// 无版权 / 已下架 / 地区限制
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: i64,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub duration_ms: i64,
    /// 专辑封面地址，用于补全 ID3（ADR-0003：缺它则产物在音乐库里全是「未知专辑」）
    pub cover_url: String,
    pub availability: Availability,
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub user_id: i64,
    pub nickname: String,
    pub vip_type: i64,
    pub avatar_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub tracks: Vec<Track>,
    pub total: i64,
}

/// 取到的直链。`bitrate` 是**实际到手**的值，可能低于请求值——
/// 术语表专门为「请求值 ≠ 到手值」立了条目。
pub struct SongUrl {
    pub url: String,
    pub bitrate: i64,
    pub size: i64,
    pub kind: String,
}

pub struct NcmClient {
    http: reqwest::Client,
    cookies: HashMap<String, String>,
}

impl NcmClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(UA)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let mut cookies = HashMap::new();
        cookies.insert("os".into(), "pc".into());
        cookies.insert("appver".into(), "8.9.70".into());
        cookies.insert("osver".into(), "15.0".into());
        cookies.insert("deviceId".into(), device_id());

        Ok(Self { http, cookies })
    }

    pub fn is_logged_in(&self) -> bool {
        self.cookies.contains_key("MUSIC_U")
    }

    pub fn music_u(&self) -> Option<&String> {
        self.cookies.get("MUSIC_U")
    }

    /// 从钥匙串恢复登录态（ADR-0004）。
    pub fn restore(&mut self, music_u: &str, csrf: &str) {
        self.cookies.insert("MUSIC_U".into(), music_u.to_string());
        if !csrf.is_empty() {
            self.cookies.insert("__csrf".into(), csrf.to_string());
        }
    }

    pub fn csrf(&self) -> String {
        self.cookies.get("__csrf").cloned().unwrap_or_default()
    }

    pub fn logout(&mut self) {
        self.cookies.remove("MUSIC_U");
        self.cookies.remove("__csrf");
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn absorb(&mut self, headers: &reqwest::header::HeaderMap) {
        for hv in headers.get_all(reqwest::header::SET_COOKIE) {
            let Ok(s) = hv.to_str() else { continue };
            let Some(pair) = s.split(';').next() else { continue };
            if let Some((k, v)) = pair.split_once('=') {
                let (k, v) = (k.trim(), v.trim());
                if !k.is_empty() && !v.is_empty() {
                    self.cookies.insert(k.to_string(), v.to_string());
                }
            }
        }
    }

    pub async fn weapi(&mut self, path: &str, mut payload: Value) -> Result<Value> {
        let csrf = self.csrf();
        if let Value::Object(ref mut m) = payload {
            m.insert("csrf_token".into(), Value::String(csrf.clone()));
        }
        let (params, enc_sec_key) = crypto::weapi(&payload.to_string())?;

        let resp = self
            .http
            .post(format!("https://music.163.com/weapi/{path}?csrf_token={csrf}"))
            .header("Referer", "https://music.163.com")
            .header("Origin", "https://music.163.com")
            .header("Cookie", self.cookie_header())
            .form(&[("params", params), ("encSecKey", enc_sec_key)])
            .send()
            .await?;

        self.absorb(resp.headers());
        let text = resp.text().await?;
        serde_json::from_str(&text).map_err(|e| {
            anyhow!(
                "weapi/{path} 响应不是 JSON: {e}\n原文: {}",
                text.chars().take(200).collect::<String>()
            )
        })
    }

    // ── 扫码登录 ─────────────────────────────────────────────

    pub async fn qr_key(&mut self) -> Result<String> {
        let r = self.weapi("login/qrcode/unikey", json!({ "type": 1 })).await?;
        r["unikey"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| anyhow!("未取到 unikey，响应: {r}"))
    }

    /// 返回状态码：801 待扫 / 802 待确认 / 803 成功 / 800 过期
    pub async fn qr_check(&mut self, unikey: &str) -> Result<i64> {
        let r = self
            .weapi("login/qrcode/client/login", json!({ "key": unikey, "type": 1 }))
            .await?;
        Ok(r["code"].as_i64().unwrap_or(0))
    }

    // ── 账号与歌单 ───────────────────────────────────────────

    pub async fn account(&mut self) -> Result<Profile> {
        let r = self.weapi("w/nuser/account/get", json!({})).await?;
        let p = &r["profile"];
        Ok(Profile {
            user_id: p["userId"]
                .as_i64()
                .ok_or_else(|| anyhow!("未取到 userId，登录态可能已失效"))?,
            nickname: p["nickname"].as_str().unwrap_or("").to_string(),
            vip_type: p["vipType"].as_i64().unwrap_or(0),
            avatar_url: p["avatarUrl"].as_str().unwrap_or("").to_string(),
        })
    }

    // ── 搜索 ─────────────────────────────────────────────────

    /// 搜索单曲。
    ///
    /// 用 `cloudsearch/pc` 而非老的 `search/get`：前者的结果带 `privilege`，
    /// 能据此判断当前账号是否有下载权限（ADR-0007）。没有它，用户只能靠点击
    /// 试错才知道哪些下不了。
    pub async fn search_songs(
        &mut self,
        keyword: &str,
        limit: i64,
        offset: i64,
    ) -> Result<SearchResult> {
        let r = self
            .weapi(
                "cloudsearch/pc",
                json!({
                    "s": keyword,
                    "type": 1,      // 1=单曲
                    "limit": limit,
                    "offset": offset,
                    "total": true,
                }),
            )
            .await?;

        let tracks = r["result"]["songs"]
            .as_array()
            .map(|a| a.iter().filter_map(parse_track).collect())
            .unwrap_or_default();

        Ok(SearchResult {
            tracks,
            total: r["result"]["songCount"].as_i64().unwrap_or(0),
        })
    }

    // ── 直链 ─────────────────────────────────────────────────

    /// 逐首现取（ADR-0005 硬约束）。
    ///
    /// **不要改成批量预取**：直链有有效期，队列跑到后段时先前取的链接已经失效，
    /// 且失败会伪装成网络错误，极难排查。
    pub async fn song_url(&mut self, track_id: i64, bitrate: i64) -> Result<SongUrl> {
        let r = self
            .weapi(
                "song/enhance/player/url",
                json!({ "ids": format!("[{track_id}]"), "br": bitrate }),
            )
            .await?;

        let item = r["data"]
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| anyhow!("song/url 无数据"))?;

        let url = item["url"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("无下载权限"))?;

        Ok(SongUrl {
            url: url.to_string(),
            bitrate: item["br"].as_i64().unwrap_or(0),
            size: item["size"].as_i64().unwrap_or(0),
            kind: item["type"].as_str().unwrap_or("mp3").to_lowercase(),
        })
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
}

fn parse_track(v: &Value) -> Option<Track> {
    // 歌单详情用 ar/al，song/detail 老字段用 artists/album，两种都认
    let artists = v["ar"]
        .as_array()
        .or_else(|| v["artists"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x["name"].as_str())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();

    Some(Track {
        id: v["id"].as_i64()?,
        title: v["name"].as_str().unwrap_or("").to_string(),
        artists,
        album: v["al"]["name"]
            .as_str()
            .or_else(|| v["album"]["name"].as_str())
            .unwrap_or("")
            .to_string(),
        duration_ms: v["dt"].as_i64().or_else(|| v["duration"].as_i64()).unwrap_or(0),
        cover_url: v["al"]["picUrl"]
            .as_str()
            .or_else(|| v["album"]["picUrl"].as_str())
            .unwrap_or("")
            .to_string(),
        availability: parse_availability(&v["privilege"]),
    })
}

/// 从 `privilege` 推断可用性。
///
/// 最可靠的信号是 `dl`（当前账号可下载的最高码率）——服务端已经把账号权限
/// 算进去了，比自己拿 fee 和 vipType 对着推靠谱得多。
///
/// 注意这只是**预判**，不是保证：真正的判据仍是 song/url 是否返回非空直链。
/// 预判的意义在于让用户在点击之前就知道哪些下不了，而不是替代实际校验。
fn parse_availability(p: &Value) -> Availability {
    // 歌单详情等接口不带 privilege。此时不做判断，交给实际下载时的直链校验。
    if p.is_null() {
        return Availability::Downloadable;
    }
    if p["st"].as_i64().unwrap_or(0) < 0 {
        return Availability::Unavailable; // 已下架 / 无版权
    }
    if p["dl"].as_i64().unwrap_or(0) > 0 {
        return Availability::Downloadable;
    }
    match p["fee"].as_i64().unwrap_or(0) {
        4 => Availability::PayAlbum,
        1 => Availability::VipOnly,
        _ => Availability::Unavailable,
    }
}

fn device_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| std::char::from_digit(rng.gen_range(0..16), 16).unwrap_or('0'))
        .collect()
}
