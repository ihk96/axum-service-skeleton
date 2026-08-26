// [아키텍처] 실행 기반. 크론식 파싱, job별 루프, 분산 락이 여기 있고 **등록은 여기서 하지
// 않는다** — 등록 창구는 registry.rs이고, 무엇을 언제 돌릴지는 domain/jobs.rs다.
use std::{collections::HashSet, str::FromStr, sync::Arc, time::Duration};

use anyhow::Context;
use chrono::{DateTime, TimeDelta, Utc};
use chrono_tz::Tz;
use croner::Cron;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::{config::AppConfig, state::AppState};

use super::{
    prune,
    registry::{JobFuture, JobRegistry},
};

// [설명] 다음 발화까지 몇 시간이 남았더라도 한 번에 이만큼씩만 잔다. `tokio::time::sleep`은
// 단조 시계(monotonic clock)를 쓰므로, 노트북이 절전에서 깨거나 NTP가 시계를 크게 당기면
// "12시간 뒤"가 실제 벽시계로는 엉뚱한 시각이 된다. 짧게 끊어 자고 매번 벽시계로 남은
// 시간을 다시 계산하면 그 어긋남이 최대 이 값만큼으로 제한된다.
const MAX_SLEEP: Duration = Duration::from_secs(60);

/// 등록된 job들을 각자의 크론식대로 돌리는 실행기.
pub struct Scheduler {
    enabled: bool,
    jobs: Vec<ScheduledJob>,
}

struct ScheduledJob {
    name: &'static str,
    cron: Cron,
    tz: Tz,
    run: Box<dyn Fn(Arc<AppState>) -> JobFuture + Send + Sync>,
}

impl Scheduler {
    /// 등록부를 받아 스케줄러를 조립한다. 호출부는 `Scheduler::build(&cfg, jobs::register)`
    /// 한 줄이면 되고, job이 늘어도 이 시그니처는 그대로다.
    // [아키텍처] 등록부를 값(`Vec<Job>`)이 아니라 **함수**로 받는 이유는, 스케줄러가 자기
    // job(prune)을 먼저 등록한 뒤 창구를 넘겨줘야 하기 때문이다. 그래서 스케줄러의 유지보수
    // job이 domain/jobs.rs에 드러나지 않는다.
    pub fn build(cfg: &AppConfig, register: impl FnOnce(&mut JobRegistry)) -> anyhow::Result<Self> {
        // [주의] 보관 기간이 0 이하면 prune이 방금 끝난 발화의 행까지 지운다. 그 행은
        // 기록이자 **락**이라, 사라지면 같은 발화를 다른 인스턴스가 다시 선점할 수 있다.
        anyhow::ensure!(
            cfg.scheduler_run_retention_days >= 1,
            "SCHEDULER_RUN_RETENTION_DAYS must be at least 1 (got {})",
            cfg.scheduler_run_retention_days
        );

        let mut registry = JobRegistry::new();
        prune::register(
            &mut registry,
            TimeDelta::days(cfg.scheduler_run_retention_days),
        );
        register(&mut registry);

        Self::from_registry(registry, cfg.scheduler_enabled)
    }

    /// 등록부가 실제로 조립되는지만 확인한다(기동 없이). 등록부의 테스트가 쓴다.
    pub fn validate(register: impl FnOnce(&mut JobRegistry)) -> anyhow::Result<()> {
        let mut registry = JobRegistry::new();
        register(&mut registry);
        Self::from_registry(registry, true).map(|_| ())
    }

    // [아키텍처] 크론식 파싱을 미리 해두는 것이 요점이다. 크론식은 문자열이라 오타가 컴파일
    // 타임에 걸리지 않는데, 이 검증이 루프 안에 있으면 "배포는 성공했는데 그 job만 조용히
    // 안 도는" 상태가 된다 — 그리고 그건 다음 발화 시각이 지나야 드러난다. 기동 시점에
    // 에러로 올려 프로세스가 아예 뜨지 않게 하는 편이 낫다(config.rs와 같은 규칙).
    fn from_registry(registry: JobRegistry, enabled: bool) -> anyhow::Result<Self> {
        let mut names = HashSet::new();
        let mut jobs = Vec::new();

        for job in registry.into_jobs() {
            anyhow::ensure!(
                names.insert(job.name),
                "duplicate scheduled job name: {} (the name is the distributed lock key)",
                job.name
            );

            let cron = Cron::from_str(job.cron)
                .with_context(|| format!("invalid cron for job {}: {:?}", job.name, job.cron))?;

            jobs.push(ScheduledJob {
                name: job.name,
                cron,
                tz: job.tz,
                run: job.run,
            });
        }

        Ok(Self { enabled, jobs })
    }

