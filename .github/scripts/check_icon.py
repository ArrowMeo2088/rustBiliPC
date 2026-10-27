#!/usr/bin/env python3
"""验一个 Windows 可执行文件里有没有编进图标资源（RT_ICON=3 + RT_GROUP_ICON=14）。

为什么要这个脚本：第五十五轮 `build.rs` 用了 `link.exe` 并不认识的 `/ICON:`，链接器只丢一句
`LNK4044 … ignored`，**编译照样绿**。光看构建成功查不出这件事，只能回头看二进制。

为什么它自己也差点骗人（第五十八轮）：两个坑叠在一起，把 `winver.exe` 这种一定有图标的文件
误判成"没有 .rsrc"——
① 资源目录是数据目录的**索引 2**，每项 8 字节，所以偏移是 `+16`，我写成 `+32`；
② 段表起点是 `pe + 24 + SizeOfOptionalHeader`，我写成 `pe + 20 + …`（少 4 字节），
   而且 Windows 的系统二进制常把 `SizeOfOptionalHeader` 写成 0，这时要按 PE32/PE32+ 用默认值。
所以这个脚本现在**会把解析到的头信息打出来**（`--verbose`），结果不对一眼就能看出来，
不用等下一轮 CI。

用法：`python .github/scripts/check_icon.py <exe> [--quiet] [--verbose]`
退出码：0 有图标；1 没有；2 读不了/不是 PE。
"""
import struct
import sys
from pathlib import Path

RT_ICON = 3
RT_GROUP_ICON = 14
KIND = {1: "RT_CURSOR", 2: "RT_BITMAP", 3: "RT_ICON", 4: "RT_MENU", 5: "RT_DIALOG",
        6: "RT_STRING", 8: "RT_FONTDIR", 9: "RT_FONT", 10: "RT_ACCELERATOR",
        11: "RT_RCDATA", 12: "RT_MESSAGETABLE", 13: "RT_GROUP_CURSOR", 14: "RT_GROUP_ICON",
        16: "RT_VERSION", 17: "RT_DLGINCLUDE", 19: "RT_PLUGPLAY", 20: "RT_VXD",
        21: "RT_ANICURSOR", 22: "RT_ANIICON", 23: "RT_HTML", 24: "RT_MANIFEST"}


def utf8_stdout():
    """Windows 控制台默认码页不是 UTF-8，中文 print 会 UnicodeEncodeError（第五十六轮踩过）。"""
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass


def parse(path, verbose=False):
    """返回 (顶层资源类型集合, 说明)。读不到资源目录时返回 (None, 原因)。"""
    b = Path(path).read_bytes()
    if b[:2] != b"MZ" or len(b) < 0x40:
        return None, "不是 MZ 可执行文件"
    pe = struct.unpack_from("<I", b, 0x3C)[0]
    if b[pe:pe + 4] != b"PE\0\0":
        return None, "PE 签名不对"
    nsec = struct.unpack_from("<H", b, pe + 6)[0]
    optsz = struct.unpack_from("<H", b, pe + 16)[0]
    magic = struct.unpack_from("<H", b, pe + 24)[0]
    pe32plus = magic == 0x20B
    if optsz == 0:                      # Windows 系统二进制常见的写法，按标准大小兜底
        optsz = 0xF0 if pe32plus else 0xE0
    sec = pe + 24 + optsz               # 签名 4 + COFF 头 20，不是 20
    dd = pe + 24 + (112 if pe32plus else 96)
    rsrc_rva, rsrc_size = struct.unpack_from("<II", b, dd + 2 * 8)   # 索引 2 = 资源
    if verbose:
        print(f"   PE 头 {pe:#x} 段数 {nsec} 可选头大小 {optsz:#x} "
              f"{'PE32+' if pe32plus else 'PE32'} 段表 {sec:#x}")
        print(f"   资源目录 RVA {rsrc_rva:#x} 大小 {rsrc_size}")
    if rsrc_rva == 0:
        return None, "没有资源目录（数据目录第 2 项是 0）"
    for i in range(nsec):
        e = b[sec + 40 * i: sec + 40 * (i + 1)]
        name = e[:8].rstrip(b"\0").decode("latin-1")
        vsize, va, rawsize, raw = struct.unpack_from("<IIII", e, 8)
        if va <= rsrc_rva < va + max(vsize, rawsize):
            base = raw + (rsrc_rva - va)
            if verbose:
                print(f"   .rsrc 段 {name} va={va:#x} raw={raw:#x} → 目录在文件偏移 {base:#x}")
            break
    else:
        return None, f"资源 RVA {rsrc_rva:#x} 落在任何段之外"
    n_named, n_id = struct.unpack_from("<HH", b, base + 12)
    kinds = set()
    named = []
    for k in range(n_named + n_id):
        entry = base + 16 + 8 * k
        raw_id = struct.unpack_from("<I", b, entry)[0]
        if k < n_named:
            # 命名条目：高位是标志，低位是在 .rsrc 里的偏移，后面跟着长度 + UTF-16 名字。
            length = struct.unpack_from("<H", b, (raw_id & 0x7FFFFFFF) + base)[0]
            blob = b[base + (raw_id & 0x7FFFFFFF) + 2: base + (raw_id & 0x7FFFFFFF) + 2 + length * 2]
            named.append(blob.decode("utf-16le", errors="replace"))
        else:
            kinds.add(raw_id)
    if verbose:
        print("   顶层 ID 条目：", ", ".join(KIND.get(x, f"type{x}") for x in sorted(kinds)) or "（空）")
        if named:
            print("   顶层命名条目：", ", ".join(named[:8]) + ("…" if len(named) > 8 else ""))
    return kinds, None


def main():
    utf8_stdout()
    argv = sys.argv[1:]
    verbose = "--verbose" in argv
    quiet = "--quiet" in argv
    args = [a for a in argv if not a.startswith("--")]
    if not args:
        print("用法：check_icon.py <exe> [--quiet] [--verbose]")
        return 2
    target = args[0]
    try:
        kinds, err = parse(target, verbose)
    except OSError as exc:
        print(f"图标校验：读不到 {target}（{exc}）")
        return 2
    if err:
        print(f"图标校验：{err} —— 这个产物不带自定义图标")
        return 1
    has_icon = RT_ICON in kinds and RT_GROUP_ICON in kinds
    names = ", ".join(KIND.get(x, f"type{x}") for x in sorted(kinds)) or "（空）"
    if has_icon:
        print(f"图标校验：有 RT_ICON + RT_GROUP_ICON（顶层：{names}）")
        return 0
    print(f"图标校验：没有图标资源（顶层：{names}）")
    return 1


if __name__ == "__main__":
    sys.exit(main())
