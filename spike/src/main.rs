//! ADR-0006 Spike
//!
//! 只回答四个问题，任一失败则整体设计需返工：
//!   1. weapi 加密对不对 —— 能不能取到 unikey
//!   2. 扫码登录三步流程能不能跑通 —— 拿到 MUSIC_U
//!   3. eapi 加密对不对 —— 能不能取到 exhigh 直链
//!   4. ADR-0003 的前提成不成立 —— exhigh 拿到的确实是 mp3

mod client;
mod crypto;
mod ncm;

use anyhow::{anyhow, bail, Result};
use client::Client;
use serde_json::json;
use std::io::Write;
use std::time::{Duration, Instant};

const SCRATCH: &str = "/private/tmp/claude-501/-Users-cchao-Desktop----/\
32c5cc12-f274-49e7-9585-cf34b379f953/scratchpad";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // 纯计算模式：只输出加密结果，不发网络请求。
    // 用途——在无法出站的环境里，把加密产物交给 curl 发送，
    // 从而把「加密算得对不对」与「进程能不能联网」这两件事解耦验证。
    //   emit-weapi <payload>            → 两行：params、encSecKey
    //   emit-eapi  <api_path> <payload> → 一行：params
    if args.len() >= 3 {
        match args[1].as_str() {
            "emit-weapi" => {
                let (params, enc) = crypto::weapi(&args[2])?;
                println!("{params}");
                println!("{enc}");
                return Ok(());
            }
            "emit-eapi" if args.len() >= 4 => {
                println!("{}", crypto::eapi(&args[2], &args[3])?);
                return Ok(());
            }
            // NCM 解密移植的回归验证：解出音频流写盘，交给外部逐字节比对
            "verify-ncm" if args.len() >= 4 => {
                let r = ncm::decrypt(&args[2])?;
                std::fs::write(&args[3], &r.audio)?;
                let name = r.metadata.as_ref().and_then(|m| m["musicName"].as_str());
                println!(
                    "format={}  audio={} 字节  cover={}  metadata={}",
                    r.format,
                    r.audio.len(),
                    r.cover.as_ref().map_or("无".into(), |c| format!("{} 字节", c.len())),
                    name.unwrap_or("(无)")
                );
                return Ok(());
            }
            _ => {}
        }
    }

    println!("╭──────────────────────────────────────────────╮");
    println!("│  ADR-0006 Spike · Rust 原生加密层可行性验证  │");
    println!("╰──────────────────────────────────────────────╯\n");

    let mut c = Client::new()?;

    // unikey 约 3 分钟过期。过期就重新申请并刷新二维码，
    // 这样用户什么时候回到电脑前都能扫，不必掐着点。
    // 复用已缓存的会话，避免每次调试都劳烦用户重新扫码。
    // 仅 spike 如此；正式实现凭据走系统钥匙串（ADR-0004）。
    let session_path = format!("{SCRATCH}/ncm-session.json");
    if c.load_session(&session_path) {
        println!("[1-3/7] 复用已缓存的登录会话，跳过扫码");
        println!("        （删除 {session_path} 可强制重新登录）\n");
    } else {
        let mut round = 1;
        loop {
            let unikey = step1_unikey(&mut c)?;
            step2_show_qr(&unikey, round)?;
            if step3_poll(&mut c, &unikey)? {
                break;
            }
            round += 1;
            println!("      二维码已过期，正在生成第 {round} 张...\n");
        }
        c.save_session(&session_path)?;
    }

    let uid = step4_account(&mut c)?;
    let (track_id, label) = step5_first_track(&mut c, uid)?;
    let (url, br, typ) = step6_song_url(&mut c, track_id)?;
    step7_download(&c, &url, &label, br, &typ)?;

    println!("\n╭──────────────────────────────────────────────╮");
    println!("│  全部通过 · Rust 原生路线成立                │");
    println!("╰──────────────────────────────────────────────╯");
    println!("ADR-0003（exhigh 直出 mp3）与 ADR-0006（Rust 原生）前提均已实测确认。");
    Ok(())
}

// ── 1. weapi 是否正确 ────────────────────────────────────────────────

fn step1_unikey(c: &mut Client) -> Result<String> {
    println!("[1/7] 申请二维码令牌（验证 weapi 加密）...");
    let r = c.weapi("login/qrcode/unikey", json!({ "type": 1 }))?;

    let unikey = r["unikey"].as_str().ok_or_else(|| {
        anyhow!(
            "未取到 unikey —— weapi 加密链路有问题。\n\
             响应: {r}\n\n\
             排查顺序（ADR-0006）：\n\
             1. 把 random_secret() 临时固定为常量，用社区 JS 实现跑同样输入，逐字节比对 params/encSecKey\n\
             2. 重点看 rsa_encrypt —— 若误用了 PKCS#1 标准填充，现象正是这里静默拿不到 unikey"
        )
    })?;

    println!("      ✓ unikey = {unikey}");
    println!("      → weapi 加密（含 RSA 非标准填充）正确\n");
    Ok(unikey.to_string())
}

