// [예제] HTTP 핸들러의 모양. 하는 일이 딱 세 단계뿐인 게 요점이다:
//   요청 파싱(Json/Path 추출) → usecase 호출 → 결과를 Json으로 감싸기.
// 검증도 SQL도 트랜잭션도 여기 없다. 핸들러가 얇으면 로직이 HTTP 없이 테스트되고, 같은
// 로직을 CLI나 컨슈머에서 재사용할 수 있다.
//
// 저장소는 AppState의 두 진입점에서 꺼내 usecase가 원하는 모양으로 넘긴다.
//   - 도메인 store/view 하나: `&*state.store.examples()`, `&*state.views.examples()`
//   - 트랜잭션이 필요한 usecase: `state.store.as_ref()` (usecase가 begin/commit)
use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    routing::{post, put},
};
use uuid::Uuid;

use crate::{
    error::AppError,
    http::extract::{Json, Path},
    state::AppState,
    store::example::ExampleView,
    usecase::example::{self, CreateExampleInput, RenameExampleInput},
};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/examples", post(create_example).get(list_examples))
        .route("/examples/{id}", put(rename_example))
}

// [Rust 특징] `State(state): State<Arc<AppState>>` — axum의 추출자(extractor)를 인자 위치에서
// 바로 구조 분해한다. 반환 타입이 `Result<_, AppError>`이면 Err 쪽은 error.rs의 IntoResponse가
// 알아서 상태 코드 + JSON으로 바꿔주므로, 핸들러에 에러 처리 코드가 아예 없다.
//
// [설명] 요청 body는 usecase의 `XxxInput`을 그대로 역직렬화한다. 서버가 채워야 하는 값
// (경로의 id, 인증된 사용자 등)은 Input에 넣지 않고 인자로 따로 넘긴다.
async fn create_example(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CreateExampleInput>,
) -> Result<Json<ExampleView>, AppError> {
    Ok(Json(
        example::create_example(&*state.store.examples(), &*state.views.examples(), input).await?,
    ))
}

async fn rename_example(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(input): Json<RenameExampleInput>,
) -> Result<Json<ExampleView>, AppError> {
    Ok(Json(
        example::rename_example(state.store.as_ref(), &*state.views.examples(), id, input).await?,
    ))
}

async fn list_examples(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ExampleView>>, AppError> {
    Ok(Json(
        example::list_examples(&*state.views.examples()).await?,
    ))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode as HttpStatus, header::CONTENT_TYPE},
    };
    use tower::ServiceExt;

    use std::sync::Arc;

    use crate::{
        state::AppState,
        store::{
            MockStore, MockViews,
            example::{ExampleView, MockExampleStore, MockExampleViews},
            health::MockHealthStore,
            job_run::MockJobRunStore,
        },
    };
    use uuid::Uuid;

    // [설명] 테스트마다 AppState를 직접 조립한다. 관심 없는 필드는 expectation 없는 빈 mock으로
    // 두는데, 그건 곧 "이 경로는 그 포트를 건드리지 않는다"는 검증이다 — 빈 mock은 호출되는
    // 순간 패닉하기 때문.
    //
    // [설명] http::build()가 아니라 이 라우트 그룹만 세운다. 여기서 확인하려는 건 라우트 등록,
    // body 역직렬화, usecase 호출, 에러 → 상태 코드 매핑이다. 레이트 리밋 같은 공통 레이어는
    // 커넥션 정보(peer IP)가 필요해서 oneshot 요청으로는 세울 수 없고, 관심사도 아니다.
    fn router(store: MockStore, views: MockViews) -> axum::Router {
        super::routes().with_state(Arc::new(AppState {
            store: Arc::new(store),
            views: Arc::new(views),
            health_store: Arc::new(MockHealthStore::new()),
            job_run_store: Arc::new(MockJobRunStore::new()),
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

    // [테스트 시나리오] 라우터를 세워 요청 하나를 흘려보낸다 — 라우트 등록, 바디 역직렬화,
    // usecase 호출, 응답 직렬화까지 한 번에 검증되고 DB는 필요 없다.
    #[tokio::test]
    async fn a_valid_request_creates_an_example() {
        let id = Uuid::new_v4();

        let mut examples = MockExampleStore::new();
        examples
            .expect_insert_example()
            .times(1)
            .returning(move |_| Ok(id));
        let mut store = MockStore::new();
        store
            .expect_examples()
            .return_once(move || Box::new(examples));

        let mut example_views = MockExampleViews::new();
        example_views.expect_get_example().returning(|id| {
            Ok(Some(ExampleView {
                id,
                name: "hello".into(),
                created_at: chrono::Utc::now(),
            }))
        });
        let mut views = MockViews::new();
        views
            .expect_examples()
            .return_once(move || Box::new(example_views));

        let response = router(store, views)
            .oneshot(post_json("/examples", r#"{"name":"hello"}"#))
            .await
            .unwrap();

        assert_eq!(response.status(), HttpStatus::OK);
    }

    // [테스트 시나리오] 도메인 검증 실패가 400으로 매핑되는지 — 핸들러에 에러 처리 코드가
    // 없어도 error.rs의 IntoResponse가 이걸 보장한다는 확인. 저장소에는 닿지 않는다.
    #[tokio::test]
    async fn a_blank_name_maps_to_bad_request() {
        let mut store = MockStore::new();
        store
            .expect_examples()
            .return_once(|| Box::new(MockExampleStore::new()));
        let mut views = MockViews::new();
        views
            .expect_examples()
            .return_once(|| Box::new(MockExampleViews::new()));

        let response = router(store, views)
            .oneshot(post_json("/examples", r#"{"name":"  "}"#))
            .await
            .unwrap();

        assert_eq!(response.status(), HttpStatus::BAD_REQUEST);
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        serde_json::from_slice(&bytes).expect("error body must be JSON")
    }

    // [테스트 시나리오] JSON 파싱 실패도 핸들러 에러와 같은 형식({"error": ...})의 400이다.
    // axum 기본 추출기였다면 평문 본문이 나간다 — http::extract::Json을 쓰는 이유.
    #[tokio::test]
    async fn malformed_json_is_a_json_bad_request() {
        let response = router(MockStore::new(), MockViews::new())
            .oneshot(post_json("/examples", "{not json"))
            .await
            .unwrap();

        assert_eq!(response.status(), HttpStatus::BAD_REQUEST);
        assert!(json_body(response).await["error"].is_string());
    }

    // [테스트 시나리오] 경로 파라미터 형식 오류(uuid 자리에 아무 문자열)도 같은 형식의 400이다.
    #[tokio::test]
    async fn an_invalid_path_parameter_is_a_json_bad_request() {
        let request = Request::builder()
            .method("PUT")
            .uri("/examples/not-a-uuid")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"name":"x"}"#))
            .unwrap();

        let response = router(MockStore::new(), MockViews::new())
            .oneshot(request)
            .await
            .unwrap();

        assert_eq!(response.status(), HttpStatus::BAD_REQUEST);
        assert!(json_body(response).await["error"].is_string());
    }
}
