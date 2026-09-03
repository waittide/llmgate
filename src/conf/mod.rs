use clap::Parser;
use std::path::PathBuf;

pub const APP_NAME: &str = "octopus";
pub const APP_DESC: &str = "all ai service in one place (Rust Edition)";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug, Clone)]
#[command(name = APP_NAME, about = APP_DESC, version = VERSION, disable_help_flag = true)]
pub struct Config {
    #[arg(short = 'h', long, default_value = "0.0.0.0", help = "Server listening host")]
    pub host: String,

    #[arg(short = 'p', long, default_value_t = 8080, help = "Server listening port")]
    pub port: u16,

    #[arg(short = 'd', long, default_value = "data", help = "Data directory for database and files")]
    pub data_dir: PathBuf,

    #[arg(long, default_value_t = false, help = "Enable debug logging")]
    pub debug: bool,

    /// Print help.
    #[arg(long = "help", action = clap::ArgAction::Help)]
    pub help: Option<bool>,
}

impl Config {
    pub fn parse_args() -> Self {
        Config::parse()
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("data.db")
    }
}