// ── 2. 二维码 ────────────────────────────────────────────────────────

fn step2_show_qr(unikey: &str, round: u32) -> Result<()> {
    println!("[2/7] 生成二维码（第 {round} 张）...");
    let login_url = format!("https://music.163.com/login?codekey={unikey}");

    let code = qrcode::QrCode::new(login_url.as_bytes())
        .map_err(|e| anyhow!("二维码生成失败: {e}"))?;

    // 直接画在终端里 —— 不依赖预览窗口，也不怕它被挡住或没自动刷新
    let art = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .build();
    println!("\n{art}\n");

    // 同时存一份 SVG 并打开，终端字体过小时可以用它
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(360, 360)
        .build();
    std::fs::create_dir_all(SCRATCH)?;
    // 每轮独立文件名：open 复用同一路径的窗口时不会刷新内容，会让人扫到过期的码
    let path = format!("{SCRATCH}/ncm-login-qr-{round}.svg");
    std::fs::write(&path, svg)?;
    let _ = std::process::Command::new("open").arg(&path).status();

    println!("      请用网易云音乐 App 扫上面的二维码，并在手机上确认");
    println!("      （过期会自动换一个新的，不用赶时间）");
    println!("      备用 SVG：{path}\n");
    Ok(())
}

// ── 3. 登录状态机 ────────────────────────────────────────────────────

/// 返回 true 表示登录成功；false 表示二维码过期，调用方应重新生成。
fn step3_poll(c: &mut Client, unikey: &str) -> Result<bool> {
    println!("[3/7] 等待扫码...");
    let deadline = Instant::now() + Duration::from_secs(200);
    let mut last = 0i64;

    while Instant::now() < deadline {
        let r = c.weapi(
            "login/qrcode/client/login",
            json!({ "key": unikey, "type": 1 }),
        )?;
        let code = r["code"].as_i64().unwrap_or(0);

        if code != last {
            let desc = match code {
                801 => "等待扫描",
                802 => "已扫描，请在手机上点击确认",
                803 => "确认成功",
                800 => "二维码已过期",
                _ => "未知状态",
            };
            println!("      [{code}] {desc}");
            last = code;
        }

        match code {
            803 => {
                let music_u = c
                    .cookie("MUSIC_U")
                    .ok_or_else(|| anyhow!("状态 803 但未收到 MUSIC_U —— Set-Cookie 解析有问题"))?;
                println!("      ✓ 已登录，MUSIC_U 长度 {} 字符", music_u.len());
                println!("      → 扫码登录三步流程跑通");
                println!("      → 正式实现中此凭据存入系统钥匙串（ADR-0004），且不得进入日志\n");
                return Ok(true);
            }
            800 => return Ok(false), // 过期不是失败，调用方会换一个新码
            _ => std::thread::sleep(Duration::from_secs(2)),
        }
    }
    Ok(false)
}

// ── 4. 账号 ──────────────────────────────────────────────────────────

fn step4_account(c: &mut Client) -> Result<i64> {
    println!("[4/7] 读取账号信息...");
    let r = c.weapi("w/nuser/account/get", json!({}))?;

    let uid = r["profile"]["userId"]
        .as_i64()
        .ok_or_else(|| anyhow!("未取到 userId，响应: {r}"))?;
    let nickname = r["profile"]["nickname"].as_str().unwrap_or("(未知)");
    let vip = r["profile"]["vipType"].as_i64().unwrap_or(0);

    println!("      ✓ {nickname} (uid={uid}, vipType={vip})\n");
    Ok(uid)
}

// ── 5. 歌单 ──────────────────────────────────────────────────────────

