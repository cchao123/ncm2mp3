//! 文件名清洗（ADR-0008）
//!
//! 目录平铺，区分度全部由文件名承载，所以清洗规则的严格程度直接决定输出库可不可用。
//!
//! 统一按 **Windows 规则**清洗——它比 macOS 严格得多。同一份代码在两个平台上
//! 必须产出**相同**的文件名，否则同一首歌在两台机器上会得到不同路径，
//! SQLite 索引里的记录随之失去可移植性。
//!
//! 原实现 `ncm_to_mp3.py:243` 有个洞：无元数据时直接用原始文件名兜底，跳过全部清洗。
//! 这里的设计前提是**清洗是落盘路径上的唯一出口**，不存在绕过分支。

/// Windows 保留设备名。即便加了扩展名，`CON.mp3` 在 Windows 上依然无法创建。
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 单个文件名的字节上限（不含扩展名）。
/// 取值偏保守：多数文件系统限制单个组件 255 字节，中文按 UTF-8 占 3 字节。
const MAX_STEM_BYTES: usize = 180;

/// 把任意字符串清洗成两个平台都能安全落盘的文件名主干。
pub fn sanitize_stem(input: &str) -> String {
    let mut s: String = input
        .chars()
        .map(|c| match c {
            // Windows 禁用字符（macOS 只禁 / 和 :）
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            // 控制字符
            c if (c as u32) < 0x20 || c as u32 == 0x7F => '_',
            c => c,
        })
        .collect();

    // Windows 会静默剥掉末尾的空格和句点，导致实际落盘名与索引记录不一致
    s = s.trim().trim_end_matches('.').trim().to_string();

    s = truncate_bytes(&s, MAX_STEM_BYTES);
    // 截断可能又露出末尾空格/句点
    s = s.trim().trim_end_matches('.').trim().to_string();

    if s.is_empty() {
        return "未命名".to_string();
    }

    // 保留名判定不区分大小写，且要看扩展名之前的部分
    let head = s.split('.').next().unwrap_or(&s).to_ascii_uppercase();
    if RESERVED.contains(&head.as_str()) {
        return format!("_{s}");
    }
    s
}

/// 按 UTF-8 字节数截断，不切断多字节字符。
fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// 生成「艺术家 - 歌名」形式的文件名主干。
pub fn track_stem(artists: &str, title: &str) -> String {
    let a = artists.trim();
    let t = title.trim();
    sanitize_stem(&if a.is_empty() {
        t.to_string()
    } else {
        format!("{a} - {t}")
    })
}

/// 目录平铺时的重名消歧：确定性地追加 ` (2)`、` (3)`……
///
/// 必须确定性——随机或时间戳后缀会让同一首歌重下时产生新文件而非覆盖，
/// 与 ADR-0009 的去重语义冲突。
pub fn dedupe_path(dir: &std::path::Path, stem: &str, ext: &str) -> std::path::PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    for n in 2..10_000 {
        let p = dir.join(format!("{stem} ({n}).{ext}"));
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("{stem} ({}).{ext}", "many"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_windows_forbidden_chars() {
        assert_eq!(sanitize_stem(r#"AC/DC: Back<In>Black"#), "AC_DC_ Back_In_Black");
    }

    #[test]
    fn keeps_chinese_and_common_punctuation() {
        assert_eq!(sanitize_stem("周杰伦 - 七里香 (Live)"), "周杰伦 - 七里香 (Live)");
    }

    #[test]
    fn strips_trailing_space_and_dot() {
        // Windows 会静默剥掉这些，留着会导致索引与磁盘不一致
        assert_eq!(sanitize_stem("歌名... "), "歌名");
        assert_eq!(sanitize_stem("  歌名  "), "歌名");
    }

    #[test]
    fn escapes_windows_reserved_device_names() {
        assert_eq!(sanitize_stem("CON"), "_CON");
        assert_eq!(sanitize_stem("con"), "_con");
        assert_eq!(sanitize_stem("COM1"), "_COM1");
        // 非保留名不应被误伤
        assert_eq!(sanitize_stem("CONCERT"), "CONCERT");
    }

    #[test]
    fn never_returns_empty() {
        assert_eq!(sanitize_stem(""), "未命名");
        assert_eq!(sanitize_stem("   "), "未命名");
        assert_eq!(sanitize_stem("///"), "___");
    }

    #[test]
    fn truncates_without_splitting_multibyte_chars() {
        let long = "很".repeat(200); // 600 字节
        let out = sanitize_stem(&long);
        assert!(out.len() <= MAX_STEM_BYTES);
        assert!(out.chars().all(|c| c == '很')); // 没有产生半个字符
    }

    #[test]
    fn track_stem_handles_missing_artist() {
        assert_eq!(track_stem("", "孤勇者"), "孤勇者");
        assert_eq!(track_stem("陈奕迅", "孤勇者"), "陈奕迅 - 孤勇者");
    }
}
