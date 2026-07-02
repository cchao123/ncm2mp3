# NCM2MP3

网易云音乐 NCM 格式转 MP3 工具

一个用于将网易云音乐下载得到的 `.ncm` 加密音频文件转换为通用音频格式的命令行工具。项目内置 Python 解密脚本，支持单个文件转换和目录批量转换；当源音频不是 MP3 时，可借助 FFmpeg 转码为 MP3。

> 本项目仅供个人学习、研究和备份合法拥有的音频文件使用，请遵守相关版权法律法规。

## 功能特性

- 支持 `.ncm` 文件解密
- 支持单文件转换
- 支持目录批量转换
- 自动读取歌曲元数据并生成文件名
- 支持 MP3、FLAC、WAV 等 NCM 内部音频格式
- 可指定输出目录
- 可指定 MP3 转码比特率

## 项目结构

```text
.
├── ncm_to_mp3.py       # 主转换脚本，推荐使用
├── ncm_decrypt.py      # 简化版 NCM 解密脚本
├── install_and_run.sh  # 交互式安装和运行脚本
├── 批量转换.sh          # 基于 ncmdump 的批量转换脚本
├── target/             # 可放置待转换的 NCM 文件
└── output/             # 默认输出目录
```

## 环境要求

- Python 3.7+
- `pycryptodome`
- FFmpeg，可选：当 NCM 内部音频不是 MP3，而你希望输出 MP3 时需要

安装 Python 依赖：

```bash
pip3 install pycryptodome
```

安装 FFmpeg：

```bash
# macOS
brew install ffmpeg

# Ubuntu / Debian
sudo apt install ffmpeg
```

## 快速开始

### 1. 转换单个文件

```bash
python3 ncm_to_mp3.py target/song.ncm
```

默认会把转换结果保存到 `output/` 目录。

### 2. 指定输出目录

```bash
python3 ncm_to_mp3.py target/song.ncm my_output
```

### 3. 指定 MP3 比特率

```bash
python3 ncm_to_mp3.py target/song.ncm output 320k
```

支持的常用比特率：

```text
128k, 192k, 256k, 320k
```

### 4. 批量转换目录

```bash
python3 ncm_to_mp3.py target
```

脚本会递归查找目录中的 `.ncm` 文件，并将结果输出到 `output/`。

## 命令格式

```bash
python3 ncm_to_mp3.py <NCM文件或目录> [输出目录] [比特率]
```

参数说明：

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `NCM文件或目录` | 是 | 无 | 要转换的 `.ncm` 文件，或包含 `.ncm` 文件的目录 |
| `输出目录` | 否 | `output` | 转换后文件保存目录 |
| `比特率` | 否 | `192k` | 非 MP3 音频转码为 MP3 时使用的比特率 |

## 交互式运行

如果你不想手动输入完整命令，可以运行：

```bash
chmod +x install_and_run.sh
./install_and_run.sh
```

脚本会检查 Python、安装依赖，并提示你输入待转换文件或目录。

## 使用 ncmdump 批量转换

项目还提供了 `批量转换.sh`，它使用第三方 `ncmdump` 命令进行转换。

先安装：

```bash
pip3 install ncmdump
```

然后运行：

```bash
chmod +x 批量转换.sh
./批量转换.sh target output
```

注意：`批量转换.sh` 中的 `NCM_CMD` 默认写死为本机路径：

```bash
/Users/cchao/Library/Python/3.9/bin/ncmdump
```

如果你的 `ncmdump` 安装在其他位置，请根据实际情况修改脚本中的 `NCM_CMD`。

## 输出说明

转换完成后，文件会保存到指定输出目录。例如：

```text
output/
├── 歌曲名 - 歌手名.mp3
└── ...
```

如果 NCM 内部本身就是 MP3，脚本会直接解密输出；如果是 FLAC、WAV 等格式，脚本会尝试使用 FFmpeg 转为 MP3。

## 常见问题

### 提示缺少 pycryptodome 怎么办？

运行：

```bash
pip3 install pycryptodome
```

### 转换 FLAC 时没有生成 MP3 怎么办？

请先安装 FFmpeg，并确认命令行中可以执行：

```bash
ffmpeg -version
```

### 可以保留原音质吗？

如果 NCM 内部音频本身是 MP3，转换过程是解密输出，不会重新编码。如果内部是 FLAC/WAV，再转为 MP3 时会发生有损压缩；可以使用 `320k` 获得较高的 MP3 输出质量。

### 可以转换整个文件夹吗？

可以：

```bash
python3 ncm_to_mp3.py /path/to/ncm_folder output 320k
```

## 免责声明

本工具仅用于技术学习和个人研究。请确保你拥有相关音频文件的合法使用权，不要将本工具用于传播、售卖或其他侵犯版权的行为。因使用本工具产生的任何法律风险由使用者自行承担。

## License

MIT License
