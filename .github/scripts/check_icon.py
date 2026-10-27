#!/usr/bin/env python3
"""验一个 Windows 可执行文件里到底有没有编进图标资源（RT_ICON=3 / RT_GROUP_ICON=14）。

存在的理由：第五十五轮 `build.rs` 用了 `link.exe` 并不认识的 `/ICON:`，链接器只丢一句
`LNK4044 … ignored`，编译照样绿、产物照样没图标。**光看编译成不成功是查不出这件事的**，
所以这一步只看二进制本身。

用法：`python .github/scripts/check_icon.py <文件> [--quiet]`
退出码：0 有图标；1 没有（`--quiet` 时只打一行结论，给 CI 的 summary 用）。
"""
import struct
import sys
from pathlib import Path

RT_ICON = 3
RT_GROUP_ICON = 14


def top_level_resource_types(path):
    """返回 .rsrc 顶层（类型层）的资源 ID 集合；没有资源目录时返回 None。"""
    b = Path(path).read_bytes()
    if b[:2] != b"MZ" or len(b) < 0x40:
        return None
    pe = struct.unpack_from("<I", b, 0x3C)[0]
    if b[pe:pe + 4] != b"PE\0\0":
        return None
    nsec = struct.unpack_from("<H", b, pe + 6)[0]
    optsz = struct.unpack_from("<H", b, pe + 16)[0]
    magic = struct.unpack_from("<H", b, pe + 24)[0]
    dd = pe + 24 + (112 if magic == 0x20B else 96)
    rva, _ = struct.unpack_from("<II", b, dd + 32)  # 数据目录第 3 个（索引 2）= 资源
    if rva == 0:
        return None
    sec = pe + 20 + optsz
    for i in range(nsec):
        e = b[sec + 40 * i: sec + 40 * (i + 1)]
        name = e[:8].rstrip(b"\0").decode("latin-1")
        vsize, va, rawsize, raw = struct.unpack_from("<IIII", e, 8)
        if name == ".rsrc":
            base = raw + (rva - va)
            n_named, n_id = struct.unpack_from("<HH", b, base + 12)
            return {
                struct.unpack_from("<I", b, base + 16 + 8 * k)[0]
                for k in range(n_named + n_id)
            }
    return set()


def _force_utf8_stdout():
    """Windows 控制台默认是码页编码（cp1252/cp936…），打印中文会直接 UnicodeEncodeError。
    第五十六轮就是这样红了一次：脚本本身算出了结论，死在那句 print 上。"""
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass


def main():
    _force_utf8_stdout()
    args = [a for a in sys.argv[1:] if a != "--quiet"]
    quiet = "--quiet" in sys.argv[1:]
    if not args:
        print("用法：check_icon.py <exe> [--quiet]")
        return 2
    target = args[0]
    try:
        ids = top_level_resource_types(target)
    except OSError as exc:
        print(f"图标校验：读不到 {target}（{exc}）")
        return 2
    if ids is None:
        verdict = "没有 .rsrc 资源目录 —— 这个产物不带任何 Win32 资源（包括图标）"
        has_icon = False
    else:
        has_icon = RT_ICON in ids and RT_GROUP_ICON in ids
        verdict = (
            f"有图标资源（RT_ICON + RT_GROUP_ICON；顶层类型 {sorted(ids)}）"
            if has_icon
            else f"没有图标资源；顶层类型只有 {sorted(ids)}"
        )
    print(("图标校验：" if not quiet else "") + verdict)
    return 0 if has_icon else 1


if __name__ == "__main__":
    sys.exit(main())
