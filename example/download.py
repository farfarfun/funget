import os
from funget import multi_thread_download, simple_download

# 确保下载目录存在
os.makedirs("./downloads", exist_ok=True)

# 单线程下载（适合小文件或测试）
print("开始单线程下载...")
success1 = simple_download(
    url="https://github.com/chen08209/FlClash/releases/download/v0.8.87/FlClash-0.8.87-macos-arm64.dmg",
    filepath="./downloads/FlClash-0.8.87-macos-arm64.dmg",
    overwrite=True,
    chunk_size=8192,  # 8KB 块大小
)

if success1:
    print("✅ 单线程下载成功！")
else:
    print("❌ 单线程下载失败！")

# 多线程下载（适合大文件）
print("\n开始多线程下载...")
success2 = multi_thread_download(
    url="https://github.com/chen08209/FlClash/releases/download/v0.8.87/FlClash-0.8.87-macos-arm64.dmg",
    filepath="./downloads/FlClash-0.8.87-macos-arm64-multi.dmg",
    worker_num=20,
    block_size=1024,  # 1MB 块大小
    overwrite=True,
    max_retries=3,
)

if success2:
    print("✅ 多线程下载成功！")
else:
    print("❌ 多线程下载失败！")

print("\n下载完成！检查 ./downloads/ 目录查看下载的文件。")
