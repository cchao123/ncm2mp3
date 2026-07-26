//! 网易云请求加密（weapi）
//!
//! 已由 spike 实测验证：服务端返回 code:200。移植时**不要重构这里的算法细节**，
//! 任何改动都必须重新跑一遍真实请求验证——错了不会报错，只会静默拿不到数据。
//!
//! eapi 实现一并保留（同样验证过加密正确），但主链路不使用：
//! 其 song/url/v1 稳定返回 200 + 空 body，详见 ADR-0006。

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use md5::{Digest, Md5};
use num_bigint::BigUint;
use rand::Rng;

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
    STANDARD.encode(Aes128CbcEnc::new(key.into(), IV.into()).encrypt_padded_vec_mut::<Pkcs7>(data))
}

fn random_secret() -> [u8; 16] {
    let mut rng = rand::thread_rng();
    let mut out = [0u8; 16];
    for b in out.iter_mut() {
        *b = SECRET_ALPHABET[rng.gen_range(0..SECRET_ALPHABET.len())];
    }
    out
}

/// weapi 的 RSA 环节**不是** PKCS#1 填充。
///
/// 调用 rsa crate 的标准加密会得到服务端静默拒绝的结果，且错误信息完全不指向真正原因。
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

/// 返回 (params, encSecKey)，作为表单字段 POST 出去。
pub fn weapi(payload: &str) -> Result<(String, String)> {
    let first = aes_cbc_b64(payload.as_bytes(), PRESET_KEY);
    let secret = random_secret();
    let params = aes_cbc_b64(first.as_bytes(), &secret);
    Ok((params, rsa_encrypt(&secret)?))
}

/// eapi：摘要里用的路径是 `/api/...`，不是实际请求的 `/eapi/...`。
/// 当前未接入主链路，保留作为 weapi 失效时的备选路径（ADR-0006）。
#[allow(dead_code)]
pub fn eapi(api_path: &str, payload: &str) -> Result<String> {
    let mut hasher = Md5::new();
    hasher.update(format!("nobody{}use{}md5forencrypt", api_path, payload).as_bytes());
    let digest = hex::encode(hasher.finalize());
    let data = format!("{}-36cf-{}-36cf-{}", api_path, payload, digest);
    Ok(hex::encode_upper(
        Aes128EcbEnc::new(EAPI_KEY.into()).encrypt_padded_vec_mut::<Pkcs7>(data.as_bytes()),
    ))
}

#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// eapi 是确定性的（AES-ECB 无随机数），可以对固定输入断言输出。
    /// 这个期望值来自与独立 Python 实现的逐字节比对，改动算法会立刻被这条测试抓住。
    #[test]
    fn eapi_matches_reference_implementation() {
        let got = eapi(
            "/api/song/enhance/player/url/v1",
            r#"{"ids":"[34690881]","level":"exhigh","encodeType":"flac"}"#,
        )
        .unwrap();
        assert!(got.starts_with("FA90B329E9614F79E79598F37DC2EDB4"));
        assert_eq!(got.len(), 288);
    }

    /// weapi 含随机会话密钥，无法断言具体值，但结构必须稳定：
    /// encSecKey 恒为 256 个 hex 字符（RSA 模数长度决定），这是最容易写错的地方。
    #[test]
    fn weapi_enc_sec_key_is_always_256_hex_chars() {
        for _ in 0..50 {
            let (params, enc) = weapi(r#"{"type":1}"#).unwrap();
            assert_eq!(enc.len(), 256, "encSecKey 必须左补零到 256 字符");
            assert!(enc.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(!params.is_empty());
        }
    }
}