    /// 종료 신호를 받을 때까지 모든 job을 돌린다.
    // [아키텍처] job마다 태스크를 하나씩 띄운다. 한 루프에서 전부 돌리면 오래 걸리는 job이
    // 그 시간 동안 다른 job의 발화를 통째로 밀어버린다 — 스케줄러에서 가장 흔한 사고다.
    //
    // [아키텍처] 종료에서 진짜 중요한 건 **취소를 어디서 관찰하느냐**다. 아래 루프는 job
    // 실행 도중이 아니라 **다음 발화를 기다리는 sleep**에서 취소를 보므로, 신호가 실행 중에
    // 도착해도 그 실행은 끝까지 마친다. 반대로 job 쪽에 select!를 걸면 지는 Future가 그
    // 자리에서 drop되어 작업이 중간에 잘린다 — DB에 절반만 반영된 채로 끝날 수 있다는 뜻이다.
    // graceful이라는 말이 가리키는 게 정확히 이 차이다. 그 대가로 종료가 한 사이클만큼
    // 늦어질 수 있어서, 호출부(server::run)가 기다리는 시간에 상한을 둔다.
    pub async fn run(self, state: Arc<AppState>, shutdown: CancellationToken) {
        if !self.enabled {
            tracing::info!("scheduler is disabled, no jobs will run");
            return;
        }

        let mut tasks = JoinSet::new();
        for job in self.jobs {
            tracing::info!(job = job.name, cron = %job.cron.pattern.as_str(), tz = %job.tz, "scheduled job registered");
            tasks.spawn(run_job(job, state.clone(), shutdown.clone()));
        }

        // [Rust 특징] JoinSet은 태스크가 끝나는 대로 하나씩 돌려준다. 여기서는 결과가
        // 필요 없고 "전부 끝날 때까지"만 기다리면 되므로 비어 있을 때까지 받아낸다.
        while tasks.join_next().await.is_some() {}

        tracing::info!("scheduler stopped");
    }
}

async fn run_job(job: ScheduledJob, state: Arc<AppState>, shutdown: CancellationToken) {
    loop {
        // [설명] 다음 발화 시각은 매번 **지금**을 기준으로 다시 계산한다. 그래서 프로세스가
        // 꺼져 있던 동안 지나간 발화는 따라잡지 않고 건너뛴다(catch-up 없음). 밀린 배치가
        // 기동 직후에 한꺼번에 쏟아지는 쪽이 대개 더 위험해서 택한 기본값이고, 놓친 발화가
        // 있었는지는 scheduled_job_runs 테이블에 빈 자리로 남으므로 사후에 알 수 있다.
        let now = Utc::now().with_timezone(&job.tz);
        let next = match job.cron.find_next_occurrence(&now, false) {
            Ok(next) => next.with_timezone(&Utc),
            // 파싱은 기동 때 끝났으므로 여기까지 오는 실패는 "이 패턴에 다음 발화가 없다"
            // (예: 존재하지 않는 2월 30일) 같은 경우다. 다시 계산해도 결과가 같으니 루프를
            // 계속 도는 것은 무의미하다.
            Err(err) => {
                tracing::error!(job = job.name, error = %err, "no upcoming occurrence, job disabled");
                return;
            }
        };
        tracing::debug!(job = job.name, next = %next, "next run scheduled");

        if !sleep_until(next, &shutdown).await {
            break;
        }

        fire(&job, &state, next).await;
    }

    tracing::info!(job = job.name, "scheduled job stopped");
}

