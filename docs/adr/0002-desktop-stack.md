# ADR-0002：桌面端采用 Tauri，目标平台 macOS + Windows

- 状态：**已接受（含未决风险，见下）**
- 日期：2026-07-26

## 背景

现有实现是 Python 命令行脚本。需求要求做成桌面应用，且需要跨平台分发（macOS + Windows），发给非本人使用。

候选方案与代价：

| 方案 | 优势 | 代价 |
| --- | --- | --- |
| Python + pywebview/Flask | 现有代码零改动，最快跑起来 | PyInstaller 打包产物体积大且脆弱，跨平台分发到他人机器上易失败 |
| Tauri (Rust) | 产物约 10MB，启动快，原生感强 | 全部逻辑需 Rust 实现 |
| Electron (TS) | 生态最成熟，轮子最全 | 产物 100MB+ 起步，内存占用高 |

## 决策

采用 **Tauri**。目标平台 macOS 与 Windows。

## 关键风险：要重写的不是解密，是请求加密

一个容易误判的点：选 Tauri 后需要用 Rust 重建的**不是** NCM 解密那 150 行。按 ADR-0001，那部分已不在主链路上。

真正需要在 Rust 侧从零建立的是网易云的**请求加密层**——社区逆向记录中，weapi 走两段 AES-CBC 加密请求参数并用 RSA 包裹会话密钥，eapi 走 AES-ECB 并附带摘要校验。

问题在于：Node 有 NeteaseCloudMusicApi、Python 有 pyncm 这类持续维护的成熟实现，**Rust 生态中没有同等量级的对应物**。这是本项目最大的单点技术风险，其实现路径见 ADR-0006（待写）。

## 跨平台带来的必办事项

选择跨平台分发后，以下不再是可选项：

- **macOS 门禁与公证**：未签名未公证的 `.app` 在他人机器上双击会提示「已损坏」。需要 Apple Developer 账号（付费）。
- **Windows 打包与签名**：未签名的 `.exe` 会触发 SmartScreen 警告。
- **凭据存储**：macOS Keychain 与 Windows Credential Manager 是两套 API，需统一抽象（见 ADR-0004，待写）。
- **路径与文件管理器集成**：「打开目录」与「定位到文件」在两个平台上是不同命令，且语义需区分（见 [glossary](../glossary.md) 中「Reveal」条目）。
- **二进制依赖分发**：若最终需要内置 ffmpeg，需为 macOS arm64、macOS x64、Windows x64 三个目标各备一份。这正是 ADR-0003 试图消除的负担。

## 后果

- 产物体积与启动速度显著优于 Electron，符合「工具型小应用」的定位
- 开发者需要能读写 Rust
- 无法直接复用现有 Python 代码，`ncm_to_mp3.py` 的解密逻辑需移植并**逐字节比对验证**（用 `target/` 下现有样本与 `output/` 下已知正确产物做回归）
- 项目从单文件脚本变为需要构建工具链的工程，仓库结构需重组
