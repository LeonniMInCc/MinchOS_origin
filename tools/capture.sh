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

# 可以喂一段脚本给 shell（CCNUOS_INPUT）。不设的话用 /dev/null ——
# 那样 shell 会一直等输入等到超时，录出来的就只有启动过程。
INPUT="${CCNUOS_INPUT:-}"

if [ -z "$INPUT" ]; then
    "$@" < /dev/null > "$LOG" 2>&1 &
    QPID=$!
else
    [ -r "$INPUT" ] || { echo "[capture] 读不到输入文件 $INPUT" >&2; exit 1; }

    # **必须按行慢喂。** 直接 `qemu < file` 会把整个文件一次性推进串口，
    # 而客户机是逐字符轮询取的（SBI 没有阻塞式读接口），来不及消费的
    # 字符就丢了 —— 症状是命令缺字母，比如 "help" 变成 "elp"。
    #
    # 先等内核启动完，再一行一行喂，每行之间留出执行时间。
    {
        sleep "${CCNUOS_BOOT_WAIT:-2}"
        while IFS= read -r cmd; do
            printf '%s\n' "$cmd"
            sleep "${CCNUOS_LINE_DELAY:-1}"
        done < "$INPUT"
    } | "$@" > "$LOG" 2>&1 &
    QPID=$!
fi

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
