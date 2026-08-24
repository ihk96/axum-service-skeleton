use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde_json::json;

use crate::state::AppState;

// [아키텍처] liveness/readiness를 분리하는 건 쿠버네티스 등 오케스트레이터의 표준 관용구다 —
// 두 프로브가 "실패했을 때 취해야 할 조치"가 다르기 때문에 엔드포인트 자체를 나눈다.
// 이 파일은 서비스 성격과 무관하게 그대로 쓸 수 있다(readiness가 확인할 의존성만 늘려가면 된다).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(liveness))
        .route("/ready", get(readiness))
}

/// Liveness: 프로세스가 살아서 요청을 처리하고 있는가.
// [설명] **의도적으로 아무 의존성도 확인하지 않는다.** 여기서 실패한다는 건 "프로세스를
// 재시작해야 한다"는 뜻이어야 하는데, DB 장애는 재시작으로 회복되는 문제가 아니다. 여기서
// DB를 찔러버리면 DB 장애 때 오케스트레이터가 파드를 계속 재시작해 crash loop가 난다 —
// 이 규칙이 이 파일에서 가장 중요한 내용이다.
async fn liveness() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

/// Readiness: 실제로 트래픽을 처리할 수 있는가(의존성에 닿는가).
// [설명] 여기서 실패한다는 건 "이쪽으로 트래픽을 더 보내지 마라"는 뜻이고, 그게 의존성
// 장애에 대한 올바른 대응이다 — 재시작이 아니라. 의존성이 늘어나면(캐시, 이벤트 버스 등)
// 이 함수가 확인할 대상도 함께 늘어난다.
async fn readiness(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.health_store.ping().await {
        Ok(_) => (StatusCode::OK, Json(json!({ "status": "ok" }))),
        Err(err) => {
            tracing::warn!(error = %err, "readiness check failed: database unreachable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "unavailable", "reason": "database unreachable" })),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode as HttpStatus},
    };
    use tower::ServiceExt;

    use std::sync::Arc;

    use crate::{
        error::AppError,
        state::AppState,
        store::{example::MockExampleStore, health::MockHealthStore},
    };

    // [설명] 테스트마다 AppState를 직접 조립한다. 관심 없는 필드에 expectation 없는 빈
    // mock을 넣는 게 요점 — 빈 mock은 호출되면 패닉하므로, 그 자체가 "이 경로는 그 포트를
    // 건드리지 않는다"는 검증이다. 편하다고 permissive한 mock을 넣으면 그 성질이 사라진다.
    fn router(health_store: MockHealthStore) -> axum::Router {
        crate::routes::build(Arc::new(AppState {
            example_store: Arc::new(MockExampleStore::new()),
            health_store: Arc::new(health_store),
        }))
    }

    // [테스트 시나리오] liveness가 어떤 의존성도 건드리지 않는지 확인한다. 두 포트 모두
    // 빈 mock이므로, 이 경로가 저장소를 부르면 패닉하며 실패한다 — 즉 "의존성을 보지
    // 않는다"는 규칙이 테스트로 고정된다.
    #[tokio::test]
    async fn liveness_does_not_touch_any_dependency() {
        let response = router(MockHealthStore::new())
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), HttpStatus::OK);
    }

    // [테스트 시나리오] 저장소가 죽으면 readiness가 503을 반환하는지.
    #[tokio::test]
    async fn readiness_reports_unavailable_when_the_store_is_down() {
        let mut store = MockHealthStore::new();
        store
            .expect_ping()
            .returning(|| Err(AppError::Database(sqlx::Error::PoolClosed)));

        let response = router(store)
            .oneshot(Request::builder().uri("/ready").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), HttpStatus::SERVICE_UNAVAILABLE);
    }
}
