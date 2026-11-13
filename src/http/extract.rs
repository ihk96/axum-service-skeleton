// [아키텍처] axum 추출기를 이 서비스의 에러 형식에 맞춘 것들. 핸들러는 axum의 `Json`/`Path`
// 대신 여기의 것을 쓴다(`use crate::http::extract::{Json, Path}`).
//
// [설명] axum 기본 추출기는 파싱에 실패하면 **평문**으로 응답한다("Failed to parse the request
// body as JSON: ..."). 그러면 클라이언트는 핸들러 에러(`{"error": ...}`)와 파싱 에러를 다른
// 형식으로 처리해야 한다. 여기서 실패를 `AppError::BadRequest`로 바꿔 형식을 하나로 맞춘다.
// 상태 코드도 400으로 모은다 — axum은 원인에 따라 400/415/422로 나누지만, 클라이언트가 보낸
// 요청이 잘못됐다는 사실은 같고 상세는 메시지에 남는다.
//
// 요청에서 값을 뽑는 extractor(`FromRequestParts` 구현)가 더 생기면(예: 게이트웨이가 넣어준
// 사용자 헤더) 이 파일을 폴더로 바꿔 함께 둔다.
use axum::{
    extract::{
        FromRequest, FromRequestParts,
        rejection::{JsonRejection, PathRejection},
    },
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::error::AppError;

/// JSON 요청 body / 응답. 요청 파싱에 실패하면 400 JSON 에러가 된다.
// [Rust 특징] `via(axum::Json)`은 "추출은 axum::Json에 맡기고, 실패(rejection)만 AppError로
// 바꾼다"는 뜻이다. 아래 `From<JsonRejection> for AppError`가 그 변환이다.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
pub struct Json<T>(pub T);

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// 경로 파라미터. 형식이 맞지 않으면(예: uuid 자리에 아무 문자열) 400 JSON 에러가 된다.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(AppError))]
pub struct Path<T>(pub T);

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        AppError::BadRequest(rejection.body_text())
    }
}

impl From<PathRejection> for AppError {
    fn from(rejection: PathRejection) -> Self {
        AppError::BadRequest(rejection.body_text())
    }
}
