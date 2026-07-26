//! 凭据存储（ADR-0004）
//!
//! MUSIC_U 等价于账号本身，因此存进操作系统凭据管理器：
//! macOS → Keychain，Windows → Credential Manager。
//!
//! 约束（ADR-0004）：
//!   - 凭据只在发起请求时读取，不长期驻留内存
//!   - **绝不写入日志**——本模块的错误信息一律不含凭据值
//!   - 「退出登录」必须真正删除条目，而不只是清内存

use anyhow::{anyhow, Result};

const SERVICE: &str = "com.cchao.ncm-desktop";
const ENTRY_MUSIC_U: &str = "MUSIC_U";
const ENTRY_CSRF: &str = "__csrf";

fn entry(name: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, name).map_err(|e| anyhow!("打开钥匙串条目失败: {e}"))
}

pub fn save(music_u: &str, csrf: &str) -> Result<()> {
    entry(ENTRY_MUSIC_U)?
        .set_password(music_u)
        .map_err(|e| anyhow!("写入钥匙串失败: {e}"))?;
    entry(ENTRY_CSRF)?
        .set_password(csrf)
        .map_err(|e| anyhow!("写入钥匙串失败: {e}"))?;
    Ok(())
}

/// 返回 (MUSIC_U, csrf)。未登录时返回 None，不视为错误。
pub fn load() -> Option<(String, String)> {
    let music_u = entry(ENTRY_MUSIC_U).ok()?.get_password().ok()?;
    if music_u.is_empty() {
        return None;
    }
    let csrf = entry(ENTRY_CSRF)
        .ok()
        .and_then(|e| e.get_password().ok())
        .unwrap_or_default();
    Some((music_u, csrf))
}

/// 真正删除钥匙串条目。条目本就不存在时视为成功——退出登录应当幂等。
pub fn clear() -> Result<()> {
    for name in [ENTRY_MUSIC_U, ENTRY_CSRF] {
        if let Ok(e) = entry(name) {
            match e.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(err) => return Err(anyhow!("删除钥匙串条目失败: {err}")),
            }
        }
    }
    Ok(())
}
