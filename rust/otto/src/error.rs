use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, OttoError>;

#[derive(Debug, Error)]
pub enum OttoError {
    #[error("{0}")]
    Usage(String),

    #[error("{0}")]
    Config(String),

    #[error("网络请求失败：{0}")]
    Network(String),

    #[error("API 请求失败：{0}")]
    Api(String),

    #[error("工具执行失败：{0}")]
    Tool(String),

    #[error("权限请求失败：{0}")]
    Permission(String),

    #[error("达到操作限制：{0}")]
    Limit(String),

    #[error("I/O 操作失败：{0}")]
    Io(#[from] io::Error),

    #[error("JSON 处理失败：{0}")]
    Json(#[from] serde_json::Error),
}

impl OttoError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Config(_) => 3,
            Self::Network(_) => 4,
            Self::Api(_) | Self::Json(_) | Self::Tool(_) | Self::Limit(_) => 5,
            Self::Permission(_) => 6,
            Self::Io(_) => 3,
        }
    }
}
