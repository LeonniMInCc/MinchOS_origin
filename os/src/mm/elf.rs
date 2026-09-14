//! 最小的 ELF64 解析器。
//!
//! 之前几章内核装载用户程序的办法是"把一段裸字节拷到固定地址"。
//! 那样有两个问题：
//!
//! 1. 不知道程序的入口在哪，只能假定它等于加载地址
//! 2. 不知道哪一段是代码、哪一段是数据 —— 于是只能给整块内存
//!    同时开读、写、执行权限，这是安全上的大忌
//!
//! ELF 文件把这些信息都写在**程序头表**里。解析它，我们才能
//! 按段设置权限：代码段只读可执行，数据段可读写不可执行。
//!
//! 这里只解析我们真正需要的部分 —— 不引入 `xmas-elf` 之类的 crate，
//! 一百来行就能把格式讲清楚。
//!
//! # ELF64 文件的结构
//!
//! ```text
//!   ┌────────────────────┐ 0
//!   │   ELF Header       │  魔数、入口地址、程序头表在哪
//!   ├────────────────────┤ e_phoff
//!   │   Program Header 0 │  每个 56 字节，描述一个"段"
//!   │   Program Header 1 │
//!   │        ...         │
//!   ├────────────────────┤
//!   │   段的实际内容      │
//!   └────────────────────┘
//! ```

/// 可加载段。装载器只关心这一种。
pub const PT_LOAD: u32 = 1;

/// 段权限标志。
pub const PF_X: u32 = 1 << 0;
pub const PF_W: u32 = 1 << 1;
pub const PF_R: u32 = 1 << 2;

/// ELF64 程序头表每一项的大小。
const PHENT_SIZE: usize = 56;
/// ELF64 文件头的最小长度。
const EHDR_SIZE: usize = 64;

/// 一个程序头，描述文件里的一段该怎么被装入内存。
#[derive(Clone, Copy, Debug)]
pub struct ProgramHeader {
    pub p_type: u32,
    /// 权限：`PF_R` / `PF_W` / `PF_X` 的组合
    pub flags: u32,
    /// 这一段在**文件**里的偏移
    pub offset: usize,
    /// 这一段该被装到哪个**虚拟地址**
    pub vaddr: usize,
    /// 在文件里占多少字节
    pub filesz: usize,
    /// 在内存里占多少字节
    ///
    /// `memsz > filesz` 是正常的：差额就是 `.bss`。它在文件里不占空间
    /// （存一堆 0 没意义），但装载时必须补上零。
    pub memsz: usize,
}

pub struct ElfFile<'a> {
    data: &'a [u8],
    phoff: usize,
    phnum: usize,
    entry: usize,
}

impl<'a> ElfFile<'a> {
    /// 解析并校验一个 ELF64 RISC-V 文件。
    ///
    /// 这些校验不是形式主义 —— 内核即将把这段字节当作**代码**执行，
    /// 而字节的来源不一定可信。格式错了要在这里就拦住，
    /// 而不是等 CPU 跳进去之后才发现。
    pub fn parse(data: &'a [u8]) -> Result<Self, &'static str> {
        if data.len() < EHDR_SIZE {
            return Err("文件太短，装不下 ELF 头");
        }
        if &data[0..4] != b"\x7fELF" {
            return Err("ELF 魔数不对");
        }
        if data[4] != 2 {
            return Err("不是 64 位 ELF");
        }
        if data[5] != 1 {
            return Err("不是小端序");
        }
        // e_machine @ 18，RISC-V 是 0xF3
        if read_u16(data, 18) != 0xF3 {
            return Err("不是 RISC-V 目标");
        }

        let entry = read_u64(data, 24) as usize;
        let phoff = read_u64(data, 32) as usize;
        let phentsize = read_u16(data, 54) as usize;
        let phnum = read_u16(data, 56) as usize;

        if phentsize != PHENT_SIZE {
            return Err("程序头大小不是 56 字节");
        }
        // 防止后面读越界：程序头表必须完整地落在文件内
        let table_end = phoff
            .checked_add(phnum * PHENT_SIZE)
            .ok_or("程序头表偏移溢出")?;
        if table_end > data.len() {
            return Err("程序头表超出文件范围");
        }

        Ok(Self {
            data,
            phoff,
            phnum,
            entry,
        })
    }

    /// 程序入口地址。
    pub fn entry(&self) -> usize {
        self.entry
    }

    /// 遍历所有程序头。
    pub fn program_headers(&self) -> impl Iterator<Item = ProgramHeader> + '_ {
        (0..self.phnum).map(move |i| {
            let off = self.phoff + i * PHENT_SIZE;
            let d = self.data;
            ProgramHeader {
                p_type: read_u32(d, off),
                flags: read_u32(d, off + 4),
                offset: read_u64(d, off + 8) as usize,
                vaddr: read_u64(d, off + 16) as usize,
                filesz: read_u64(d, off + 32) as usize,
                memsz: read_u64(d, off + 40) as usize,
            }
        })
    }

    /// 取出某一段在文件里的内容。
    ///
    /// 返回 `None` 表示这一段的 `offset/filesz` 指到了文件之外 ——
    /// 一个构造出来的恶意 ELF 完全可能这么干。
    pub fn segment_data(&self, ph: &ProgramHeader) -> Option<&'a [u8]> {
        let end = ph.offset.checked_add(ph.filesz)?;
        self.data.get(ph.offset..end)
    }
}

/// 把权限标志渲染成 `RWX` 形式，便于在日志里对齐比较。
pub fn flags_str(flags: u32) -> [char; 3] {
    [
        if flags & PF_R != 0 { 'R' } else { '-' },
        if flags & PF_W != 0 { 'W' } else { '-' },
        if flags & PF_X != 0 { 'X' } else { '-' },
    ]
}

// ---------------------------------------------------------------------------
// 小端序读取
// ---------------------------------------------------------------------------
//
// 调用点都已经保证了范围（parse 里校验过文件长度和程序头表边界），
// 所以这里用定长数组切片 + unwrap，越界会 panic 而不是读到垃圾。

fn read_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(data[off..off + 2].try_into().unwrap())
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

fn read_u64(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(data[off..off + 8].try_into().unwrap())
}
