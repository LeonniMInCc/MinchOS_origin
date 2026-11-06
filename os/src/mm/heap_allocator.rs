//! 伙伴系统：每阶链表存放大小为 2^阶 的空闲块。
//! 空闲块首字存 next；已分配块完全归调用者所有。
use core::{alloc::Layout, mem::size_of, ptr::null_mut};

pub const MAX_ORDER: usize = 32;
pub struct FreeList {
    head: *mut usize,
}
impl FreeList {
    pub const fn new() -> Self {
        Self { head: null_mut() }
    }
    /// SAFETY: block 独占、可写、usize 对齐，至少一字大小且不在链表中。
    pub unsafe fn push(&mut self, block: *mut usize) {
        unsafe {
            block.write(self.head as usize);
        }
        self.head = block;
    }
    pub unsafe fn pop(&mut self) -> Option<*mut usize> {
        if self.head.is_null() {
            return None;
        }
        let block = self.head;
        // SAFETY: 链表不变量保证头节点可读。
        self.head = unsafe { block.read() as *mut usize };
        Some(block)
    }
    /// 只在同阶空闲链表中查找伙伴，不能仅凭地址相邻就合并。
    pub unsafe fn remove(&mut self, target: *mut usize) -> bool {
        let mut previous: *mut usize = null_mut();
        let mut current = self.head;
        while !current.is_null() {
            let next = unsafe { current.read() as *mut usize };
            if current == target {
                if previous.is_null() {
                    self.head = next;
                } else {
                    unsafe {
                        previous.write(next as usize);
                    }
                }
                return true;
            }
            previous = current;
            current = next;
        }
        false
    }
}

pub struct BuddyAllocator {
    free_list: [FreeList; MAX_ORDER],
    total: usize,
    allocated: usize,
    peak: usize,
}
impl BuddyAllocator {
    pub const fn new() -> Self {
        Self {
            free_list: [const { FreeList::new() }; MAX_ORDER],
            total: 0,
            allocated: 0,
            peak: 0,
        }
    }
    /// 按地址对齐能力和剩余空间，分解不规则大小的堆。
    /// SAFETY: [start,end) 是独占可写内存，生命周期覆盖分配器的使用；
    /// 不能重复加入或与既有堆重叠，调用期间不能并发访问。
    pub unsafe fn add_to_heap(&mut self, start: usize, end: usize) {
        let word = size_of::<usize>();
        let Some(rounded) = start.checked_add(word - 1) else {
            return;
        };
        let mut cursor = rounded & !(word - 1);
        let end = end & !(word - 1);
        while cursor < end {
            let remaining = end - cursor;
            let by_size = (usize::BITS - 1 - remaining.leading_zeros()) as usize;
            let by_align = cursor.trailing_zeros() as usize;
            let order = by_size.min(by_align).min(MAX_ORDER - 1);
            let size = 1usize << order;
            unsafe {
                self.free_list[order].push(cursor as *mut usize);
            }
            self.total += size;
            cursor += size;
        }
    }
    fn block_size(layout: Layout) -> Option<usize> {
        layout
            .size()
            .max(layout.align())
            .max(size_of::<usize>())
            .checked_next_power_of_two()
    }
    /// 内存不足返回空指针；内部不能打印或创建容器，避免递归分配。
    pub fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let Some(size) = Self::block_size(layout) else {
            return null_mut();
        };
        let wanted = size.trailing_zeros() as usize;
        if wanted >= MAX_ORDER {
            return null_mut();
        }
        for order in wanted..MAX_ORDER {
            // SAFETY: 独占访问，节点来自 add_to_heap 或 dealloc。
            if let Some(block) = unsafe { self.free_list[order].pop() } {
                let address = block as usize;
                // 保留低地址一半，高地址一半放回链表，直到请求大小。
                for level in (wanted..order).rev() {
                    let buddy = (address + (1usize << level)) as *mut usize;
                    unsafe {
                        self.free_list[level].push(buddy);
                    }
                }
                self.allocated += size;
                self.peak = self.peak.max(self.allocated);
                return block.cast();
            }
        }
        null_mut()
    }
    /// SAFETY: ptr 是本分配器尚未释放的块，layout 与分配时一致。
    /// 禁止二次释放、释放外部地址或错误 layout（同 GlobalAlloc 契约）。
    pub unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        let Some(size) = Self::block_size(layout) else {
            return;
        };
        let mut order = size.trailing_zeros() as usize;
        let mut block = ptr as usize;
        // 扣除原分配块大小，不能扣除合并后的大小。
        self.allocated -= size;
        while order + 1 < MAX_ORDER {
            let buddy = block ^ (1usize << order);
            if !unsafe { self.free_list[order].remove(buddy as *mut usize) } {
                break;
            }
            block = block.min(buddy);
            order += 1;
        }
        unsafe {
            self.free_list[order].push(block as *mut usize);
        }
    }
    pub fn stats(&self) -> (usize, usize, usize) {
        (self.total, self.allocated, self.peak)
    }
}
