#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
网易云音乐 NCM 格式转换工具
基于博客园完整代码实现：https://www.cnblogs.com/cyx-b/p/13443003.html
"""

import binascii
import struct
import base64
import json
import os
from Crypto.Cipher import AES


def dump(file_path):
    """解密 NCM 文件"""
    core_key = binascii.a2b_hex("687A4852416D736F356B496E62617857")
    meta_key = binascii.a2b_hex("2331346C6A6B5F215C5D2630553C2728")
    unpad = lambda s: s[0:-(s[-1] if type(s[-1]) == int else ord(s[-1]))]

    f = open(file_path, 'rb')

    # 1. 读取并验证文件头
    header = f.read(8)
    assert binascii.b2a_hex(header) == b'4354454e4644414d'

    # 2. 跳过 2 字节 gap
    f.seek(2, 1)

    # 3. 读取密钥长度和密钥数据
    key_length = f.read(4)
    key_length = struct.unpack('<I', bytes(key_length))[0]
    key_data = f.read(key_length)

    # 4. XOR 0x64 解密密钥数据
    key_data_array = bytearray(key_data)
    for i in range(0, len(key_data_array)):
        key_data_array[i] ^= 0x64
    key_data = bytes(key_data_array)

    # 5. AES 解密
    cryptor = AES.new(core_key, AES.MODE_ECB)
    key_data = unpad(cryptor.decrypt(key_data))[17:]

    # 6. 构建密钥盒 (RC4-KSA)
    key_length = len(key_data)
    key_data = bytearray(key_data)
    key_box = bytearray(range(256))
    c = 0
    last_byte = 0
    key_offset = 0

    for i in range(256):
        swap = key_box[i]
        c = (swap + last_byte + key_data[key_offset]) & 0xff
        key_offset += 1
        if key_offset >= key_length:
            key_offset = 0
        key_box[i] = key_box[c]
        key_box[c] = swap
        last_byte = c

    # 7. 读取元数据长度和元数据
    meta_length = f.read(4)
    meta_length = struct.unpack('<I', bytes(meta_length))[0]
    meta_data = f.read(meta_length)

    # 8. XOR 0x63 解密元数据
    meta_data_array = bytearray(meta_data)
    for i in range(0, len(meta_data_array)):
        meta_data_array[i] ^= 0x63
    meta_data = bytes(meta_data_array)

    # 9. Base64 解码并去掉前缀
    meta_data = base64.b64decode(meta_data[22:])

    # 10. AES 解密元数据
    cryptor = AES.new(meta_key, AES.MODE_ECB)
    meta_data = unpad(cryptor.decrypt(meta_data)).decode('utf-8')[6:]

    # 11. 解析 JSON
    meta_data = json.loads(meta_data)

    # 12. 跳过 CRC32 和 gap
    crc32 = f.read(4)
    crc32 = struct.unpack('<I', bytes(crc32))[0]
    f.seek(5, 1)

    # 13. 读取图片
    image_size = f.read(4)
    image_size = struct.unpack('<I', bytes(image_size))[0]
    image_data = f.read(image_size)

    # 14. 构造输出文件名
    file_name = meta_data['musicName'] + '.' + meta_data['format']
    output_path = os.path.join(os.path.split(file_path)[0], file_name)

    # 15. 解密音频数据并写入文件
    m = open(output_path, 'wb')
    chunk = bytearray()

    while True:
        chunk = bytearray(f.read(0x8000))
        chunk_length = len(chunk)
        if not chunk:
            break
        for i in range(1, chunk_length + 1):
            j = i & 0xff
            chunk[i - 1] ^= key_box[(key_box[j] + key_box[(key_box[j] + j) & 0xff]) & 0xff]
        m.write(chunk)

    m.close()
    f.close()

    return output_path, meta_data


def file_extension(path):
    """获取文件扩展名"""
    return os.path.splitext(path)[1]


def process_file(file_path):
    """处理单个 NCM 文件"""
    try:
        print(f"正在处理: {os.path.basename(file_path)}")
        output_path, metadata = dump(file_path)
        print(f"✓ 成功: {output_path}")
        print(f"  歌曲: {metadata.get('musicName', 'Unknown')}")
        print(f"  艺术家: {metadata.get('artist', [['Unknown']])[0][0] if metadata.get('artist') else 'Unknown'}")
        print(f"  格式: {metadata.get('format', 'Unknown')}")
        return True
    except Exception as e:
        print(f"✗ 失败: {e}")
        import traceback
        traceback.print_exc()
        return False


def main():
    """主函数"""
    import sys

    if len(sys.argv) < 2:
        print("网易云音乐 NCM 格式转换工具")
        print()
        print("用法:")
        print(f"  {sys.argv[0]} <NCM文件或目录>")
        print()
        print("示例:")
        print(f"  {sys.argv[0]} song.ncm")
        print(f"  {sys.argv[0]} /path/to/ncm/folder")
        sys.exit(1)

    input_path = sys.argv[1]

    if os.path.isfile(input_path):
        # 处理单个文件
        process_file(input_path)
    elif os.path.isdir(input_path):
        # 批量处理目录
        success_count = 0
        total_count = 0

        for root, dirs, files in os.walk(input_path):
            for file in files:
                if file_extension(file) == ".ncm":
                    total_count += 1
                    file_path = os.path.join(root, file)
                    if process_file(file_path):
                        success_count += 1
                    print()

        print("=" * 50)
        print(f"处理完成: {success_count}/{total_count} 成功")
    else:
        print(f"错误: 路径不存在 - {input_path}")
        sys.exit(1)


if __name__ == '__main__':
    main()