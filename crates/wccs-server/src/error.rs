use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
pub struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}
impl ApiError {
    pub fn new(status: u16, message: &str) -> Self {
        Self {
            status: StatusCode::from_u16(status).unwrap(),
            message: message.into(),
        }
    }
    pub fn bad(message: &str) -> Self {
        Self::new(400, message)
    }
    pub fn unauthorized(message: &str) -> Self {
        Self::new(401, message)
    }
    pub fn forbidden(message: &str) -> Self {
        Self::new(403, message)
    }
    pub fn conflict(message: &str) -> Self {
        Self::new(409, message)
    }
    pub fn internal() -> Self {
        Self::new(500, "服务器存储或内部操作失败，请重试并查看服务日志")
    }
    pub fn unavailable() -> Self {
        Self::new(503, "数据库连接暂时不可用，可使用相同请求编号重试")
    }
}
impl From<tokio_postgres::Error> for ApiError {
    fn from(error: tokio_postgres::Error) -> Self {
        eprintln!("database_error code={:?}", error.code());
        if error.is_closed()
            || std::error::Error::source(&error).is_some_and(|source| source.is::<std::io::Error>())
            || error.code().is_some_and(|code| {
                code.code().starts_with("08") || matches!(code.code(), "57P01" | "57P02" | "57P03")
            })
        {
            return Self::unavailable();
        }
        Self::internal()
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}
