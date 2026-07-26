//! NCM 容器解密（ADR-0001 的兼容分支）
//!
//! 主链路走 API 直链，这里只服务一个场景：用户手头已有的历史 .ncm 文件。
//!
//! 移植自 `ncm_to_mp3.py`，但**不继承其缺陷**——原实现在元数据段后只跳 5 字节
//! （应为 9），导致音频流整体错位，产出的每个文件都是不可播放的乱码。
//! 验收方式只有一种：与已知正确产物逐字节比对。

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyInit};
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};

type Aes128EcbDec = ecb::Decryptor<aes::Aes128>;

const MAGIC: &[u8; 8] = b"CTENFDAM";
const CORE_KEY: &[u8; 16] = b"hzHRAmso5kInbaxW";
const META_KEY: &[u8; 16] = b"#14ljk_!\\]&0U<'(";

pub struct Ncm {
    pub audio: Vec<u8>,
    pub format: String,
    pub metadata: Option<serde_json::Value>,
    pub cover: Option<Vec<u8>>,
}

/// 读小端 u32 并前进游标
fn u32_le(buf: &[u8], pos: &mut usize) -> Result<u32> {
    let end = *pos + 4;
    if end > buf.len() {
        bail!("文件在偏移 {} 处意外结束", pos);
    }
    let v = u32::from_le_bytes(buf[*pos..end].try_into()?);
    *pos = end;
    Ok(v)
}

fn take<'a>(buf: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8]> {
    let end = *pos + n;
    if end > buf.len() {
        bail!("需要 {n} 字节但文件只剩 {}", buf.len().saturating_sub(*pos));
    }
    let s = &buf[*pos..end];
    *pos = end;
    Ok(s)
}

fn ecb_decrypt(data: &[u8], key: &[u8; 16]) -> Result<Vec<u8>> {
    Aes128EcbDec::new(key.into())
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|e| anyhow!("AES-ECB 解密失败: {e}"))
}

pub fn decrypt(path: &str) -> Result<Ncm> {
    let buf = std::fs::read(path)?;
    let mut pos = 0usize;

    // 1. 魔术字
    if take(&buf, &mut pos, 8)? != MAGIC {
        bail!("不是 NCM 文件（魔术字不匹配）: {path}");
    }
    pos += 2; // gap

    // 2. RC4 密钥：XOR 0x64 → AES-ECB → 去掉 "neteasecloudmusic" 17 字节前缀
    let key_len = u32_le(&buf, &mut pos)? as usize;
    let key_enc: Vec<u8> = take(&buf, &mut pos, key_len)?.iter().map(|b| b ^ 0x64).collect();
    let key_raw = ecb_decrypt(&key_enc, CORE_KEY)?;
    if key_raw.len() <= 17 {
        bail!("RC4 密钥段过短");
    }
    let rc4_key = &key_raw[17..];

    // 3. RC4-KSA 构建密钥盒
    let key_box = build_key_box(rc4_key);

    // 4. 元数据：XOR 0x63 → 去 "163 key(Don't modify):" 22 字节 → base64 → AES-ECB → 去 "music:" 6 字节
    let meta_len = u32_le(&buf, &mut pos)? as usize;
    let mut metadata = None;
    if meta_len > 0 {
        let meta_enc: Vec<u8> = take(&buf, &mut pos, meta_len)?.iter().map(|b| b ^ 0x63).collect();
        if meta_enc.len() > 22 {
            if let Ok(b64) = STANDARD.decode(&meta_enc[22..]) {
                if let Ok(pt) = ecb_decrypt(&b64, META_KEY) {
                    if pt.len() > 6 {
                        metadata = serde_json::from_slice(&pt[6..]).ok();
                    }
                }
            }
        }
    }

    // 5. CRC32(4) + gap(5) = 9 字节
    //
    //    ★ 原 Python 实现此处只跳了 5，是导致解密全盘失效的根因：
    //      后续 image_size 落在 gap 的全零字节上而恒为 0，封面段被当作音频起始。
    //      它不报错，只是安静地产出垃圾——所以必须靠逐字节比对来验收。
    pos += 9;

    // 6. 封面（原实现直接丢弃，这里保留下来写进 ID3）
    let cover_size = u32_le(&buf, &mut pos)? as usize;
    let cover = if cover_size > 0 {
        Some(take(&buf, &mut pos, cover_size)?.to_vec())
    } else {
        None
    };

    // 7. 音频流 RC4 解密
    let mut audio = buf[pos..].to_vec();
    for i in 0..audio.len() {
        let j = (i + 1) & 0xff;
        let a = key_box[j] as usize;
        let b = key_box[(a + j) & 0xff] as usize;
        audio[i] ^= key_box[(a + b) & 0xff];
    }

    let format = detect_format(&audio, &metadata);
    Ok(Ncm { audio, format, metadata, cover })
}

fn build_key_box(key: &[u8]) -> [u8; 256] {
    let mut key_box = [0u8; 256];
    for (i, v) in key_box.iter_mut().enumerate() {
        *v = i as u8;
    }
    let mut last: u8 = 0;
    let mut offset = 0usize;
    for i in 0..256 {
        let swap = key_box[i];
        let c = swap.wrapping_add(last).wrapping_add(key[offset]);
        offset = (offset + 1) % key.len();
        key_box[i] = key_box[c as usize];
        key_box[c as usize] = swap;
        last = c;
    }
    key_box
}

/// 以文件头为准，元数据的 format 字段仅作兜底。
/// 原实现反过来优先信元数据，且把 `ID3` 误写成 `IDV`。
fn detect_format(audio: &[u8], metadata: &Option<serde_json::Value>) -> String {
    if audio.len() >= 4 {
        if &audio[..3] == b"ID3" || (audio[0] == 0xFF && (audio[1] & 0xE0) == 0xE0) {
            return "mp3".into();
        }
        if &audio[..4] == b"fLaC" {
            return "flac".into();
        }
        if &audio[..4] == b"RIFF" {
            return "wav".into();
        }
    }
    metadata
        .as_ref()
        .and_then(|m| m["format"].as_str())
        .unwrap_or("mp3")
        .to_string()
}
