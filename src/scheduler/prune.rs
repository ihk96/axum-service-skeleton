// [아키텍처] 스케줄러가 자기 실행 기록을 자기가 청소하는 job. 이 job은 usecase/jobs.rs에
// 없다 — scheduled_job_runs는 스케줄러의 살림살이지 이 서비스의 업무가 아니기 때문이다.
// 등록부에는 "이 서비스가 무엇을 언제 돌리는가"만 남아야 한다.
//
// 대신 **얼마나 보관할지**는 서비스가 정할 값이라 설정(AppConfig)에서 받는다.
use std::sync::Arc;

use chrono::{TimeDelta, Utc};
use chrono_tz::Tz;

use crate::state::AppState;

use super::registry::{JobFuture, JobRegistry};

/// 스케줄러가 기동할 때 자기 job을 스스로 등록한다.
// [설명] 시각을 UTC로 못박은 것은 이 job이 서비스 업무가 아니라 유지보수라서다. 어느
// 시간대의 새벽에 도는지는 중요하지 않고, 배포 지역이 바뀌어도 동작이 같은 편이 낫다.
pub(super) fn register(registry: &mut JobRegistry, retain: TimeDelta) {
    registry.add(
        "scheduler_prune_runs",
        "30 4 * * *",
        Tz::UTC,
        prune_runs(retain),
    );
}

/// `retain`보다 오래된 실행 기록을 지우는 job을 만든다.
// [설명] 그 테이블은 발화마다 한 줄씩 쌓여서(1분 주기 job 하나면 하루 1440행) 방치하면
// 계속 자란다.
//
// [Rust 특징] job 함수를 그냥 `async fn`으로 두지 않고 **함수를 반환하는 함수**로 만들었다.
// 인자가 필요한 job은 이 모양을 쓰면 된다 — 설정값을 클로저에 담아 두면 등록 시점에 값이
// 정해지고, job 함수 자체의 시그니처는 다른 job과 똑같이 유지된다.
fn prune_runs(retain: TimeDelta) -> impl Fn(Arc<AppState>) -> JobFuture + Send + Sync {
    move |state| {
        Box::pin(async move {
            let cutoff = Utc::now() - retain;
            let deleted = state.job_run_store.delete_runs_before(cutoff).await?;

            tracing::info!(deleted, cutoff = %cutoff, "pruned scheduled job runs");
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MockStore, MockViews, health::MockHealthStore, job_run::MockJobRunStore};

    // [테스트 시나리오] 보관 기간이 실제 cutoff로 옮겨지는지. 부호를 뒤집으면 "30일보다
    // 오래된 것"이 아니라 사실상 전부를 지우게 되는데, 그건 조용히 성공하는 종류의 사고다 —
    // 게다가 이 테이블은 기록이자 락이라 최근 행이 사라지면 중복 실행까지 열린다.
    #[tokio::test]
    async fn the_cutoff_is_the_retention_window_before_now() {
        let before = Utc::now();

        let mut store = MockJobRunStore::new();
        store
            .expect_delete_runs_before()
            .withf(move |cutoff| {
                let expected = before - TimeDelta::days(30);
                (*cutoff - expected).abs() < TimeDelta::seconds(5)
            })
            .return_once(|_| Ok(3));

        let state = Arc::new(AppState {
            store: Arc::new(MockStore::new()),
            views: Arc::new(MockViews::new()),
            health_store: Arc::new(MockHealthStore::new()),
            job_run_store: Arc::new(store),
        });

        prune_runs(TimeDelta::days(30))(state).await.unwrap();
    }
}
