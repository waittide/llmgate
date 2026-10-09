use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
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

    // 同时支持环境变量，容器里改挂载点无需改命令行：
    // `docker run -e LLMGATE_DATA_DIR=/data -v 宿主目录:/data`
    #[arg(
        short = 'd',
        long,
        env = "LLMGATE_DATA_DIR",
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

    /// 启动自检：确保数据目录存在，且当前用户确实能往里写。
    ///
    /// 容器部署最常见的坑是把宿主机目录挂进来、但属主不是容器内的运行用户
    /// （镜像里是 uid/gid 1000）。不提前检查的话，问题要到 sqlite 打不开
    /// 数据库时才暴露，报错与真实原因相去甚远。
    pub fn ensure_data_dir(&self) -> Result<()> {
        ensure_writable_dir(&self.data_dir)
    }
}

/// 见 [`Config::ensure_data_dir`]。
fn ensure_writable_dir(dir: &Path) -> Result<()> {
    const HINT: &str = concat!(
        "容器部署请确认挂载目录的属主与容器内运行用户一致（镜像内为 uid/gid 1000）：\n",
        "  mkdir -p <宿主机目录> && sudo chown -R 1000:1000 <宿主机目录>",
    );

    fs::create_dir_all(dir).map_err(|e| anyhow::anyhow!("无法创建数据目录 {}: {e}\n{HINT}", dir.display()))?;

    // 判据是「真的写一个文件」，而不是看权限位：只读挂载、ACL、SELinux
    // 都能让权限位看起来一切正常。探针文件用完即删，不留在数据目录里。
    let probe = dir.join(".write-probe");
    fs::write(&probe, b"").map_err(|e| anyhow::anyhow!("数据目录 {} 不可写: {e}\n{HINT}", dir.display()))?;
    let _ = fs::remove_file(&probe);
    Ok(())
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

    /// 临时目录下一个唯一路径，供数据目录自检用例使用。
    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("llmgate_conf_{tag}_{}", nanoid::nanoid!(10)))
    }

    /// 目录不存在时会被创建，且不留下探针文件。
    #[test]
    fn ensure_data_dir_creates_missing_dir() {
        let dir = temp_path("create");
        ensure_writable_dir(&dir).unwrap();
        assert!(dir.is_dir());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "探针文件应已删除");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// 目标路径被文件占住时立刻报错，而不是拖到打开数据库时才炸。
    #[test]
    fn ensure_data_dir_rejects_path_taken_by_file() {
        let path = temp_path("file");
        fs::write(&path, b"x").unwrap();

        let err = ensure_writable_dir(&path).unwrap_err().to_string();
        assert!(err.contains("无法创建数据目录"), "{err}");
        assert!(err.contains("chown"), "报错应给出属主修复提示: {err}");

        fs::remove_file(&path).unwrap();
    }

    /// 目录不可写时报错并给出属主提示。
    ///
    /// 以 root 运行时写探针照样成功，这个失败场景没法在测试里构造，故直接跳过。
    #[cfg(unix)]
    #[test]
    fn ensure_data_dir_rejects_readonly_dir() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_path("readonly");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();

        let result = ensure_writable_dir(&dir);
        if result.is_ok() {
            return;
        }
        let err = result.unwrap_err().to_string();
        assert!(err.contains("不可写"), "{err}");
        assert!(err.contains("chown"), "报错应给出属主修复提示: {err}");

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}
