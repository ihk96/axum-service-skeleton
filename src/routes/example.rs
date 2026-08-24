// [예제] HTTP 핸들러의 모양. 하는 일이 딱 세 단계뿐인 게 요점이다:
//   요청 파싱(Json/Path 추출) → domain 함수 호출 → 결과를 Json으로 감싸기.
// 검증도 SQL도 여기 없다. 핸들러가 얇으면 도메인 로직이 HTTP 없이 테스트되고, 같은 로직을
// CLI나 컨슈머에서 재사용할 수 있다.
use std::sync::Arc;

use axum::{extract::State, routing::post, Json, Router};
use serde::Deserialize;

use crate::{
    error::AppError,
    example,
    state::AppState,
    store::ExampleRecord,
};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/examples", post(create_example).get(list_examples))
}

#[derive(Deserialize)]
struct CreateExampleBody {
    name: String,
}

// [Rust 특징] `State(state): State<Arc<AppState>>` — axum의 추출자(extractor)를 인자 위치에서
// 바로 구조 분해한다. 반환 타입이 `Result<_, AppError>`이면 Err 쪽은 error.rs의 IntoResponse가
// 알아서 상태 코드 + JSON으로 바꿔주므로, 핸들러에 에러 처리 코드가 아예 없다.
async fn create_example(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateExampleBody>,
) -> Result<Json<ExampleRecord>, AppError> {
    Ok(Json(
        example::create_example(state.example_store.as_ref(), &body.name).await?,
    ))
}

async fn list_examples(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ExampleRecord>>, AppError> {
    Ok(Json(example::list_examples(state.example_store.as_ref()).await?))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{header::CONTENT_TYPE, Request, StatusCode as HttpStatus},
    };
    use tower::ServiceExt;

    use std::sync::Arc;

    use crate::{
        state::AppState,
        store::{example::MockExampleStore, health::MockHealthStore, ExampleRecord},
    };
    use uuid::Uuid;

    // [설명] 테스트마다 AppState를 직접 조립한다. health_store는 이 경로가 쓰지 않으므로
    // expectation 없는 빈 mock으로 두는데, 그건 곧 "이 경로는 그 포트를 건드리지 않는다"는
    // 검증이다 — 빈 mock은 호출되는 순간 패닉하기 때문.
    fn router(example_store: MockExampleStore) -> axum::Router {
        crate::routes::build(Arc::new(AppState {
            example_store: Arc::new(example_store),
            health_store: Arc::new(MockHealthStore::new()),
        }))
    }

    fn post_json(uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    // [테스트 시나리오] 라우터를 통째로 세워 요청 하나를 흘려보낸다 — 라우트 등록, 바디
    // 역직렬화, 도메인 호출, 응답 직렬화까지 한 번에 검증되고 DB는 필요 없다.
    #[tokio::test]
    async fn a_valid_request_creates_an_example() {
        let mut store = MockExampleStore::new();
        store
            .expect_insert_example()
            .times(1)
            .returning(|name| {
                Ok(ExampleRecord {
                    id: Uuid::new_v4(),
                    name: name.to_string(),
                    created_at: chrono::Utc::now(),
                })
            });

        let response = router(store)
            .oneshot(post_json("/examples", r#"{"name":"hello"}"#))
            .await
            .unwrap();

        assert_eq!(response.status(), HttpStatus::OK);
    }

    // [테스트 시나리오] 도메인 검증 실패가 400으로 매핑되는지 — 핸들러에 에러 처리 코드가
    // 없어도 error.rs의 IntoResponse가 이걸 보장한다는 확인.
    #[tokio::test]
    async fn a_blank_name_maps_to_bad_request() {
        let response = router(MockExampleStore::new())
            .oneshot(post_json("/examples", r#"{"name":"  "}"#))
            .await
            .unwrap();

        assert_eq!(response.status(), HttpStatus::BAD_REQUEST);
    }
}
