use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

// [아키텍처] 애플리케이션 전체에서 쓰는 단일 에러 타입. domain/ 함수들은 전부
// `Result<T, AppError>`를 반환하고, 라우트 핸들러가 그걸 그대로 반환하면 아래
// `IntoResponse`가 적절한 상태 코드 + JSON 바디로 바꿔준다 — 핸들러마다 "이 에러는
// 몇 번으로 응답할지"를 반복해 적을 필요가 없다.
//
// variant는 서비스에 맞게 늘리거나 줄인다. 다만 "HTTP 상태 코드 하나당 variant 하나"로
// 만들지 말 것 — 도메인이 구분하고 싶은 실패의 종류를 담고, HTTP 매핑은 그 결과일 뿐이다.
//
// [Rust 특징] `#[from]`이 붙은 variant는 `From` 변환도 생성되므로, 도메인 코드에서
// `?`로 sqlx 에러를 던지면 자동으로 `AppError::Database`로 감싸진다.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    // [Rust 특징] 소진적 match — variant를 새로 추가하면 컴파일러가 이 분기가 빠졌다고
    // 알려주므로 "에러 타입은 늘렸는데 상태 코드 매핑을 깜빡하는" 실수가 컴파일 타임에 걸린다.
    pub fn status_code(&self) -> StatusCode {
        match self {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Database(_) | AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

// [아키텍처] 5xx(서버 잘못)만 로그를 남기고 4xx(클라이언트 잘못)는 남기지 않는다 —
// 잘못된 요청이 들어올 때마다 서버 로그가 시끄러워지는 것을 막는다.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();

        if matches!(self, AppError::Database(_) | AppError::Internal(_)) {
            tracing::error!(error = %self, "request failed");
        }

        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
