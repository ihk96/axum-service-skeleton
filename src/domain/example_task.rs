// [예제] 백그라운드 태스크의 모양. 여기서 지켜야 할 규칙이 이 스켈레톤의 핵심 중 하나다:
//
//   진입점(run_periodic)은 `Arc<AppState>`를 통째로 받고, 실제 로직(sweep)은 필요한 포트만
//   `&dyn`으로 좁게 받는다.
//
// 이렇게 나누면 양쪽 이득을 다 가진다. 진입점이 state를 받으므로 이 태스크가 나중에 다른
// 포트를 더 쓰게 되어도 server::run의 호출부는 손댈 필요가 없고, 로직은 좁은 시그니처라
// 테스트가 mock 하나로 끝나며 시그니처 자체가 의존성 문서가 된다.
//
// kafka 컨슈머 루프도 구조가 정확히 같다 — 진입점에서 state를 받고, 메시지 하나를 처리하는
// 함수는 좁은 포트만 받는다.
use std::{sync::Arc, time::Duration};

use crate::{error::AppError, state::AppState, store::ExampleStore};

/// 한 사이클에 할 일. 이 함수가 테스트 대상이다.
pub async fn sweep(store: &dyn ExampleStore) -> Result<usize, AppError> {
    let examples = store.list_examples().await?;
    tracing::debug!(count = examples.len(), "example sweep");
    Ok(examples.len())
}

/// `sweep`을 즉시 한 번 실행하고, 이후 고정 주기로 영원히 반복한다.
// [Rust 특징] `tokio::time::interval`의 첫 틱은 "생성 즉시" 발생한다. 위에서 이미 한 번
// 수동으로 돌렸으므로 그 첫 틱을 소비해서 버린다 — 안 그러면 기동 직후 두 번 연달아 실행된다.
// [설명] 이 함수는 정상 종료하지 않는다. 프로세스가 죽을 때까지 도는 게 의도된 동작이고,
// server::run이 tokio::spawn으로 백그라운드에 던져둔다. 한 사이클이 실패해도 에러를 로그로만
// 남기고 루프는 계속 돈다 — 일시적 DB 장애로 태스크가 영구히 죽어버리면 안 되기 때문.
pub async fn run_periodic(state: Arc<AppState>, interval: Duration) {
    let cycle = || sweep(state.example_store.as_ref());

    if let Err(err) = cycle().await {
        tracing::warn!(error = %err, "initial example sweep failed");
    }

    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        if let Err(err) = cycle().await {
            tracing::warn!(error = %err, "example sweep failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{example::MockExampleStore, ExampleRecord};
    use uuid::Uuid;

    fn record(name: &str) -> ExampleRecord {
        ExampleRecord {
            id: Uuid::new_v4(),
            name: name.to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    // [테스트 시나리오] run_periodic은 무한 루프라 테스트하지 않는다 — 테스트 대상은 좁은
    // 시그니처를 가진 sweep이고, 이게 진입점과 로직을 나눈 이유이기도 하다.
    #[tokio::test]
    async fn the_sweep_reports_how_many_examples_it_saw() {
        let mut store = MockExampleStore::new();
        store
            .expect_list_examples()
            .returning(|| Ok(vec![record("a"), record("b")]));

        assert_eq!(sweep(&store).await.unwrap(), 2);
    }

    // [테스트 시나리오] 저장소 실패는 삼키지 않고 전파한다 — 호출자(run_periodic)가 그
    // 사실을 로그로 남길 수 있어야 한다.
    #[tokio::test]
    async fn a_failing_store_propagates() {
        let mut store = MockExampleStore::new();
        store
            .expect_list_examples()
            .returning(|| Err(AppError::Database(sqlx::Error::PoolClosed)));

        assert!(matches!(sweep(&store).await.unwrap_err(), AppError::Database(_)));
    }
}
