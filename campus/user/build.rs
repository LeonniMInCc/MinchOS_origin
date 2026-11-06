fn main() {
    // 告诉 cargo：链接脚本是这个 crate 的构建输入。
    //
    // cargo 默认**不知道**这件事 —— 链接脚本只是通过 `-Clink-arg=-T...`
    // 传给链接器的一个字符串，改了它 cargo 照样认为产物是新鲜的，
    // 直接复用缓存里用旧脚本链接出来的二进制。
    //
    // 这个坑在第 6 周和第 8 周各踩了一次。这行是第一道防线；
    // 第二道防线在 Makefile 里 —— 构建完会核对每个程序的入口地址。
    println!("cargo:rerun-if-changed=src/linker.ld");
}
