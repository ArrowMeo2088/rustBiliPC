#!/usr/bin/env python3
"""推送前本地校验 .github/workflows/*.yml —— 抓 GitHub 会拒、或更糟会**静默不跑**的那几类错。

为什么要这个脚本（2026-10-27，第五十二轮）：`python -c "yaml.safe_load(...)"` 只能证明 YAML
语法能读，读不出下面这几类，而它们里有一类让我连红三次还查不出原因：

1. **重复键**：`yaml.safe_load` 静默取最后一个，GitHub 直接报 `'p-static' is already defined`。
2. **`steps.<id>.*` 指到一个没有 `id:` 的步骤**：求值成**空串**而不报错。
   我那次是作业输出映射 `outputs.matrix: ${{ steps.pick.outputs.matrix }}`，
   而那个写 `$GITHUB_OUTPUT` 的步骤只写了 `name:`、漏了 `id: pick` → 输出永远为空 →
   下游 `fromJson('')` 得到**零条矩阵腿** → 那个作业**在作业列表里根本不出现**，
   run 结论红、日志与 annotations 零错误。**这是本地一条检查就能挡住的。**
3. `needs.<job_id>` 指到一个不存在的作业；`${{ inputs.x }}` 用一个没声明的输入。

用法：`python .github/scripts/check_workflows.py`（有错就非零退出，并把每条错印出来）。
"""

import re
import sys
from pathlib import Path

import yaml


def load(path):
    # 重复键之外的语法错误照常抛出；这里只为把"静默吞掉"的那类变成响的。
    def construct(loader, node, deep=False):
        mapping = {}
        for key_node, value_node in node.value:
            key = loader.construct_object(key_node, deep=deep)
            if key in mapping:
                raise ValueError(
                    f"重复键 {key!r}（第 {key_node.start_mark.line + 1} 行，"
                    f"先前出现在第 {mapping[key]!r} 那一行）"
                )
            mapping[key] = key_node.start_mark.line + 1
            mapping[key] = loader.construct_object(value_node, deep=deep)
        return mapping

    loader = type("L", (yaml.SafeLoader,), {})
    loader.add_constructor(
        yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
        lambda ldr, node, deep=False: construct(ldr, node, deep),
    )
    return yaml.load(path.read_text(encoding="utf-8"), Loader=loader)


def text_of(node):
    """把一棵解析后的子树重新序列化成文本，好在上面跑正则。"""
    return yaml.dump(node, allow_unicode=True, default_flow_style=False)


def check(path):
    errors = []
    doc = load(path)
    if not isinstance(doc, dict):
        return [f"{path}: 顶层不是映射"]

    on = doc.get(True, doc.get("on")) or {}
    inputs_declared = set()
    for trig in on.values() if isinstance(on, dict) else []:
        if isinstance(trig, dict):
            spec = trig.get("inputs", {}) if isinstance(trig.get("inputs"), dict) else {}
            inputs_declared |= set(spec.keys())

    jobs = doc.get("jobs") or {}

    for job_id, job in jobs.items():
        if not isinstance(job, dict):
            continue
        steps = job.get("steps") or []
        ids = set()
        for step in steps if isinstance(steps, list) else []:
            if isinstance(step, dict) and "id" in step:
                ids.add(str(step["id"]))
        blob = text_of(job)
        for ref in set(re.findall(r"steps\.([A-Za-z0-9_-]+)\.", blob)):
            if ref not in ids:
                errors.append(
                    f"作业 {job_id}：引用了 steps.{ref}.* 但没有 id 为 {ref!r} 的步骤"
                    f"（那个引用求值为空串，GitHub 不报错）"
                )
        needs = job.get("needs")
        wanted = [needs] if isinstance(needs, str) else (needs or [])
        for dep in wanted:
            if dep not in jobs:
                errors.append(f"作业 {job_id}：needs 引用了不存在的作业 {dep!r}")
        for ctx in set(re.findall(r"inputs\.([A-Za-z0-9_-]+)", blob)):
            if inputs_declared and ctx not in inputs_declared:
                errors.append(
                    f"作业 {job_id}：用了 inputs.{ctx}，但触发器里没有这个输入"
                )

    # 矩阵：`include` 每一项给的键要一致，否则某条腿缺键时 `matrix.x` 与 null 比较是坑。
    for job_id, job in jobs.items():
        strategy = (job or {}).get("strategy") if isinstance(job, dict) else None
        matrix = (strategy or {}).get("matrix") if isinstance(strategy, dict) else None
        include = matrix.get("include") if isinstance(matrix, dict) else None
        if isinstance(include, list) and include:
            keys = {frozenset(item.keys()) for item in include if isinstance(item, dict)}
            if len(keys) > 1:
                errors.append(
                    f"作业 {job_id}：矩阵 include 各项的键不一致 {sorted(sorted(k) for k in keys)}"
                )
    return errors


def main():
    root = Path(__file__).resolve().parents[1] / "workflows"
    files = sorted(root.glob("*.yml")) + sorted(root.glob("*.yaml"))
    bad = 0
    for f in files:
        try:
            errs = check(f)
        except Exception as exc:  # 语法/重复键错误从这里出来
            print(f"[红] {f.name}: {exc}")
            bad += 1
            continue
        if errs:
            bad += 1
            for e in errs:
                print(f"[红] {e}")
        else:
            print(f"[绿] {f.name}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
