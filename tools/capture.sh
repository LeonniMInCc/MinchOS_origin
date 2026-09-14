#!/usr/bin/env bash
#
# 捕获一次 CCNUOS 启动的完整串口输出（保留 ANSI 颜色），写进日志文件。
#
#   用法: tools/capture.sh <日志路径> <qemu 命令...>
#
# 内核正常情况下会在自检结束后调 SBI 关机，QEMU 自行退出。
# 这里的超时只是兜底 —— 如果内核卡死了（第 3 周写汇编时很可能），
# 不至于让脚本永远挂在那里。

set -u

LOG="$1"
shift

TIMEOUT="${CCNUOS_TIMEOUT:-30}"

mkdir -p "$(dirname "$LOG")"
: > "$LOG"

"$@" < /dev/null > "$LOG" 2>&1 &
QPID=$!

elapsed=0
while kill -0 "$QPID" 2>/dev/null; do
    if [ "$elapsed" -ge "$TIMEOUT" ]; then
        echo "[capture] QEMU 超过 ${TIMEOUT}s 未退出，强制终止 —— 内核可能卡住了" >&2
        kill "$QPID" 2>/dev/null
        wait "$QPID" 2>/dev/null
        break
    fi
    sleep 1
    elapsed=$((elapsed + 1))
done

wait "$QPID" 2>/dev/null

echo "[capture] 已写入 $LOG（$(wc -l < "$LOG" | tr -d ' ') 行，耗时 ${elapsed}s）"
