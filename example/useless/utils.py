import logging
import os
import pickle
import re
from datetime import datetime, timedelta
from random import choice, choices, sample, shuffle, uniform

import requests

logger = logging.getLogger("fundrive")

# 调试日志设置

headers = {
    "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/75.0.3770.100 Safari/537.36",
    "Referer": "https://www.lanzous.com",
    "Accept-Language": "zh-CN,zh;q=0.9",
}


def remove_notes(html: str) -> str:
    """删除网页的注释"""
    # 去掉 html 里面的 // 和 <!-- --> 注释，防止干扰正则匹配提取数据
    # 蓝奏云的前端程序员喜欢改完代码就把原来的代码注释掉,就直接推到生产环境了 =_=
    html = re.sub(r"<!--.+?-->|\s+//\s*.+", "", html)  # html 注释
    html = re.sub(r"(.+?[,;])\s*//.+", r"\1", html)  # js 注释
    return html


def name_format(name: str) -> str:
    """去除非法字符"""
    name = (
        name.replace("\xa0", " ").replace("\u3000", " ").replace("  ", " ")
    )  # 去除其它字符集的空白符,去除重复空白字符
    return re.sub(r"[$%^!*<>)(+=`\'\"/:;,?]", "", name)


def time_format(time_str: str) -> str:
    """输出格式化时间 %Y-%m-%d"""
    if "秒前" in time_str or "分钟前" in time_str or "小时前" in time_str:
        return datetime.today().strftime("%Y-%m-%d")
    elif "昨天" in time_str:
        return (datetime.today() - timedelta(days=1)).strftime("%Y-%m-%d")
    elif "前天" in time_str:
        return (datetime.today() - timedelta(days=2)).strftime("%Y-%m-%d")
    elif "天前" in time_str:
        days = time_str.replace(" 天前", "")
        return (datetime.today() - timedelta(days=int(days))).strftime("%Y-%m-%d")
    else:
        return time_str


def is_name_valid(filename: str) -> bool:
    """检查文件名是否允许上传"""

    valid_suffix_list = (
        "ppt",
        "xapk",
        "ke",
        "azw",
        "cpk",
        "gho",
        "dwg",
        "db",
        "docx",
        "deb",
        "e",
        "ttf",
        "xls",
        "bat",
        "crx",
        "rpm",
        "txf",
        "pdf",
        "apk",
        "ipa",
        "txt",
        "mobi",
        "osk",
        "dmg",
        "rp",
        "osz",
        "jar",
        "ttc",
        "z",
        "w3x",
        "xlsx",
        "cetrainer",
        "ct",
        "rar",
        "mp3",
        "pptx",
        "mobileconfig",
        "epub",
        "imazingapp",
        "doc",
        "iso",
        "img",
        "appimage",
        "7z",
        "rplib",
        "lolgezi",
        "exe",
        "azw3",
        "zip",
        "conf",
        "tar",
        "dll",
        "flac",
        "xpa",
        "lua",
    )

    return filename.split(".")[-1] in valid_suffix_list


def is_file_url(share_url: str) -> bool:
    """判断是否为文件的分享链接"""
    base_pat = r"https?://.+?\.lanzou[six|w].com/.+"
    # user_pat = r'https?://.+?\.lanzou[six|w].com/i[a-z0-9]{5,}/?'  # 普通用户 URL 规则
    user_pat = (
        r"https?://.+?\.lanzou[six|w].com/i[a-zA-Z0-9]{5,10}/?"  # 普通用户 URL 规则
    )
    if not re.fullmatch(base_pat, share_url):
        return False
    elif re.fullmatch(user_pat, share_url):
        return True
    else:  # VIP 用户的 URL 很随意
        try:
            html = requests.get(share_url, headers=headers).text
            html = remove_notes(html)
            return (
                True
                if re.search(r'class="fileinfo"|id="file"|文件描述', html)
                else False
            )
        except (requests.RequestException, Exception):
            return False


def is_folder_url(share_url: str) -> bool:
    """判断是否为文件夹的分享链接"""
    base_pat = r"https?://.+?\.lanzou[six].com/.+"
    user_pat = r"https?://.+?\.lanzou[six].com/b[a-z0-9]{7,}/?"
    if not re.fullmatch(base_pat, share_url):
        return False
    elif re.fullmatch(user_pat, share_url):
        return True
    else:  # VIP 用户的 URL 很随意
        try:
            html = requests.get(share_url, headers=headers).text
            html = remove_notes(html)
            return True if re.search(r'id="infos"', html) else False
        except (requests.RequestException, Exception):
            return False


