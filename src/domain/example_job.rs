// [예제] 스케줄러가 돌리는 job 하나. 백그라운드에서 도는 코드는 전부 이 규칙을 따른다:
// **진입점(run)은 `Arc<AppState>`를 통째로 받고, 실제 로직(summarize)은 필요한 포트만
// `&dyn`으로 좁게 받는다.** 그래서 job이 나중에 포트를 하나 더 쓰게 되어도 등록부
// (domain/jobs.rs)는 손댈 필요가 없고, 로직 쪽은 mock 하나로 테스트가 끝난다.
use std::sync::Arc;

use crate::{error::AppError, state::AppState, store::ExampleStore};

/// 스케줄러에 등록되는 진입점. 시그니처가 `scheduler::Job`이 요구하는 모양이다.
pub async fn run(state: Arc<AppState>) -> Result<(), AppError> {
    let summary = summarize(state.example_store.as_ref()).await?;
    tracing::info!(summary, "daily example report");
    Ok(())
}

/// 한 번의 실행이 하는 일. 이 함수가 테스트 대상이다.
// [설명] job 본체가 `Err`를 반환하면 스케줄러가 그것을 실행 기록(scheduled_job_runs.error)에
// 남긴다. 그러니 실패를 여기서 삼켜 `Ok`로 만들지 말 것 — 삼키면 "실패한 적 없는 배치"가 된다.
async fn summarize(store: &dyn ExampleStore) -> Result<String, AppError> {
    let examples = store.list_examples().await?;
    Ok(format!("{} examples", examples.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{example::MockExampleStore, ExampleRecord};
    use chrono::Utc;
    use uuid::Uuid;

    // [테스트 시나리오] 저장소 실패가 job의 실패로 그대로 올라와야 한다. 여기서 에러를
    // 삼키면 스케줄러는 성공으로 기록하고, 배치가 며칠째 아무 일도 안 하는 걸 아무도 모른다.
    #[tokio::test]
    async fn a_storage_failure_fails_the_job() {
        let mut store = MockExampleStore::new();
        store
            .expect_list_examples()
            .return_once(|| Err(AppError::Database(sqlx::Error::PoolClosed)));

        assert!(matches!(
            summarize(&store).await.unwrap_err(),
            AppError::Database(_)
        ));
    }

    #[tokio::test]
    async fn the_summary_counts_what_the_store_returned() {
        let mut store = MockExampleStore::new();
        store.expect_list_examples().return_once(|| {
            Ok(vec![ExampleRecord {
                id: Uuid::new_v4(),
                name: "first".into(),
                created_at: Utc::now(),
            }])
        });

        assert_eq!(summarize(&store).await.unwrap(), "1 examples");
    }
}
