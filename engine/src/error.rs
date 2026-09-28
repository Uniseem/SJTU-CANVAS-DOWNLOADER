//! Errors of school requests and user actions, with the codes the apps see.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("请先扫码登录 Canvas")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    VideoUnavailable(String),
    #[error("新视频平台未找到该课程的直录播安排")]
    VideoNotScheduled,
    #[error("学校服务暂时不可用：{0}")]
    Upstream(String),
    #[error("学校服务暂时不可用：{message}")]
    UpstreamUnavailable {
        message: String,
        retry_after_seconds: u64,
    },
    #[error("内部错误：{0:#}")]
    Internal(#[source] anyhow::Error),
}

impl AppError {
    pub fn internal(error: impl Into<anyhow::Error>) -> Self {
        Self::Internal(error.into())
    }

    pub fn upstream_unavailable(message: impl Into<String>, retry_after_seconds: u64) -> Self {
        Self::UpstreamUnavailable {
            message: message.into(),
            retry_after_seconds: retry_after_seconds.max(1),
        }
    }

    /// The JSON-RPC error code shown to the apps.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "invalid_params",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::VideoUnavailable(_) | Self::VideoNotScheduled => "video_unavailable",
            Self::Upstream(_) => "upstream_error",
            Self::UpstreamUnavailable { .. } => "upstream_unavailable",
            Self::Internal(_) => "internal",
        }
    }

    /// Seconds after which an automatic retry makes sense.
    pub fn retry_after(&self) -> Option<u64> {
        match self {
            Self::UpstreamUnavailable {
                retry_after_seconds,
                ..
            } => Some(*retry_after_seconds),
            _ => None,
        }
    }

    /// Temporary school or network trouble: worth trying again later.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Upstream(_) | Self::UpstreamUnavailable { .. })
    }
}

impl From<reqwest::Error> for AppError {
    fn from(error: reqwest::Error) -> Self {
        let message = if error.is_timeout() {
            "请求学校服务超时"
        } else if error.is_connect() {
            "无法连接学校服务"
        } else if error.is_decode() {
            "学校服务返回了无法识别的数据"
        } else if error.is_body() {
            "读取学校服务响应时连接中断"
        } else {
            "学校服务请求失败"
        };
        Self::Upstream(message.into())
    }
}

impl From<url::ParseError> for AppError {
    fn from(error: url::ParseError) -> Self {
        Self::BadRequest(format!("地址格式无效：{error}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_upstream_carries_a_retry_delay() {
        let error = AppError::upstream_unavailable("课堂视频服务维护中", 30);
        assert_eq!(error.code(), "upstream_unavailable");
        assert_eq!(error.retry_after(), Some(30));
        assert!(error.is_transient());
        assert_eq!(
            AppError::upstream_unavailable("x", 0).retry_after(),
            Some(1)
        );
        assert!(!AppError::Unauthorized.is_transient());
    }
}
