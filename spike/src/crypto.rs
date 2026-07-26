//! 网易云请求加密层
//!
//! ADR-0006 标记的项目最高风险点。两套加密约定：
//!   weapi —— 两段 AES-128-CBC，会话密钥用 RSA 包裹
//!   eapi  —— AES-128-ECB，请求体带 md5 摘要校验
//!
//! RSA 那段不是标准填充，见 `rsa_encrypt` 的注释。

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use md5::{Digest, Md5};
use num_bigint::BigUint;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128EcbEnc = ecb::Encryptor<aes::Aes128>;
type Aes128EcbDec = ecb::Decryptor<aes::Aes128>;

const PRESET_KEY: &[u8; 16] = b"0CoJUm6Qyw8W8jud";
const IV: &[u8; 16] = b"0102030405060708";
const EAPI_KEY: &[u8; 16] = b"e82ckenh8dichen8";

const RSA_MODULUS: &str = "00e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b7251\
52b3ab17a876aea8a5aa76d2e417629ec4ee341f56135fccf695280104e0312ecbda92557c93870114af6c9d05c4f7f0\
c3685b7a46bee255932575cce10b424d813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3ece0462db0a22b8e7";
const RSA_EXPONENT: &str = "010001";

const SECRET_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

fn aes_cbc_b64(data: &[u8], key: &[u8; 16]) -> String {
    let ct = Aes128CbcEnc::new(key.into(), IV.into()).encrypt_padded_vec_mut::<Pkcs7>(data);
    STANDARD.encode(ct)
}

/// 从 /dev/urandom 取 16 字节，映射到字母表。
/// 不引 rand crate 是为了避开 0.8/0.9 的 API 断层——spike 不该为依赖版本浪费时间。
///
/// 必须用 read_exact 而非 fs::read：后者一路读到 EOF，而字符设备永不返回 EOF，
/// 进程会无限分配内存直到被 OOM 杀掉（表现为 SIGKILL/137，极易误判成网络或沙箱问题）。
fn random_secret() -> Result<Vec<u8>> {
    let raw = read_urandom_16()
        .or_else(|_| -> Result<Vec<u8>> {
            // 极端兜底：这里的随机性不是安全关键，服务端只用它解密本次请求
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .subsec_nanos();
            Ok((0..16u32).map(|i| (nanos.wrapping_mul(i + 7) >> 3) as u8).collect())
        })?;
    Ok(raw
        .iter()
        .map(|b| SECRET_ALPHABET[*b as usize % SECRET_ALPHABET.len()])
        .collect())
}

/// weapi 的 RSA 环节**不是** PKCS#1 填充。
///
/// 直接调 rsa crate 的标准加密会得到服务端静默拒绝的结果，且错误信息完全不指向真正原因——
/// ADR-0006 说的"最可能卡死人的单点"就是这里。
///
/// 正确做法：密钥字符串反转 → 按字节取 hex → 当作大整数 → 直接模幂 → 左补零至 256 个 hex 字符。
fn rsa_encrypt(secret: &[u8]) -> Result<String> {
    let reversed: Vec<u8> = secret.iter().rev().copied().collect();
    let m = BigUint::parse_bytes(hex::encode(&reversed).as_bytes(), 16)
        .ok_or_else(|| anyhow!("会话密钥转大整数失败"))?;
    let e = BigUint::parse_bytes(RSA_EXPONENT.as_bytes(), 16)
        .ok_or_else(|| anyhow!("RSA 指数解析失败"))?;
    let n = BigUint::parse_bytes(RSA_MODULUS.as_bytes(), 16)
        .ok_or_else(|| anyhow!("RSA 模数解析失败"))?;
    Ok(format!("{:0>256}", m.modpow(&e, &n).to_str_radix(16)))
}

/// 返回 (params, encSecKey)，作为 form 字段 POST 出去。
fn read_urandom_16() -> Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.to_vec())
}

pub fn weapi(payload: &str) -> Result<(String, String)> {
    let first = aes_cbc_b64(payload.as_bytes(), PRESET_KEY);
    let secret = random_secret()?;
    let key: &[u8; 16] = secret
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("会话密钥长度不是 16"))?;
    let params = aes_cbc_b64(first.as_bytes(), key);
    Ok((params, rsa_encrypt(&secret)?))
}

/// eapi：摘要里用的路径是 `/api/...`，不是实际请求的 `/eapi/...`。
/// 写成一样的会得到一个毫无线索的失败。
pub fn eapi(api_path: &str, payload: &str) -> Result<String> {
    let mut hasher = Md5::new();
    hasher.update(format!("nobody{}use{}md5forencrypt", api_path, payload).as_bytes());
    let digest = hex::encode(hasher.finalize());

    let data = format!("{}-36cf-{}-36cf-{}", api_path, payload, digest);
    let ct = Aes128EcbEnc::new(EAPI_KEY.into()).encrypt_padded_vec_mut::<Pkcs7>(data.as_bytes());
    Ok(hex::encode_upper(ct))
}

/// eapi 响应可能是密文，也可能直接是 JSON。先按明文试，失败再解密。
pub fn eapi_decrypt_response(body: &[u8]) -> Result<String> {
    if let Ok(s) = std::str::from_utf8(body) {
        let t = s.trim_start();
        if t.starts_with('{') || t.starts_with('[') {
            return Ok(s.to_string());
        }
    }
    let pt = Aes128EcbDec::new(EAPI_KEY.into())
        .decrypt_padded_vec_mut::<Pkcs7>(body)
        .map_err(|e| anyhow!("eapi 响应解密失败: {e}"))?;
    Ok(String::from_utf8(pt)?)
}