/// 발화 하나를 선점하고 실행한다.
async fn fire(job: &ScheduledJob, state: &Arc<AppState>, scheduled_for: DateTime<Utc>) {
    // [아키텍처] 인스턴스가 여러 개여도 한 번만 돌게 하는 지점. 발화 시각은 크론식에서
    // 나오므로 모든 인스턴스가 같은 값을 계산하고, 그 값을 먼저 기록한 하나만 실행한다.
    match state.job_run_store.claim_run(job.name, scheduled_for).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::debug!(job = job.name, "run claimed by another instance, skipping");
            return;
        }
        // DB가 잠깐 죽은 경우다. 선점 여부를 모르는 채로 실행하면 중복 실행이 될 수 있으니
        // 이번 발화는 건너뛰고 루프는 유지한다.
        Err(err) => {
            tracing::warn!(job = job.name, error = %err, "could not claim run, skipping");
            return;
        }
    }

    tracing::info!(job = job.name, scheduled_for = %scheduled_for, "job started");

    // [Rust 특징] job 본체를 `tokio::spawn`으로 한 겹 감싼 이유는 **패닉 격리**다. 그냥
    // await하면 job 안의 패닉이 이 루프까지 풀려 올라가 그 job이 영영 멈춘다. 태스크 경계를
    // 하나 두면 패닉이 `JoinError`라는 값으로 잡히므로, 실패로 기록하고 다음 발화를 기다릴 수
    // 있다. (job의 future가 `Send + 'static`이라 이 감싸기가 공짜로 성립한다.)
    let error = match tokio::spawn((job.run)(state.clone())).await {
        Ok(Ok(())) => None,
        Ok(Err(err)) => Some(err.to_string()),
        Err(join_err) => Some(format!("job panicked: {join_err}")),
    };

    match &error {
        None => tracing::info!(job = job.name, "job finished"),
        Some(message) => tracing::error!(job = job.name, error = %message, "job failed"),
    }

    // 마감 기록에 실패해도 루프는 계속 간다 — job은 이미 실행됐고 기록만 못 남긴 상황이라,
    // 다음 발화를 포기할 이유가 없다.
    if let Err(err) = state
        .job_run_store
        .finish_run(job.name, scheduled_for, error.as_deref())
        .await
    {
        tracing::warn!(job = job.name, error = %err, "could not record job outcome");
    }
}

