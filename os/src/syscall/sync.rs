//! 同步原语相关的系统调用。
//!
//! 这些调用号都在 1000 以上 —— 它们不是 POSIX。真实系统里互斥锁和条件
//! 变量是**进程内**的对象（pthread 那一套），用来同步同一地址空间里的
//! 多个线程；CCNUOS 没有线程，所以做成了系统级的内核对象，用整数 ID 引用。
//!
//! 好处是同步原语可以**跨进程**使用，哲学家就餐那种经典场景直接就能演。
//! 代价是没有"锁属于谁"的概念。见 `crate::sync` 的模块文档。

use crate::sync::{
    create_condvar, create_mutex, create_semaphore, get_condvar, get_mutex, get_semaphore,
};
use crate::task::current_pid;

pub fn sys_mutex_create() -> isize {
    create_mutex() as isize
}

pub fn sys_mutex_lock(id: usize) -> isize {
    let Some(mutex) = get_mutex(id) else {
        error!(
            "sys_mutex_lock：进程 {} 用了不存在的锁 {}",
            current_pid(),
            id
        );
        return -1;
    };
    mutex.lock();
    0
}

pub fn sys_mutex_unlock(id: usize) -> isize {
    let Some(mutex) = get_mutex(id) else {
        error!(
            "sys_mutex_unlock：进程 {} 用了不存在的锁 {}",
            current_pid(),
            id
        );
        return -1;
    };
    mutex.unlock();
    0
}

pub fn sys_semaphore_create(initial: usize) -> isize {
    create_semaphore(initial) as isize
}

pub fn sys_semaphore_down(id: usize) -> isize {
    let Some(sem) = get_semaphore(id) else {
        error!("sys_semaphore_down：不存在的信号量 {}", id);
        return -1;
    };
    sem.down();
    0
}

pub fn sys_semaphore_up(id: usize) -> isize {
    let Some(sem) = get_semaphore(id) else {
        error!("sys_semaphore_up：不存在的信号量 {}", id);
        return -1;
    };
    sem.up();
    0
}

pub fn sys_condvar_create() -> isize {
    create_condvar() as isize
}

/// `condvar_wait(condvar_id, mutex_id)` —— 放开锁、睡下，醒来重新上锁。
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    let (Some(cv), Some(mutex)) = (get_condvar(condvar_id), get_mutex(mutex_id)) else {
        error!(
            "sys_condvar_wait：条件变量 {} 或锁 {} 不存在",
            condvar_id, mutex_id
        );
        return -1;
    };
    cv.wait(&mutex);
    0
}

pub fn sys_condvar_signal(id: usize) -> isize {
    let Some(cv) = get_condvar(id) else {
        error!("sys_condvar_signal：不存在的条件变量 {}", id);
        return -1;
    };
    cv.signal();
    0
}

pub fn sys_condvar_broadcast(id: usize) -> isize {
    let Some(cv) = get_condvar(id) else {
        error!("sys_condvar_broadcast：不存在的条件变量 {}", id);
        return -1;
    };
    cv.broadcast() as isize
}
