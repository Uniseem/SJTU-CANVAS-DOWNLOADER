use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
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
    #[error("上游服务暂时不可用：{0}")]
    Upstream(String),
    #[error("上游服务暂时不可用：{message}")]
    UpstreamUnavailable {
        message: String,
        retry_after_seconds: u64,
    },
    #[error("服务器内部错误")]
    Internal(#[source] anyhow::Error),
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorPayload,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorPayload {
    code: &'static str,
    message: String,
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
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            Self::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::VideoUnavailable(_) => (StatusCode::UNPROCESSABLE_ENTITY, "video_unavailable"),
            Self::VideoNotScheduled => (StatusCode::UNPROCESSABLE_ENTITY, "video_unavailable"),
            Self::Upstream(_) => (StatusCode::BAD_GATEWAY, "upstream_error"),
            Self::UpstreamUnavailable { .. } => {
                (StatusCode::SERVICE_UNAVAILABLE, "upstream_unavailable")
            }
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };

        let retry_after = match &self {
            Self::UpstreamUnavailable {
                retry_after_seconds,
                ..
            } => Some(*retry_after_seconds),
            _ => None,
        };

        if let Self::Internal(source) = &self {
            tracing::error!(error = ?source, "request failed");
        }

        let mut response = (
            status,
            Json(ErrorBody {
                error: ErrorPayload {
                    code,
                    message: self.to_string(),
                },
            }),
        )
            .into_response();
        if let Some(seconds) = retry_after
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
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
    fn unavailable_upstream_sets_retry_contract() {
        let response = AppError::upstream_unavailable("课堂视频服务维护中", 30).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("30")
        );
    }
}
