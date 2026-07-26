#!/bin/bash
# 网易云音乐 NCM 批量转换工具
# 使用官方 ncmdump 库，导出到 output 目录

NCM_CMD="/Users/cchao/Library/Python/3.9/bin/ncmdump"
OUTPUT_DIR="output"

if [ ! -f "$NCM_CMD" ]; then
    echo "错误: ncmdump 未安装"
    echo "请运行: pip3 install ncmdump"
    exit 1
fi

if [ -z "$1" ]; then
    echo "网易云音乐 NCM 批量转换工具"
    echo ""
    echo "用法:"
    echo "  $0 <NCM文件或目录> [输出目录]"
    echo ""
    echo "示例:"
    echo "  $0 song.ncm"
    echo "  $0 song.ncm my_output"
    echo "  $0 /path/to/ncm/folder"
    echo ""
    echo "默认输出目录: output/"
    exit 1
fi

INPUT_PATH="$1"

# 如果提供了第二个参数，使用它作为输出目录
if [ -n "$2" ]; then
    OUTPUT_DIR="$2"
fi

# 创建输出目录
mkdir -p "$OUTPUT_DIR"

echo "输出目录: $OUTPUT_DIR"
echo ""

if [ -f "$INPUT_PATH" ]; then
    echo "转换文件: $INPUT_PATH"
    "$NCM_CMD" -o "$OUTPUT_DIR" "$INPUT_PATH"

    if [ $? -eq 0 ]; then
        echo "✓ 完成"
        echo "输出文件: $OUTPUT_DIR/"
    else
        echo "✗ 转换失败"
        exit 1
    fi

elif [ -d "$INPUT_PATH" ]; then
    echo "批量转换目录: $INPUT_PATH"
    echo ""

    SUCCESS=0
    TOTAL=0

    # 查找所有 .ncm 文件
    while IFS= read -r -d '' file; do
        TOTAL=$((TOTAL + 1))
        echo "[$TOTAL] 转换: $(basename "$file")"

        if "$NCM_CMD" -o "$OUTPUT_DIR" "$file"; then
            SUCCESS=$((SUCCESS + 1))
            echo "  ✓ 成功"
        else
            echo "  ✗ 失败"
        fi
        echo ""
    done < <(find "$INPUT_PATH" -type f -name "*.ncm" -print0)

    echo "================================"
    echo "处理完成: $SUCCESS/$TOTAL 成功"
    echo "输出目录: $OUTPUT_DIR/"
else
    echo "错误: 路径不存在 - $INPUT_PATH"
    exit 1
fi
