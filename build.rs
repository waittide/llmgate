//! 注入构建版本标识。
//!
//! CI 由 `.github/workflows/build.yml` 传入 `LLMGATE_BUILD_VERSION`，形如
//! `0.2.0+1a2b3c4`（版本号 + 提交短 hash）。透传给 rustc 后，
//! [`crate::conf::VERSION`] 用 `option_env!` 读取并对外上报。
//!
//! 本地 `cargo build` 没有该变量，`option_env!` 得到 `None`，
//! 常量回退到 `CARGO_PKG_VERSION`，因此本地构建行为完全不变。

fn main() {
    println!("cargo:rerun-if-env-changed=LLMGATE_BUILD_VERSION");

    if let Ok(version) = std::env::var("LLMGATE_BUILD_VERSION") {
        let version = version.trim();
        if !version.is_empty() {
            println!("cargo:rustc-env=LLMGATE_BUILD_VERSION={version}");
        }
    }
}
