//! 磁盘文件：[`File`] trait 的第三个实现。
//!
//! 第 10 周定义 `File` trait 时写过一句承诺：
//!
//! > 有了 trait，那两章只要多写一个实现，`sys_read` / `sys_write`
//! > 一行都不用改。
//!
//! 第 11 周的管道兑现了一次，这一章是第二次。`sys_read` / `sys_write`
//! 从第 10 周写完之后**再也没有动过**。
//!
//! # 从此内核不再自带用户程序
//!
//! 第 3–12 周，用户程序是被 `build.rs` 用 `.incbin` 嵌进内核 `.data` 段的 ——
//! 因为那时没有文件系统，内核没有任何办法"读取"一个程序。
//!
//! 现在有了。`exec` 改成从磁盘读，`link_app.S` 和 `loader.rs` 一起退休。
//! 内核镜像因此瘦了两百多 KiB，而且**换程序不必重新编译内核**。

use super::File;
use crate::drivers::block_device;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use easy_fs::{EasyFileSystem, Inode};

/// `open` 的标志位。取值和 Linux 一致。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OpenFlags(u32);

impl OpenFlags {
    pub const RDONLY: Self = Self(0);
    pub const WRONLY: Self = Self(1 << 0);
    pub const RDWR: Self = Self(1 << 1);
    /// 不存在就创建
    pub const CREATE: Self = Self(1 << 9);
    /// 打开时清空已有内容
    pub const TRUNC: Self = Self(1 << 10);

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// 组合两个标志。
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// 解析出 `(可读, 可写)`。
    ///
    /// 注意 `RDONLY` 是 0，所以不能用"包含"来判断 —— 得先看低两位。
    fn rw(self) -> (bool, bool) {
        match self.0 & 0b11 {
            x if x == Self::RDONLY.0 => (true, false),
            x if x == Self::WRONLY.0 => (false, true),
            _ => (true, true), // RDWR
        }
    }
}

/// 一个打开的磁盘文件。
pub struct OSInode {
    readable: bool,
    writable: bool,
    inner: UPSafeCell<OSInodeInner>,
}

struct OSInodeInner {
    /// 读写位置。**它属于"打开的文件"而不是"文件本身"** ——
    /// 同一个文件被打开两次会有两个独立的位置，而 `fork` 出来的父子
    /// 共享同一个 `OSInode`，因而共享位置。这正是 Unix 的语义。
    offset: usize,
    inode: Arc<Inode>,
}

impl OSInode {
    fn new(readable: bool, writable: bool, inode: Arc<Inode>) -> Self {
        Self {
            readable,
            writable,
            // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
            inner: unsafe { UPSafeCell::new(OSInodeInner { offset: 0, inode }) },
        }
    }

    /// 从当前位置写一段字节。内核自己用（不经过用户地址空间）。
    pub fn write_bytes(&self, data: &[u8]) -> usize {
        let mut inner = self.inner.exclusive_access();
        let n = inner.inode.write_at(inner.offset, data);
        inner.offset += n;
        n
    }

    /// 一次性读出整个文件。`exec` 用它取 ELF。
    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.exclusive_access();
        let size = inner.inode.size();
        let mut data = alloc::vec![0u8; size];
        let mut read = 0;
        while read < size {
            let n = inner.inode.read_at(inner.offset, &mut data[read..]);
            if n == 0 {
                break;
            }
            inner.offset += n;
            read += n;
        }
        data.truncate(read);
        data
    }
}

impl File for OSInode {
    fn readable(&self) -> bool {
        self.readable
    }

    fn writable(&self) -> bool {
        self.writable
    }

    fn read(&self, mut buf: UserBuffer) -> Option<usize> {
        if !self.readable {
            error!("这个文件不是以可读方式打开的");
            return None;
        }
        // 先读到内核的临时缓冲，再搬进用户空间。
        // 不能直接读进 UserBuffer —— 它是按页切开的若干片段，
        // 而 inode 的 read_at 要一段连续内存。
        let want = buf.len();
        let mut tmp = alloc::vec![0u8; want];
        let n = {
            let mut inner = self.inner.exclusive_access();
            let n = inner.inode.read_at(inner.offset, &mut tmp);
            inner.offset += n;
            n
        };
        for (slot, byte) in buf.bytes_mut().zip(tmp.into_iter().take(n)) {
            *slot = byte;
        }
        Some(n)
    }

    fn write(&self, buf: UserBuffer) -> Option<usize> {
        if !self.writable {
            error!("这个文件不是以可写方式打开的");
            return None;
        }
        let data: Vec<u8> = buf.bytes().collect();
        let mut inner = self.inner.exclusive_access();
        let n = inner.inode.write_at(inner.offset, &data);
        inner.offset += n;
        Some(n)
    }
}

// ---------------------------------------------------------------------------
// 挂载
// ---------------------------------------------------------------------------

static ROOT_INODE: UPSafeCell<Option<Arc<Inode>>> =
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(None) };

/// 挂载块设备上的文件系统。失败返回 `false`。
pub fn init() -> bool {
    let Some(fs) = EasyFileSystem::open(block_device()) else {
        error!("块设备上不是一个合法的 CCNUOS 文件系统（魔数不对）");
        return false;
    };
    let root = Arc::new(EasyFileSystem::root_inode(&fs));
    let (inodes, used, total) = fs.lock().usage();
    info!(
        "文件系统已挂载：根目录 {} 个文件，inode {} 个，数据块 {}/{}",
        root.ls().len(),
        inodes,
        used,
        total
    );
    *ROOT_INODE.exclusive_access() = Some(root);
    true
}

fn root() -> Arc<Inode> {
    ROOT_INODE
        .exclusive_access()
        .as_ref()
        .expect("文件系统还没挂载")
        .clone()
}

/// 根目录下有哪些文件。`ls` 命令和 shell 的补全都用它。
pub fn list_files() -> Vec<String> {
    root().ls()
}

/// 按名字打开一个文件。
pub fn open_file(name: &str, flags: OpenFlags) -> Option<Arc<OSInode>> {
    let (readable, writable) = flags.rw();
    let root = root();

    if flags.contains(OpenFlags::CREATE) {
        if let Some(inode) = root.find(name) {
            // 已存在。CREATE 不是"必须新建"，所以直接用现成的。
            if flags.contains(OpenFlags::TRUNC) {
                inode.clear();
            }
            return Some(Arc::new(OSInode::new(readable, writable, inode)));
        }
        let inode = root.create(name)?;
        return Some(Arc::new(OSInode::new(readable, writable, inode)));
    }

    let inode = root.find(name)?;
    if flags.contains(OpenFlags::TRUNC) {
        inode.clear();
    }
    Some(Arc::new(OSInode::new(readable, writable, inode)))
}

/// 读出一个文件的全部内容。`exec` 用它取 ELF。
pub fn read_file(name: &str) -> Option<Vec<u8>> {
    open_file(name, OpenFlags::RDONLY).map(|f| f.read_all())
}
