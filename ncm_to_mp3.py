#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
网易云音乐 NCM 格式转 MP3 工具
参考项目：https://github.com/Majjcom/ncmDumper
         https://github.com/haojiezhe12345/ncmCacheDump
详细文档：https://www.cnblogs.com/cyx-b/p/13443003.html
"""

import os
import sys
import struct
import base64
import json
import binascii
from pathlib import Path
from typing import Optional, Dict

try:
    from Crypto.Cipher import AES
except ImportError:
    print("错误: 缺少 pycryptodome 库")
    print("请运行: pip install pycryptodome")
    sys.exit(1)


class NCMDecryptor:
    """NCM 文件解密器"""

    # 固定密钥 (十六进制转二进制)
    CORE_KEY = binascii.a2b_hex("687A4852416D736F356B496E62617857")
    META_KEY = binascii.a2b_hex("2331346C6A6B5F215C5D2630553C2728")

    # 文件头魔术
    MAGIC = b'CTENFDAM'

    def __init__(self, file_path: str):
        self.file_path = file_path
        self.metadata = None

    @staticmethod
    def unpad(s: bytes) -> bytes:
        """去除 PKCS7 填充"""
        return s[:-(s[-1] if isinstance(s[-1], int) else ord(s[-1]))]

    def decrypt(self) -> tuple[bytes, str, Optional[Dict]]:
        """解密 NCM 文件，返回 (音频数据, 格式, 元数据)"""
        with open(self.file_path, 'rb') as f:
            # 1. 读取文件头 (8字节)
            header = f.read(8)
            if header != self.MAGIC:
                raise ValueError(f"无效的 NCM 文件: {self.file_path}")

            # 2. 跳过 gap (2字节)
            f.seek(2, 1)

            # 3. 读取密钥长度 (4字节)
            key_length_data = f.read(4)
            key_length = struct.unpack('<I', key_length_data)[0]

            # 4. 读取加密密钥并 XOR 0x64
            key_data = f.read(key_length)
            key_data_array = bytearray(key_data)
            for i in range(len(key_data_array)):
                key_data_array[i] ^= 0x64
            key_data = bytes(key_data_array)

            # 5. AES 解密密钥数据
            cryptor = AES.new(self.CORE_KEY, AES.MODE_ECB)
            key_data = self.unpad(cryptor.decrypt(key_data))[17:]

            # 6. 构建 RC4 密钥盒 (标准 RC4-KSA 算法)
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

            # 7. 读取元数据长度
            meta_length_data = f.read(4)
            meta_length = struct.unpack('<I', meta_length_data)[0]

            # 8. 读取并解密元数据
            metadata = None
            if meta_length > 0:
                meta_data = f.read(meta_length)

                # XOR 0x63
                meta_data_array = bytearray(meta_data)
                for i in range(len(meta_data_array)):
                    meta_data_array[i] ^= 0x63
                meta_data = bytes(meta_data_array)

                # 去掉前缀 "163 key(Don't modify):" (22字节)
                meta_data = base64.b64decode(meta_data[22:])

                # AES 解密
                cryptor = AES.new(self.META_KEY, AES.MODE_ECB)
                meta_data = self.unpad(cryptor.decrypt(meta_data))

                # 转换为字符串并去掉 "music:" 前缀 (6字节)
                json_str = meta_data.decode('utf-8')[6:]

                # 解析 JSON
                self.metadata = json.loads(json_str)
                metadata = self.metadata

            # 9. 跳过 CRC32 (4字节) 和 gap (5字节)
            f.seek(5, 1)

            # 10. 读取图片大小和图片数据
            image_size_data = f.read(4)
            image_size = struct.unpack('<I', image_size_data)[0]
            if image_size > 0:
                f.read(image_size)  # 跳过图片数据

            # 11. 读取加密音频数据
            encrypted_audio = f.read()

            # 12. 使用 RC4 PRGA 算法解密音频
            audio_data = bytearray()
            chunk_size = 0x8000  # 32KB

            for offset in range(0, len(encrypted_audio), chunk_size):
                chunk = bytearray(encrypted_audio[offset:offset + chunk_size])
                chunk_length = len(chunk)

                # RC4 解密：每个字节与密钥盒异或
                for i in range(1, chunk_length + 1):
                    j = i & 0xff
                    chunk[i - 1] ^= key_box[(key_box[j] + key_box[(key_box[j] + j) & 0xff]) & 0xff]

                audio_data.extend(chunk)

            # 13. 检测音频格式
            audio_format = self.detect_format(bytes(audio_data), metadata)

            return bytes(audio_data), audio_format, metadata

    def detect_format(self, audio_data: bytes, metadata: Optional[Dict]) -> str:
        """检测音频格式"""
        # 从元数据获取格式
        if metadata and 'format' in metadata:
            return metadata['format']

        # 通过文件头检测
        if audio_data[:3] == b'IDV' or audio_data[:2] == b'\xff\xfb':
            return 'mp3'
        elif audio_data[:4] == b'fLaC':
            return 'flac'
        elif audio_data[:4] == b'RIFF':
            return 'wav'
        else:
            return 'mp3'  # 默认


def convert_to_mp3(input_file: str, output_file: str, bitrate: str = '192k') -> bool:
    """使用 ffmpeg 将音频转换为 MP3"""
    try:
        # 检查 ffmpeg 是否可用
        result = subprocess.run(['ffmpeg', '-version'],
                              capture_output=True,
                              timeout=5)
        if result.returncode != 0:
            raise FileNotFoundError
    except (FileNotFoundError, subprocess.TimeoutExpired):
        print("警告: 未找到 ffmpeg，尝试直接复制文件...")
        try:
            import shutil
            shutil.copy2(input_file, output_file)
            return True
        except Exception as e:
            print(f"文件复制失败: {e}")
            return False

    try:
        import subprocess
        cmd = [
            'ffmpeg',
            '-i', input_file,
            '-b:a', bitrate,
            '-y',  # 覆盖输出文件
            output_file
        ]

        result = subprocess.run(cmd,
                              capture_output=True,
                              timeout=300)

        if result.returncode == 0:
            # 成功转换，删除原文件
            os.remove(input_file)
            return True
        else:
            print(f"ffmpeg 转换失败: {result.stderr.decode()}")
            return False
    except subprocess.TimeoutExpired:
        print("转换超时")
        return False
    except Exception as e:
        print(f"转换过程出错: {e}")
        return False


def get_song_name(metadata: Optional[Dict], file_path: str) -> str:
    """从元数据或文件名获取歌曲名称"""
    if metadata:
        try:
            name = metadata.get('musicName', '')
            artists = metadata.get('artist', [])
            if artists:
                # artist 可能是嵌套列表或简单列表
                if isinstance(artists[0], list):
                    artist_name = artists[0][0] if artists[0] else '未知艺术家'
                else:
                    artist_name = artists[0] if isinstance(artists[0], str) else '未知艺术家'
            else:
                artist_name = '未知艺术家'

            if name:
                # 清理文件名中的非法字符
                clean_name = f"{name} - {artist_name}"
                invalid_chars = '<>:"/\\|?*'
                for char in invalid_chars:
                    clean_name = clean_name.replace(char, '_')
                return clean_name
        except Exception as e:
            print(f"解析元数据失败: {e}")

    # 使用原文件名
    return Path(file_path).stem


def process_ncm_file(ncm_path: str, output_dir: str = 'output', bitrate: str = '192k') -> bool:
    """处理单个 NCM 文件"""
    try:
        print(f"正在处理: {os.path.basename(ncm_path)}")

        # 创建解密器
        decryptor = NCMDecryptor(ncm_path)

        # 解密文件
        audio_data, audio_format, metadata = decryptor.decrypt()

        # 创建输出目录
        os.makedirs(output_dir, exist_ok=True)

        # 获取歌曲名称
        song_name = get_song_name(metadata, ncm_path)

        # 保存解密后的音频
        temp_file = os.path.join(output_dir, f"{song_name}.{audio_format}")

        with open(temp_file, 'wb') as f:
            f.write(audio_data)

        print(f"  ✓ 解密完成: {temp_file}")

        # 如果不是 MP3 格式，尝试转换
        if audio_format.lower() != 'mp3':
            mp3_file = os.path.join(output_dir, f"{song_name}.mp3")
            print(f"  正在转换为 MP3...")
            if convert_to_mp3(temp_file, mp3_file, bitrate):
                print(f"  ✓ 转换成功: {mp3_file}")
                return True
            else:
                print(f"  ⚠ 转换失败，保留原格式: {temp_file}")
                return False
        else:
            return True

    except Exception as e:
        print(f"✗ 处理文件 {ncm_path} 时出错: {e}")
        import traceback
        traceback.print_exc()
        return False


def batch_process(input_dir: str, output_dir: str = 'output', bitrate: str = '192k'):
    """批量处理目录中的所有 NCM 文件"""
    input_path = Path(input_dir)

    if not input_path.exists():
        print(f"错误: 目录不存在 - {input_dir}")
        return

    # 查找所有 NCM 文件
    ncm_files = list(input_path.rglob('*.ncm'))

    if not ncm_files:
        print(f"未找到 NCM 文件 in {input_dir}")
        return

    print(f"找到 {len(ncm_files)} 个 NCM 文件")
    print("=" * 50)

    success_count = 0
    for ncm_file in ncm_files:
        if process_ncm_file(str(ncm_file), output_dir, bitrate):
            success_count += 1
        print()

    print("=" * 50)
    print(f"处理完成: {success_count}/{len(ncm_files)} 成功")


def main():
    """主函数"""
    if len(sys.argv) < 2:
        print("网易云音乐 NCM 转 MP3 工具")
        print()
        print("用法:")
        print(f"  {sys.argv[0]} <NCM文件或目录> [输出目录] [比特率]")
        print()
        print("示例:")
        print(f"  {sys.argv[0]} song.ncm")
        print(f"  {sys.argv[0]} song.ncm output 320k")
        print(f"  {sys.argv[0]} /path/to/ncm/folder")
        print()
        print("比特率选项: 128k, 192k, 256k, 320k (默认: 192k)")
        sys.exit(1)

    input_path = sys.argv[1]
    output_dir = sys.argv[2] if len(sys.argv) > 2 else 'output'
    bitrate = sys.argv[3] if len(sys.argv) > 3 else '192k'

    if os.path.isfile(input_path):
        # 处理单个文件
        process_ncm_file(input_path, output_dir, bitrate)
    elif os.path.isdir(input_path):
        # 批量处理
        batch_process(input_path, output_dir, bitrate)
    else:
        print(f"错误: 路径不存在 - {input_path}")
        sys.exit(1)


if __name__ == '__main__':
    main()