def un_serialize(data: bytes):
    """反序列化文件信息数据"""
    try:
        ret = pickle.loads(data)
        if not isinstance(ret, dict):
            return None
        return ret
    except Exception as e:  # 这里可能会丢奇怪的异常
        print(e)
        return None


def big_file_split(
    file_path: str, max_size: int = 100, start_byte: int = 0
) -> (int, str):
    """将大文件拆分为大小、格式随机的数据块, 可指定文件起始字节位置(用于续传)
    :return 数据块文件的大小和绝对路径
    """
    file_name = os.path.basename(file_path)
    file_size = os.path.getsize(file_path)
    tmp_dir = (
        os.path.dirname(file_path) + os.sep + "__" + ".".join(file_name.split(".")[:-1])
    )

    if not os.path.exists(tmp_dir):
        os.makedirs(tmp_dir)

    def get_random_size() -> int:
        """按权重生成一个不超过 max_size 的文件大小"""
        reduce_size = choices(
            [uniform(0, 20), uniform(20, 30), uniform(30, 60), uniform(60, 80)],
            weights=[2, 5, 2, 1],
        )
        return round((max_size - reduce_size[0]) * 1048576)

    def get_random_name() -> str:
        """生成一个随机文件名"""
        # 这些格式的文件一般都比较大且不容易触发下载检测
        suffix_list = (
            "zip",
            "rar",
            "apk",
            "ipa",
            "exe",
            "pdf",
            "7z",
            "tar",
            "deb",
            "dmg",
            "rpm",
            "flac",
        )
        name = list(file_name.replace(".", "").replace(" ", ""))
        name = name + sample("abcdefghijklmnopqrstuvwxyz", 3) + sample("1234567890", 2)
        shuffle(name)  # 打乱顺序
        name = "".join(name) + "." + choice(suffix_list)
        return name_format(name)  # 确保随机名合法

    with open(file_path, "rb") as big_file:
        big_file.seek(start_byte)
        left_size = file_size - start_byte  # 大文件剩余大小
        random_size = get_random_size()
        tmp_file_size = random_size if left_size > random_size else left_size
        tmp_file_path = tmp_dir + os.sep + get_random_name()

        chunk_size = 524288  # 512KB
        left_read_size = tmp_file_size
        with open(tmp_file_path, "wb") as small_file:
            while left_read_size > 0:
                if left_read_size < chunk_size:  # 不足读取一次
                    small_file.write(big_file.read(left_read_size))
                    break
                # 一次读取一块,防止一次性读取占用内存
                small_file.write(big_file.read(chunk_size))
                left_read_size -= chunk_size

    return tmp_file_size, tmp_file_path


def let_me_upload(file_path):
    """允许文件上传"""
    file_size = os.path.getsize(file_path) / 1024 / 1024  # MB
    file_name = os.path.basename(file_path)

    big_file_suffix = [
        "zip",
        "rar",
        "apk",
        "ipa",
        "exe",
        "pdf",
        "7z",
        "tar",
        "deb",
        "dmg",
        "rpm",
        "flac",
    ]
    small_file_suffix = big_file_suffix + ["doc", "epub", "mobi", "mp3", "ppt", "pptx"]
    big_file_suffix = choice(big_file_suffix)
    small_file_suffix = choice(small_file_suffix)
    suffix = small_file_suffix if file_size < 30 else big_file_suffix
    new_file_path = ".".join(file_path.split(".")[:-1]) + "." + suffix

    with open(new_file_path, "wb") as out_f:
        # 写入原始文件数据
        with open(file_path, "rb") as in_f:
            chunk = in_f.read(4096)
            while chunk:
                out_f.write(chunk)
                chunk = in_f.read(4096)
        # 构建文件 "报尾" 保存真实文件名,大小 512 字节
        # 追加数据到文件尾部，并不会影响文件的使用，无需修改即可分享给其他人使用，自己下载时则会去除，确保数据无误
        padding = 512 - len(file_name.encode("utf-8")) - 42  # 序列化后空字典占 42 字节
        data = {"name": file_name, "padding": b"\x00" * padding}
        data = pickle.dumps(data)
        out_f.write(data)
    return new_file_path


def unit_step(size):
    if size < 1024:
        unit = "B"
        step = 1
    elif size < 1024 * 1024:
        unit = "KB"
        step = 1024
    elif size < 1024 * 1024 * 1024:
        unit = "MB"
        step = 1024 * 1024
    else:
        unit = "GB"
        step = 1024 * 1024 * 1024
    return unit, step * 1.0
