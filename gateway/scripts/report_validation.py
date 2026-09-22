#!/usr/bin/env python3
"""Summarize measured evidence without turning missing scenarios into passes."""
import json
from pathlib import Path

root = Path(__file__).resolve().parents[2]
evidence = root / 'reviews/spec5-12-validation'
read = lambda path: json.loads((evidence / path).read_text())
checks = read('checks.json')
build = read('build.json')
lines = ['# Spec 05–12 本地联合验收', '',
         '日期：2026-09-21。基于既有 01–04 工作树实现；保留用户修改。未 Git 提交、远程部署或连接真实账户。', '',
         '## 结论与验收边界', '',
         'DO/RPC、请求 IR、遥测、cron、Hono Codex 插件、四方言请求/JSON/SSE 转换、WebSocket/realtime 已接入真实路径。功能范围与明确拒绝规则见 [实现说明](../../docs/spec5-12-implementation.md)。', '',
         '**不能标记全部 Spec 12 发布验收通过**：当前 cgroup 未向会话委派 CPU 控制器，无法执行 0.5/1/2 vCPU 的真实配额对照；线程数 1/2/4 的基线不代替该项。远程 CI 尚未执行，真实账户烟测尚未执行。', '',
         'Spec 06 原文引用的历史 bench/view_conformance 文件不在交付代码中，本轮提供 38 项本地 raw Proxy 语义断言；不能称为跑过那份历史文件。完整 Workers/WPT 与 v2 延后项未纳入通过声明。', '',
         '## 构建与检查', '',
         f"全量：{build['full_suite']['integration_passed']} 个集成用例、3 个 compile-fail 文档用例；2 个手动容量用例在普通测试中忽略，并在 release 专项单独执行。cleanup 的嵌套子进程重复输出不重复计数。", '',
         '| 检查 | 退出码 | 秒 |', '|---|---:|---:|']
for check in checks:
    lines.append(f"| {check['name']} | {check['exit_code']} | {check['seconds']:.2f} |")
lines += ['', '命令与日志见 `checks.json`、`full-tests.log`、`release-tests.log`。release 专项覆盖安装/重装、SIGTERM/SIGINT、kill -9 后确认 KV/DO 恢复、Codex OAuth/并发刷新/粘性、realtime 互斥和两种 WS 生命周期。另有 DO 网络/定时器连续初始化与 OOM 后持久数据恢复回归。', '',
          '构建产物与 Cargo.lock/示例 ZIP 的 SHA-256 见 `build.json`。Linux x86_64/glibc；QuickJS-ng 0.16.2、SQLite 静态编入，仍依赖系统 libc/libm/libgcc。第三方许可证在发布包 licenses 中。', '',
          '## HTTP 容量', '',
          'MB 使用十进制。RSS/CPU 来自 /proc，每 100ms 采样一次管理指标；空载为已安装测试插件、无常驻 DO。宿主/JS/trace 指标不是互斥的 RSS 分项，不能直接相加。测试运行于共享虚拟机，部分开发验证同机进行，延迟数据只作本机基线。', '',
          '所有非零大正文负载持续 60 秒 SSE，每秒输出一帧；478 KB 是精确 478000 字节。峰值 RSS 门槛 128 MB 对应 50 并发/478 KB 场景；128 并发与 800 KB 全遍历属于扩展基线，超出 128 MB 仍如实记录。0 并发为空载快照；8.8 KB/线程数对照持续 5 秒。没有把 SSE 总 wall 时间当作 CPU 执行时间。', '',
          '完整矩阵 `final-capacity/` 使用边界修复前的二进制，文件逐个记录 binary_sha256。最终交付构建的四个必测 50 并发场景重新测量于 `release-capacity/`；两个构建的差异为 DO 初始化门控修复，HTTP 基线无 DO。', '',
          '| 证据集/模式 | trace body | 并发 | body B | 线程 | 空载 MB | 峰值 MB | TTFB p95 ms | CPU s/请求 |',
          '|---|---|---:|---:|---:|---:|---:|---:|---:|']
for directory in ['final-capacity', 'release-capacity']:
    for path in sorted((evidence / directory).glob('*.json')):
        if path.name == 'completed.json':
            continue
        data = json.loads(path.read_text())
        scenario = data['scenario']
        ttfb = data['ttfb']['p95']
        cpu = data['cpu_seconds_per_request']
        lines.append(f"| {directory}/{scenario['mode']} | {scenario['trace']} | {scenario['clients']} | {data['payload_bytes']} | {scenario['threads']} | {data['idle_rss_bytes']/1e6:.2f} | {data['peak_rss_bytes']/1e6:.2f} | {ttfb*1000 if ttfb is not None else 0:.2f} | {cpu if cpu is not None else 0:.4f} |")
lines += ['', '每个 JSON 还保存 HTTP TTFB/wall p50/p95/p99、吞吐、CPU/request、采样序列、JS/宿主/DO/trace 峰值、正文 hash、有效 trace/log payload、DB/WAL 文件峰值及退出日志。慢流吞吐受固定 60 秒响应时长约束，不等于最大服务吞吐。', '',
          '## WebSocket 与 SQLite', '',
          '| WS 模式 | 连接 | 活动请求 Engine | 峰值 RSS MB | 10 秒 echo 数 |', '|---|---:|---:|---:|---:|']
for data in read('websocket-capacity.json'):
    lines.append(f"| {data['mode']} | {data['clients']} | {data['active_metrics']['requests']} | {data['peak_rss_bytes']/1e6:.2f} | {data['echoes']} |")
lines += ['', '每帧 256 B；原生转发释放请求 Engine，JS accept 保留到连接结束。详见 `websocket-capacity.json`。', '',
          'SQLite FULL/NORMAL 使用独立连接。`storage-capacity.json` 分别记录 KV/DO 200 次 512 B 确认提交的 p50/p95/p99，以及 4096 条 NORMAL 遥测 N=64/T=200ms 的入队提交、WAL 与 PASSIVE checkpoint；这些是本机存储基线。HTTP 结果另报告有效保留 payload 和真实 DB/WAL 大小。', '',
          '## 可重跑与交付', '',
          '从源码根目录执行：', '', '```sh',
          'gateway/scripts/verify.sh',
          'cargo test --manifest-path gateway/Cargo.toml --release --locked --test process --test do_gates --test do_storage --test codex_e2e --test realtime_e2e --test ws_capacity --test storage_capacity -- --include-ignored --nocapture',
          'python3 gateway/scripts/capacity_matrix.py --binary gateway/target/release/gateway --output-dir reviews/spec5-12-validation/reproduction',
          'python3 gateway/scripts/package_release.py', '```', '',
          '运行包只含二进制、配置、示例 ZIP、管理/实现文档、最终证据和许可证；源码另打包，排除 target、node_modules、业务数据库、凭据和工作区配置。SHA256SUMS 用于核验运行包内容。', '',
          '模块全局不跨普通请求保存；waitUntil 有时限；原生同步调用不可强行中断；单进程 SQLite；重装取消旧执行，不承诺零停机；异常退出可能丢失遥测尾批。DO 自发日志使用 durable 上下文 ID，未提供跨 RPC 的请求 ID 追踪。', '']
(evidence / 'README.md').write_text('\n'.join(lines))