/// 벽시계로 `target`이 될 때까지 잔다. 종료 신호를 받아 중간에 깨면 `false`.
async fn sleep_until(target: DateTime<Utc>, shutdown: &CancellationToken) -> bool {
    loop {
        let remaining = (target - Utc::now()).to_std().unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            return true;
        }

        tokio::select! {
            _ = shutdown.cancelled() => return false,
            _ = tokio::time::sleep(remaining.min(MAX_SLEEP)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::AppError,
        store::{example::MockExampleStore, health::MockHealthStore, job_run::MockJobRunStore},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn state(job_runs: MockJobRunStore) -> Arc<AppState> {
        Arc::new(AppState {
            example_store: Arc::new(MockExampleStore::new()),
            health_store: Arc::new(MockHealthStore::new()),
            job_run_store: Arc::new(job_runs),
        })
    }

    fn assemble(register: impl FnOnce(&mut JobRegistry)) -> anyhow::Result<Scheduler> {
        let mut registry = JobRegistry::new();
        register(&mut registry);
        Scheduler::from_registry(registry, true)
    }

    /// 호출 횟수를 세는 job 하나짜리 스케줄러와, 그 카운터.
    fn counting_job(result: fn() -> Result<(), AppError>) -> (ScheduledJob, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();

        let mut scheduler = assemble(|registry| {
            registry.add("test_job", "0 3 * * *", Tz::UTC, move |_state| {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    result()
                }
            });
        })
        .unwrap();

        (scheduler.jobs.pop().unwrap(), calls)
    }

    // [테스트 시나리오] 크론식 오타는 기동 시점에 드러나야 한다. 이게 깨지면 배포는
    // 성공했는데 그 job만 조용히 안 도는, 발화 시각이 지나야 알 수 있는 상태가 된다.
    #[test]
    fn an_invalid_cron_expression_fails_at_build_time() {
        let err = assemble(|registry| {
            registry.add("bad", "not a cron", Tz::UTC, |_| async { Ok(()) });
        })
        .err()
        .unwrap()
        .to_string();

        assert!(err.contains("bad"), "{err}");
    }

    // [테스트 시나리오] job 이름은 분산 락의 키다. 두 job이 같은 이름을 쓰면 서로의 발화를
    // 선점해 한쪽이 영영 실행되지 않으므로, 조립 단계에서 막는다.
    #[test]
    fn duplicate_job_names_are_rejected() {
        let err = assemble(|registry| {
            registry
                .add("same", "0 3 * * *", Tz::UTC, |_| async { Ok(()) })
                .add("same", "0 4 * * *", Tz::UTC, |_| async { Ok(()) });
        })
        .err()
        .unwrap()
        .to_string();

        assert!(err.contains("duplicate"), "{err}");
    }

    // [테스트 시나리오] 다른 인스턴스가 선점했으면 job 본체가 **아예 실행되지 않아야** 한다.
    // 이 스켈레톤에서 다중 인스턴스 중복 실행을 막는 유일한 장치라 가장 중요한 테스트다.
    #[tokio::test]
    async fn a_run_claimed_by_another_instance_does_not_execute() {
        let mut store = MockJobRunStore::new();
        store.expect_claim_run().return_once(|_, _| Ok(false));
        // 실행하지 않았으므로 마감 기록도 없어야 한다. expectation이 없는 mock 메서드는
        // 호출되는 순간 패닉하므로, 이 줄이 없다는 사실 자체가 검증이 된다.

        let (job, calls) = counting_job(|| Ok(()));
        fire(&job, &state(store), Utc::now()).await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    // [테스트 시나리오] job이 실패해도 그 사실이 실행 기록에 남아야 한다 — 로그만 남기면
    // "어제 그 배치 제대로 돌았나"를 나중에 확인할 방법이 없다.
    #[tokio::test]
    async fn a_failing_job_is_recorded_with_its_error() {
        let mut store = MockJobRunStore::new();
        store.expect_claim_run().return_once(|_, _| Ok(true));
        store
            .expect_finish_run()
            .withf(|_, _, error| error.is_some_and(|e| e.contains("boom")))
            .return_once(|_, _, _| Ok(()));

        let (job, calls) = counting_job(|| Err(AppError::BadRequest("boom".into())));
        fire(&job, &state(store), Utc::now()).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    // [테스트 시나리오] job이 패닉해도 스케줄러가 함께 죽지 않고 실패로 기록해야 한다.
    // 이게 깨지면 job 하나의 버그가 그 job을 프로세스 수명 내내 멈춰버린다.
    #[tokio::test]
    async fn a_panicking_job_is_recorded_as_a_failure() {
        let mut store = MockJobRunStore::new();
        store.expect_claim_run().return_once(|_, _| Ok(true));
        store
            .expect_finish_run()
            .withf(|_, _, error| error.is_some_and(|e| e.contains("panicked")))
            .return_once(|_, _, _| Ok(()));

        let mut scheduler = assemble(|registry| {
            registry.add("panics", "0 3 * * *", Tz::UTC, |_| async {
                panic!("job blew up");
            });
        })
        .unwrap();

        fire(&scheduler.jobs.pop().unwrap(), &state(store), Utc::now()).await;
    }

    // [테스트 시나리오] 종료 신호를 받으면 다음 발화를 기다리다가 즉시 빠져나와야 한다.
    // 대기 상한(MAX_SLEEP)까지 붙잡혀 있으면 graceful shutdown이 그만큼 늦어진다.
    #[tokio::test]
    async fn waiting_for_the_next_run_wakes_up_on_shutdown() {
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        let target = Utc::now() + TimeDelta::hours(1);

        assert!(!sleep_until(target, &shutdown).await);
    }

    // [테스트 시나리오] 이미 지난 시각을 기다리라고 하면 자지 않고 바로 진행해야 한다.
    #[tokio::test]
    async fn a_target_in_the_past_does_not_sleep() {
        let target = Utc::now() - TimeDelta::hours(1);

        assert!(sleep_until(target, &CancellationToken::new()).await);
    }
}
