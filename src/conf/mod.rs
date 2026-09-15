use std::path::PathBuf;

use clap::Parser;

pub const APP_NAME: &str = "LLMGate";
pub const APP_DESC: &str = "全能 AI 服务网关 (Rust 版)";

/// 本构建的版本标识。
///
/// 优先取 CI 注入的构建标识（`版本号+提交短hash`，如 `0.2.0+1a2b3c4`），
/// 便于把任何一个发布出去的二进制对应回具体提交；本地构建未注入时
/// 回退到 Cargo 包版本。
pub const VERSION: &str = match option_env!("LLMGATE_BUILD_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Parser, Debug, Clone)]
#[command(name = APP_NAME, about = APP_DESC, version = VERSION)]
pub struct Config {
    #[arg(long, default_value = "0.0.0.0", help = "服务监听的主机地址")]
    pub host: String,

    #[arg(short = 'p', long, default_value_t = 8080, help = "服务监听端口")]
    pub port: u16,

    #[arg(short = 'd', long, default_value = "data", help = "数据库与文件存储目录")]
    pub data_dir: PathBuf,

    #[arg(long, default_value_t = false, help = "开启 Debug 级别调试日志")]
    pub debug: bool,
}

impl Config {
    pub fn parse_args() -> Self {
        Config::parse()
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("data.db")
    }
}
