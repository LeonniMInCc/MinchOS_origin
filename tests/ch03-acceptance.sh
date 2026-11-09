#!/usr/bin/env bash
# Chapter 3 QEMU acceptance regression; consumes captured serial output.
set -euo pipefail
log=${1:?usage: tests/ch03-acceptance.sh serial.log}
root=$(cd "$(dirname "$0")/.." && pwd)
need() { grep -Fq -- "$1" "$log" || { echo "missing runtime output: $1" >&2; exit 1; }; }
need 'sys_get_time(169)'
need '用户程序正常退出，退出码 0'
need '用户程序传来了越界的缓冲区'
need '用户程序执行了非法指令'
need '用户程序访存出错'
need '全部 5 个用户程序执行完毕，关机'
# Source-level guards cover details that are not printed by the guest.
grep -Fq 'checked_add' "$root/os/src/batch.rs"
grep -Fq 'fence.i' "$root/os/src/batch.rs"
grep -Fq '#[repr(C)]' "$root/os/src/trap/context.rs"
grep -Fq 'SYSCALL_GET_TIME: usize = 169' "$root/os/src/syscall/mod.rs"
printf 'chapter 3 acceptance: PASS\n'
