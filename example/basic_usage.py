#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Funget 基本使用示例
演示如何使用 funget 进行文件下载和上传
"""

import os
from funget import download, multi_thread_download, simple_download
from funget.upload import single_upload


def example_basic_download():
    """基本下载示例"""
    print("=== 基本下载示例 ===")

    # 智能下载（自动选择单线程或多线程）
    url = "https://httpbin.org/bytes/1024"
    filepath = "./downloads/test_file.bin"

    print(f"下载文件: {url}")
    success = download(url, filepath)

    if success:
        print(f"✅ 下载成功: {filepath}")
        print(f"文件大小: {os.path.getsize(filepath)} 字节")
    else:
        print("❌ 下载失败")


def example_multi_thread_download():
    """多线程下载示例"""
    print("\n=== 多线程下载示例 ===")

    # 大文件多线程下载
    url = "https://httpbin.org/bytes/10485760"  # 10MB
    filepath = "./downloads/large_file.bin"

    print(f"多线程下载大文件: {url}")
    success = multi_thread_download(
        url=url,
        filepath=filepath,
        worker_num=8,  # 8个工作线程
        block_size=1,  # 1MB 块大小
        max_retries=3,  # 最大重试3次
        overwrite=True,  # 覆盖已存在的文件
    )

    if success:
        print(f"✅ 多线程下载成功: {filepath}")
        print(f"文件大小: {os.path.getsize(filepath)} 字节")
    else:
        print("❌ 多线程下载失败")


def example_single_thread_download():
    """单线程下载示例"""
    print("\n=== 单线程下载示例 ===")

    # 小文件单线程下载
    url = "https://httpbin.org/json"
    filepath = "./downloads/test.json"

    print(f"单线程下载: {url}")
    success = simple_download(
        url=url,
        filepath=filepath,
        chunk_size=1024,  # 1KB 块大小
        overwrite=True,
    )

    if success:
        print(f"✅ 单线程下载成功: {filepath}")
        # 显示文件内容（如果是文本文件）
        try:
            with open(filepath, "r") as f:
                content = f.read()
                print(f"文件内容预览: {content[:200]}...")
        except:
            pass
    else:
        print("❌ 单线程下载失败")


def example_upload():
    """文件上传示例"""
    print("\n=== 文件上传示例 ===")

    # 创建一个测试文件
    test_file = "./uploads/test_upload.txt"
    os.makedirs(os.path.dirname(test_file), exist_ok=True)

    with open(test_file, "w") as f:
        f.write("这是一个测试上传文件\n" * 100)

    # 上传到 httpbin（仅用于测试）
    upload_url = "https://httpbin.org/put"

    print(f"上传文件: {test_file} -> {upload_url}")
    success = single_upload(
        url=upload_url,
        filepath=test_file,
        method="PUT",  # 使用 PUT 方法
        chunk_size=1024,  # 1KB 块大小
        max_retries=3,
    )

    if success:
        print(f"✅ 上传成功: {test_file}")
    else:
        print("❌ 上传失败")


def example_with_custom_headers():
    """使用自定义请求头的示例"""
    print("\n=== 自定义请求头示例 ===")

    url = "https://httpbin.org/headers"
    filepath = "./downloads/headers_response.json"

    # 自定义请求头
    custom_headers = {
        "User-Agent": "Funget/1.0 (Custom Client)",
        "Accept": "application/json",
        "Authorization": "Bearer your-token-here",
    }

    print(f"使用自定义请求头下载: {url}")
    success = download(
        url=url, filepath=filepath, headers=custom_headers, overwrite=True
    )

    if success:
        print(f"✅ 下载成功: {filepath}")
    else:
        print("❌ 下载失败")


def example_error_handling():
    """错误处理示例"""
    print("\n=== 错误处理示例 ===")

    # 尝试下载不存在的文件
    invalid_url = "https://httpbin.org/status/404"
    filepath = "./downloads/not_found.txt"

    print(f"尝试下载不存在的文件: {invalid_url}")
    success = download(url=invalid_url, filepath=filepath, max_retries=2)

    if not success:
        print("❌ 预期的下载失败（404错误）")

    # 尝试下载到只读目录（如果权限不足）
    readonly_path = "/root/readonly_file.txt"  # 大多数用户没有写权限

    print(f"尝试下载到受限目录: {readonly_path}")
    success = download(url="https://httpbin.org/bytes/100", filepath=readonly_path)

    if not success:
        print("❌ 预期的下载失败（权限不足）")


def main():
    """主函数"""
    print("🚀 Funget 使用示例")
    print("=" * 50)

    # 确保下载和上传目录存在
    os.makedirs("./downloads", exist_ok=True)
    os.makedirs("./uploads", exist_ok=True)

    try:
        # 运行各种示例
        example_basic_download()
        example_single_thread_download()
        example_multi_thread_download()
        example_upload()
        example_with_custom_headers()
        example_error_handling()

        print("\n" + "=" * 50)
        print("✅ 所有示例执行完成！")

    except KeyboardInterrupt:
        print("\n⚠️  用户中断执行")
    except Exception as e:
        print(f"\n❌ 执行过程中发生错误: {e}")


if __name__ == "__main__":
    main()
