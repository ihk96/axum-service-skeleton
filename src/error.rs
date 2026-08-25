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

    // [아키텍처] 응답 바디에 실을 메시지. `Display`(=`to_string()`)와 일부러 분리했다.
    //
    // 4xx는 클라이언트가 스스로 고칠 수 있는 실패이므로 원인을 그대로 알려주는 게 맞다.
    // 반면 5xx의 상세는 감춘다 — 예컨대 `Database` variant의 Display는 sqlx가 만든 원문이라
    // 제약 이름이나 컬럼명, 경우에 따라 쿼리 조각까지 담고 있어서, 그대로 내보내면 외부
    // 호출자가 응답만 모아도 내부 스키마를 재구성할 수 있다. 상세는 아래 로그에만 남긴다.
    //
    // [주의] variant를 추가할 때 여기의 `_ =>`가 그것을 4xx처럼 취급해 상세를 노출한다.
    // 새 variant가 5xx라면 위 목록에 함께 넣을 것.
    fn client_message(&self) -> String {
        match self {
            AppError::Database(_) | AppError::Internal(_) => "internal server error".to_string(),
            _ => self.to_string(),
        }
    }
}

// [아키텍처] 5xx(서버 잘못)만 로그를 남기고 4xx(클라이언트 잘못)는 남기지 않는다 —
// 잘못된 요청이 들어올 때마다 서버 로그가 시끄러워지는 것을 막는다. 로그에는 `self`를
// 그대로 찍어 상세를 남기고, 클라이언트에게는 위 `client_message()`로 걸러서 보낸다.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();

        if matches!(self, AppError::Database(_) | AppError::Internal(_)) {
            tracing::error!(error = %self, "request failed");
        }

        (status, Json(json!({ "error": self.client_message() }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [테스트 시나리오] 5xx의 상세가 응답 바디로 새어나가지 않는지. sqlx 에러 원문에는
    // 제약 이름이나 컬럼명이 담기므로, 이 규칙이 깨지면 스키마가 외부로 노출된다.
    #[test]
    fn server_errors_do_not_leak_details_to_the_client() {
        let err = AppError::Database(sqlx::Error::PoolClosed);

        // Display는 상세를 유지한다 — 로그가 쓸 정보다.
        assert!(err.to_string().contains("PoolClosed") || err.to_string().contains("pool"));
        // 클라이언트로 나가는 쪽은 일반화된 문구뿐이다.
        assert_eq!(err.client_message(), "internal server error");
    }

    // [테스트 시나리오] 반대로 4xx는 원인을 그대로 알려줘야 한다 — 클라이언트가 고칠 수
    // 있는 실패이기 때문이다.
    #[test]
    fn client_errors_keep_their_reason() {
        let err = AppError::BadRequest("name must not be empty".into());

        assert_eq!(err.client_message(), "bad request: name must not be empty");
    }
}
