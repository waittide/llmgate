//! 测试专用共享工具（仅随 `cargo test` 编译），提供隔离的临时数据库与应用状态。

use std::path::PathBuf;

use crate::op::AppState;

/// 建立独立的临时 SQLite 与完整 AppState；`prefix` 用于隔离各测试模块的临时文件。
pub async fn setup_test_state(prefix: &str) -> (AppState, PathBuf) {
    let rnd = nanoid::nanoid!(12);
    let db_path = std::env::temp_dir().join(format!("octopus_{prefix}_test_{rnd}.db"));
    let pool = crate::db::init_db(&db_path).await.unwrap();
    let state = AppState::init(pool).await.unwrap();
    (state, db_path)
}
