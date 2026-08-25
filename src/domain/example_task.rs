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

use tokio_util::sync::CancellationToken;

use crate::{error::AppError, state::AppState, store::ExampleStore};

/// 한 사이클에 할 일. 이 함수가 테스트 대상이다.
pub async fn sweep(store: &dyn ExampleStore) -> Result<usize, AppError> {
    let examples = store.list_examples().await?;
    tracing::debug!(count = examples.len(), "example sweep");
    Ok(examples.len())
}

/// `sweep`을 즉시 한 번 실행하고 고정 주기로 반복하다가, 종료 신호를 받으면 멈춘다.
// [Rust 특징] `tokio::time::interval`의 첫 틱은 "생성 즉시" 발생한다. 위에서 이미 한 번
// 수동으로 돌렸으므로 그 첫 틱을 소비해서 버린다 — 안 그러면 기동 직후 두 번 연달아 실행된다.
//
// [설명] 한 사이클이 실패해도 에러를 로그로만 남기고 루프는 계속 돈다 — 일시적 DB 장애로
// 태스크가 영구히 죽어버리면 안 되기 때문.
//
// [아키텍처] 종료는 `shutdown` 토큰으로 알린다. 여기서 진짜 중요한 건 **취소를 어디서
// 관찰하느냐**다. 아래 select!는 `cycle()`이 아니라 `ticker.tick()`과 겨룬다. 즉 취소 신호가
// 사이클 도중에 도착하면 그 사이클은 끝까지 마치고, 다음 틱을 기다리는 자리에서 루프를
// 빠져나간다. 반대로 `cycle()` 쪽에 select!를 걸면 지는 쪽 Future가 **그 자리에서 drop**되어
// 작업이 중간에 잘린다 — DB에 절반만 반영된 상태로 끝날 수 있다는 뜻이다. graceful이라는
// 말이 가리키는 게 정확히 이 차이다.
//
// 그 대가로 이 함수는 신호를 받고도 최대 한 사이클만큼 더 걸릴 수 있다. 호출자(server::run)가
// 이 태스크를 기다릴 때 상한을 두는 이유다.
pub async fn run_periodic(state: Arc<AppState>, interval: Duration, shutdown: CancellationToken) {
    let cycle = || sweep(state.example_store.as_ref());

    if let Err(err) = cycle().await {
        tracing::warn!(error = %err, "initial example sweep failed");
    }

    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = ticker.tick() => {
                if let Err(err) = cycle().await {
                    tracing::warn!(error = %err, "example sweep failed");
                }
            }
        }
    }

    tracing::info!("example sweep task stopped");
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

    // [설명] 사이클의 내용은 좁은 시그니처를 가진 sweep으로 검증한다 — 이게 진입점과 로직을
    // 나눈 이유다. 반면 run_periodic 쪽에서 검증할 것은 딱 하나, "종료 신호를 받으면 실제로
    // 멈추는가"다. 토큰을 넘겨받기만 하고 루프에서 관찰하지 않아도 컴파일은 되기 때문에,
    // 이건 타입으로 막히지 않는 종류의 실수라 테스트로 고정해 둔다.
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

    // [테스트 시나리오] 종료 신호를 받으면 루프가 실제로 빠져나오는지. 주기를 한 시간으로
    // 두어 틱이 올 일이 없게 만들었으므로, 태스크가 끝났다면 그 이유는 취소뿐이다 —
    // 만약 토큰을 관찰하지 않는다면 이 테스트는 타임아웃으로 실패한다.
    #[tokio::test]
    async fn the_loop_stops_when_the_shutdown_token_is_cancelled() {
        let mut store = MockExampleStore::new();
        store.expect_list_examples().returning(|| Ok(vec![]));

        let state = Arc::new(AppState {
            example_store: Arc::new(store),
            health_store: Arc::new(crate::store::health::MockHealthStore::new()),
        });

        let shutdown = CancellationToken::new();
        let task = tokio::spawn(run_periodic(
            state,
            Duration::from_secs(3600),
            shutdown.clone(),
        ));

        shutdown.cancel();

        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("종료 신호를 받고도 run_periodic이 멈추지 않았다")
            .expect("태스크가 패닉했다");
    }
}
