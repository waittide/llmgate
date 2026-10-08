use std::{env, ffi::OsString, path::PathBuf};

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

    #[arg(
        short = 'd',
        long,
        default_value_os_t = default_data_dir(),
        help = "数据库与文件存储目录（默认 ~/.config/llmgate）"
    )]
    pub data_dir: PathBuf,

    #[arg(long, default_value_t = false, help = "开启 Debug 级别调试日志")]
    pub debug: bool,
}

/// 默认数据目录：`~/.config/llmgate`。取不到用户主目录（如容器内无 HOME）时
/// 退回相对目录 `data`，与原行为一致，可用 `-d` 显式覆盖。
fn default_data_dir() -> PathBuf {
    data_dir_from_home(env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")))
}

fn data_dir_from_home(home: Option<OsString>) -> PathBuf {
    match home {
        Some(home) if !home.is_empty() => PathBuf::from(home).join(".config").join("llmgate"),
        _ => PathBuf::from("data"),
    }
}

impl Config {
    pub fn parse_args() -> Self {
        Config::parse()
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("data.db")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认数据目录跟随用户主目录；主目录缺失或为空时退回相对目录 `data`。
    #[test]
    fn default_data_dir_follows_home() {
        assert_eq!(
            data_dir_from_home(Some(OsString::from("/home/u"))),
            PathBuf::from("/home/u/.config/llmgate")
        );
        assert_eq!(data_dir_from_home(Some(OsString::new())), PathBuf::from("data"));
        assert_eq!(data_dir_from_home(None), PathBuf::from("data"));
    }
}
