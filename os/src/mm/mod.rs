pub mod heap_allocator;
use self::heap_allocator::BuddyAllocator;
use crate::sync::UPSafeCell;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::addr_of_mut;

const KERNEL_HEAP_SIZE: usize = 3 * 1024 * 1024;
#[repr(C, align(4096))]
struct HeapSpace([u8; KERNEL_HEAP_SIZE]);
static mut HEAP_SPACE: HeapSpace = HeapSpace([0; KERNEL_HEAP_SIZE]);
struct KernelAllocator(UPSafeCell<BuddyAllocator>);
// SAFETY: 遵守 Layout 对齐和分配/释放契约；单核、无中断重入。
unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.exclusive_access().alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            self.0.exclusive_access().dealloc(ptr, layout);
        }
    }
}
#[global_allocator]
static HEAP: KernelAllocator = KernelAllocator(unsafe { UPSafeCell::new(BuddyAllocator::new()) });

/// 必须在 clear_bss 后、创建动态容器前调用，且只能初始化一次。
pub fn init() {
    let start = addr_of_mut!(HEAP_SPACE) as usize;
    {
        let mut allocator = HEAP.0.exclusive_access();
        assert_eq!(allocator.stats().0, 0, "heap initialized twice");
        // SAFETY: 静态堆可写，首次完整交给分配器，不与其他对象重叠。
        unsafe {
            allocator.add_to_heap(start, start + KERNEL_HEAP_SIZE);
        }
    }
    println!(
        "[INFO ] 内核堆就绪：[0x{:010x}, 0x{:010x})  {} KiB",
        start,
        start + KERNEL_HEAP_SIZE,
        KERNEL_HEAP_SIZE / 1024
    );
}

pub fn heap_test() {
    use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};
    let baseline = HEAP.0.exclusive_access().stats().1;
    assert_eq!(baseline, 0);
    let start = addr_of_mut!(HEAP_SPACE) as usize;
    let in_heap = |address: usize| address >= start && address < start + KERNEL_HEAP_SIZE;
    {
        let value = Box::new(2026u32);
        assert_eq!(*value, 2026);
        assert!(in_heap((&*value as *const u32) as usize));
        let mut values = Vec::new();
        for n in 0..500usize {
            values.push(n);
        }
        assert_eq!(values.iter().sum::<usize>(), 499 * 500 / 2);
        assert!(in_heap(values.as_ptr() as usize));
        let mut text = String::from("MinchOS");
        text.push_str(" 内核堆");
        assert_eq!(text, "MinchOS 内核堆");
        assert!(in_heap(text.as_ptr() as usize));
        let mut map = BTreeMap::new();
        for n in 0..100usize {
            map.insert(n, n * n);
        }
        for n in 0..100usize {
            assert_eq!(map.get(&n), Some(&(n * n)));
        }
        for n in 0..50usize {
            assert_eq!(map.remove(&n), Some(n * n));
        }
        assert_eq!(map.len(), 50);
        println!("[INFO ] Box / Vec / String / BTreeMap 自检通过");
    } // Drop 自动释放所有容器。
    assert_eq!(HEAP.0.exclusive_access().stats().1, baseline);

    // 直接测试底层分配器，不用动态容器存地址，避免测试干扰统计。
    {
        let mut allocator = HEAP.0.exclusive_access();
        let whole = Layout::from_size_align(4096, 4096).unwrap();
        let half = Layout::from_size_align(2048, 2048).unwrap();
        let parent = allocator.alloc(whole);
        assert!(!parent.is_null());
        unsafe {
            allocator.dealloc(parent, whole);
        }
        let left = allocator.alloc(half);
        let right = allocator.alloc(half);
        assert!(!left.is_null() && !right.is_null());
        assert_eq!((left as usize) ^ 2048, right as usize);
        unsafe {
            allocator.dealloc(left, half);
            allocator.dealloc(right, half);
        }
        let merged = allocator.alloc(whole);
        assert_eq!(merged, parent);
        unsafe {
            allocator.dealloc(merged, whole);
        }
        let aligned = Layout::from_size_align(33, 8192).unwrap();
        let block = allocator.alloc(aligned);
        assert!(!block.is_null());
        assert_eq!(block as usize % 8192, 0);
        unsafe {
            allocator.dealloc(block, aligned);
        }
        let too_big = Layout::from_size_align(KERNEL_HEAP_SIZE * 2, 8).unwrap();
        assert!(allocator.alloc(too_big).is_null());
        assert_eq!(allocator.stats().1, baseline);
    }
    let (total, used, peak) = HEAP.0.exclusive_access().stats();
    println!("[INFO ] 伙伴合并、对齐与内存不足检查通过");
    println!(
        "[INFO ] 堆自检通过：容量 {} KiB，峰值占用 {} B，释放后回落至 {} B",
        total / 1024,
        peak,
        used
    );
}
