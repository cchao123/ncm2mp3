#!/bin/bash
# 网易云音乐 NCM 转 MP3 工具 - 快速启动脚本

echo "网易云音乐 NCM 转 MP3 工具"
echo "======================"
echo ""

# 检查 Python
if ! command -v python3 &> /dev/null; then
    echo "❌ 错误: 未找到 Python 3"
    echo "请先安装 Python 3.7 或更高版本"
    exit 1
fi

echo "✅ Python 已安装"

# 检查 pycryptodome
if ! python3 -c "import Crypto" 2>/dev/null; then
    echo "📦 正在安装 pycryptodome..."
    pip3 install pycryptodome
fi

echo "✅ 依赖库已安装"
echo ""

# 检查 FFmpeg（可选）
if command -v ffmpeg &> /dev/null; then
    echo "✅ FFmpeg 已安装（支持格式转换）"
else
    echo "⚠️  FFmpeg 未安装（可选，用于格式转换）"
    echo "   macOS: brew install ffmpeg"
    echo "   其他系统: 访问 https://ffmpeg.org/download.html"
fi

echo ""
echo "使用方法:"
echo "--------"
echo "1. 转换单个文件:"
echo "   python3 ncm_to_mp3.py song.ncm"
echo ""
echo "2. 批量转换目录:"
echo "   python3 ncm_to_mp3.py /path/to/ncm/folder"
echo ""
echo "3. 指定输出目录和比特率:"
echo "   python3 ncm_to_mp3.py song.ncm output 320k"
echo ""

# 询问是否继续
read -p "是否要开始转换? (y/n): " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    read -p "请输入 NCM 文件或目录路径: " input_path
    if [ -f "$input_path" ] || [ -d "$input_path" ]; then
        read -p "输出目录 (默认: output): " output_dir
        output_dir=${output_dir:-output}
        read -p "比特率 (默认: 192k): " bitrate
        bitrate=${bitrate:-192k}

        echo ""
        echo "开始转换..."
        python3 ncm_to_mp3.py "$input_path" "$output_dir" "$bitrate"
    else
        echo "❌ 路径不存在: $input_path"
    fi
fi

echo ""
echo "完成！"