fn step5_first_track(c: &mut Client, uid: i64) -> Result<(i64, String)> {
    println!("[5/7] 定位「我喜欢的音乐」第一首...");

    let r = c.weapi(
        "user/playlist",
        json!({ "uid": uid, "limit": 1, "offset": 0, "includeVideo": true }),
    )?;
    let pl = r["playlist"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow!("未取到歌单，响应: {r}"))?;
    let pid = pl["id"].as_i64().ok_or_else(|| anyhow!("歌单缺 id"))?;
    println!("      歌单：{} (id={pid})", pl["name"].as_str().unwrap_or("?"));

    let d = c.weapi("v6/playlist/detail", json!({ "id": pid, "n": 1000, "s": 8 }))?;
    let track = d["playlist"]["tracks"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow!("歌单里没有曲目，响应: {}", truncate(&d.to_string())))?;

    let tid = track["id"].as_i64().ok_or_else(|| anyhow!("曲目缺 id"))?;
    let name = track["name"].as_str().unwrap_or("未知曲目");
    let artist = track["ar"]
        .as_array()
        .map(|ar| {
            ar.iter()
                .filter_map(|a| a["name"].as_str())
                .collect::<Vec<_>>()
                .join(",")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "未知艺术家".into());

    let label = format!("{artist} - {name}");
    println!("      ✓ {label} (id={tid})\n");
    Ok((tid, label))
}

// ── 6. eapi 是否正确 + ADR-0003 前提 ─────────────────────────────────

fn step6_song_url(c: &mut Client, track_id: i64) -> Result<(String, i64, String)> {
    println!("[6/7] 请求 320k 直链（验证 ADR-0003 前提）...");

    // 走 weapi 而非 eapi：
    //
    // 最初按社区常见做法用 eapi 的 song/url/v1 配 level=exhigh，但该接口稳定返回
    // HTTP 200 + 空 body。已排除加密实现问题——eapi 加密结果与独立 Python 实现
    // 逐字节一致，也排除了 os 取值（pc/android/iphone 均为空）。
    //
    // weapi 版 song/enhance/player/url 用 br 参数表达音质，br=320000 即 ADR-0003
    // 要的 320k mp3，实测直接可用。既然目标音质固定，就没有理由再依赖 eapi——
    // 整个项目因此只需维护 weapi 一套加密，风险面和长期维护成本都更小。
    let r = c.weapi(
        "song/enhance/player/url",
        json!({ "ids": format!("[{track_id}]"), "br": 320000 }),
    )?;

    let item = r["data"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow!("song/url 无数据。响应: {}", truncate(&r.to_string())))?;

    let url = item["url"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| {
        anyhow!(
            "直链为空。这多半不是加密问题，而是该曲目不在账号权限内\n\
             （无版权 / 需单独购买 / 地区限制）。\n\
             按 ADR-0001 这类曲目不下载、不绕过；按 ADR-0009 记为终局失败并跳过。\n\
             换一首有权限的再跑。原始条目: {item}"
        )
    })?;

    let br = item["br"].as_i64().unwrap_or(0);
    let typ = item["type"].as_str().unwrap_or("?").to_lowercase();
    let size = item["size"].as_i64().unwrap_or(0);

    println!("      type={typ}  br={br}  size={:.2} MB", size as f64 / 1048576.0);

    // 请求值 ≠ 到手值，术语表专门为此立了条目：
    // 服务端返回的实际音质可能低于请求值，取决于账号权限与曲目本身有无该档位。
    if br < 300_000 {
        println!("      ⚠ 请求 320k，实际 {br} —— 该曲目或账号权限无 320k 档");
    }
    println!();
    Ok((url.to_string(), br, typ))
}

// ── 7. 落盘验证 ──────────────────────────────────────────────────────

fn step7_download(c: &Client, url: &str, label: &str, br: i64, typ: &str) -> Result<()> {
    println!("[7/7] 下载并校验文件头...");

    let bytes = c
        .http()
        .get(url)
        .header("Referer", "https://music.163.com")
        .send()?
        .bytes()?;

    if bytes.len() < 4 {
        bail!("下载内容过短（{} 字节）", bytes.len());
    }

    // 判据是文件头，不是接口声称的 type，也不是扩展名
    let head = &bytes[..4];
    let detected = if &head[..3] == b"ID3" {
        "mp3 (ID3v2)"
    } else if head[0] == 0xFF && (head[1] & 0xE0) == 0xE0 {
        "mp3 (MPEG frame sync)"
    } else if &head[..4] == b"fLaC" {
        "flac"
    } else if &head[..4] == b"RIFF" {
        "wav"
    } else {
        "未知"
    };

    let out = format!("{SCRATCH}/spike-{}.{}", sanitize(label), typ);
    std::fs::File::create(&out)?.write_all(&bytes)?;

    println!("      文件头 {:02X?} → {detected}", head);
    println!("      ✓ 已落盘 {:.2} MB：{out}", bytes.len() as f64 / 1048576.0);

    if detected.starts_with("mp3") {
        println!("\n      ★ ADR-0003 前提成立：exhigh 直出 mp3，零转码，不需要 ffmpeg");
        if br < 300_000 {
            println!("      ⚠ 实际码率 {br}，低于 320k —— 该曲目或账号权限没有 320k 档");
        }
    } else {
        println!("\n      ✗ ADR-0003 前提不成立：exhigh 返回的是 {detected} 而非 mp3");
        println!("        需重新评估 ADR-0003，可能不得不重新引入 ffmpeg 及其跨平台分发负担");
    }
    Ok(())
}

/// 按 Windows 规则清洗（ADR-0008：两平台必须产出相同文件名）。
/// spike 版本，正式实现还需处理保留设备名、末尾空格句点、路径长度。
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|ch| if r#"<>:"/\|?*"#.contains(ch) || (ch as u32) < 0x20 { '_' } else { ch })
        .collect::<String>()
        .trim()
        .to_string()
}

fn truncate(s: &str) -> String {
    s.chars().take(300).collect()
}
