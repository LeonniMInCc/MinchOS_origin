fn main() {
    // logging.rs 用 option_env!("LOG") 在编译期决定日志级别。
    // 没有这行的话，改了 LOG 也不会触发重新编译，日志级别会诡异地不生效。
    println!("cargo:rerun-if-env-changed=LOG");
    println!("cargo:rerun-if-changed=src/linker.ld");
    println!("cargo:rerun-if-changed=src/entry.asm");
}
