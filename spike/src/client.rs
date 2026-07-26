//! 最小 HTTP 客户端：自管 cookie，封装 weapi / eapi 两种请求。

use crate::crypto;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct Client {
    http: reqwest::blocking::Client,
    cookies: HashMap<String, String>,
}

impl Client {
    pub fn new() -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .user_agent(UA)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        // 客户端标识。缺了这些 eapi 会以各种难懂的方式失败。
        let mut cookies = HashMap::new();
        cookies.insert("os".into(), "pc".into());
        cookies.insert("appver".into(), "8.9.70".into());
        cookies.insert("osver".into(), "15.0".into());
        cookies.insert("deviceId".into(), device_id());

        Ok(Self { http, cookies })
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// 吸收响应里的 Set-Cookie。登录成功时 MUSIC_U 就是这么进来的。
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

    pub fn cookie(&self, name: &str) -> Option<&String> {
        self.cookies.get(name)
    }

    /// ⚠ 仅供 spike 调试：把会话缓存到本地，避免每次调试都要重新扫码。
    /// 文件含 MUSIC_U（等价于账号凭据）。正式实现按 ADR-0004 走系统钥匙串，
    /// 绝不落盘成明文；spike 结束后应删除该文件。
    pub fn save_session(&self, path: &str) -> Result<()> {
        std::fs::write(path, serde_json::to_string(&self.cookies)?)?;
        Ok(())
    }

    pub fn load_session(&mut self, path: &str) -> bool {
        let Ok(s) = std::fs::read_to_string(path) else {
            return false;
        };
        let Ok(m) = serde_json::from_str::<HashMap<String, String>>(&s) else {
            return false;
        };
        if !m.contains_key("MUSIC_U") {
            return false;
        }
        self.cookies.extend(m);
        true
    }

    pub fn http(&self) -> &reqwest::blocking::Client {
        &self.http
    }

    pub fn weapi(&mut self, path: &str, payload: Value) -> Result<Value> {
        let mut payload = payload;
        // 登录后的接口需要 csrf_token，且 query 与 body 里都要带
        let csrf = self.cookies.get("__csrf").cloned().unwrap_or_default();
        if let Value::Object(ref mut m) = payload {
            m.insert("csrf_token".into(), Value::String(csrf.clone()));
        }

        let (params, enc_sec_key) = crypto::weapi(&payload.to_string())?;
        let url = format!("https://music.163.com/weapi/{path}?csrf_token={csrf}");

        let resp = self
            .http
            .post(&url)
            .header("Referer", "https://music.163.com")
            .header("Origin", "https://music.163.com")
            .header("Cookie", self.cookie_header())
            .form(&[("params", params), ("encSecKey", enc_sec_key)])
            .send()?;

        self.absorb(resp.headers());
        let text = resp.text()?;
        serde_json::from_str(&text)
            .map_err(|e| anyhow!("weapi/{path} 响应不是 JSON: {e}\n原文: {}", truncate(&text)))
    }

    /// eapi 要求请求体里内嵌一份客户端标识 `header`，且 Cookie 用同一批字段。
    /// 缺了它服务端会返回 **HTTP 200 + 空 body**——不报错，只是什么都不给。
    fn eapi_header(&self) -> Value {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let get = |k: &str, d: &str| self.cookies.get(k).cloned().unwrap_or_else(|| d.to_string());

        json!({
            "osver":       get("osver", "15.0"),
            "deviceId":    get("deviceId", ""),
            "appver":      get("appver", "8.9.70"),
            "versioncode": get("versioncode", "140"),
            "mobilename":  "",
            "buildver":    (now_ms / 1000).to_string(),
            "resolution":  "1920x1080",
            "__csrf":      get("__csrf", ""),
            "os":          get("os", "pc"),
            "channel":     "",
            "requestId":   format!("{}_{:04}", now_ms, now_ms % 1000),
            "MUSIC_U":     get("MUSIC_U", ""),
        })
    }

    /// `api_path` 是摘要用的 `/api/...` 路径；实际请求打到 interface 域的 `/eapi/...`。
    pub fn eapi(&mut self, api_path: &str, payload: Value) -> Result<Value> {
        let header = self.eapi_header();

        let mut payload = payload;
        if let Value::Object(ref mut m) = payload {
            m.insert("header".into(), header.clone());
        }

        // Cookie 也由 header 的字段拼成，与请求体保持一致
        let eapi_cookie = header
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| format!("{k}={s}")))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();

        let params = crypto::eapi(api_path, &payload.to_string())?;
        let url = format!(
            "https://interface.music.163.com/eapi/{}",
            api_path.trim_start_matches("/api/")
        );

        let resp = self
            .http
            .post(&url)
            .header("Referer", "https://music.163.com")
            .header("Cookie", eapi_cookie)
            .form(&[("params", params)])
            .send()?;

        self.absorb(resp.headers());
        let status = resp.status();
        let ctype = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("(无)")
            .to_string();
        let cenc = resp
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("(无)")
            .to_string();
        let bytes = resp.bytes()?;

        let text = crypto::eapi_decrypt_response(&bytes).map_err(|e| {
            // 解密失败时把原始响应留下来，否则只能靠猜
            let dump = format!("{}/eapi-raw-response.bin", super::SCRATCH);
            let _ = std::fs::write(&dump, &bytes);
            anyhow!(
                "{e}\n\
                 ── 诊断信息 ──\n\
                 HTTP 状态      : {status}\n\
                 Content-Type   : {ctype}\n\
                 Content-Encoding: {cenc}\n\
                 响应长度       : {} 字节\n\
                 前 64 字节 hex : {}\n\
                 可打印前缀     : {:?}\n\
                 原始响应已存至 : {dump}",
                bytes.len(),
                hex::encode(&bytes[..bytes.len().min(64)]),
                String::from_utf8_lossy(&bytes[..bytes.len().min(120)])
            )
        })?;

        serde_json::from_str(&text)
            .map_err(|e| anyhow!("eapi{api_path} 响应不是 JSON: {e}\n原文: {}", truncate(&text)))
    }
}

fn device_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(20260726);
    format!("{:032x}", nanos)
}

fn truncate(s: &str) -> String {
    s.chars().take(200).collect()
}
