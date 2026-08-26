// [아키텍처] **등록 창구**. 스케줄러에서 바깥(domain/jobs.rs)이 보는 유일한 면이다.
//
// 실행 기반(runner.rs)과 이 파일을 나눈 이유가 요점이다. 등록부는 "무엇을 언제 돌리는가"만
// 적는 자리여야 하는데, 창구가 없으면 그 파일이 `Job` 구조체를 직접 만들고 `Vec`를 조립하고
// 결국 스케줄러 내부 타입을 하나씩 알게 된다. 창구를 하나 두면 등록부가 아는 이름은
// `JobRegistry`와 `Tz` 둘뿐이고, 그 아래가 어떻게 도는지는 전부 감춰진다.
use std::{future::Future, pin::Pin, sync::Arc};

use chrono_tz::Tz;

use crate::{error::AppError, state::AppState};

/// job 하나가 반환하는 future. 인자를 받는 job을 팩토리로 만들 때만 직접 쓸 일이 있다
/// (예: scheduler/prune.rs). 보통은 이름을 볼 일이 없다.
pub type JobFuture = Pin<Box<dyn Future<Output = Result<(), AppError>> + Send>>;

// [Rust 특징] async fn마다 컴파일러가 서로 다른 익명 타입을 만들기 때문에, 여러 job을 한
// `Vec`에 담으려면 반환 future를 `Pin<Box<dyn Future>>`로 통일해야 한다. 그 박싱을 아래
// `add`가 대신 해주므로, 등록부는 async fn 이름만 그대로 넘기면 된다.
type BoxedJobFn = Box<dyn Fn(Arc<AppState>) -> JobFuture + Send + Sync>;

/// 등록된 job 하나. 창구를 거치지 않고는 만들 수 없도록 생성 경로를 `add`로 한정했다.
pub(super) struct Job {
    pub(super) name: &'static str,
    pub(super) cron: &'static str,
    pub(super) tz: Tz,
    pub(super) run: BoxedJobFn,
}

/// job을 등록받는 창구. `Scheduler::build`가 하나 만들어 등록부에 건네준다.
#[derive(Default)]
pub struct JobRegistry {
    jobs: Vec<Job>,
}

impl JobRegistry {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn into_jobs(self) -> Vec<Job> {
        self.jobs
    }

    /// job 하나를 등록한다.
    ///
    /// - `name`: 로그와 실행 기록(scheduled_job_runs)에 남는 식별자. **분산 락의 키이므로
    ///   job끼리 겹치면 안 되고**(중복은 기동 시점에 걸러진다), 한 번 정한 뒤 바꾸면 그
    ///   시점까지의 실행 기록과 이어지지 않는다.
    /// - `cron`: 5필드(분 시 일 월 요일) 또는 6필드(초를 앞에 붙임). `@daily` 같은 별칭도 된다.
    /// - `tz`: 위 크론식을 해석할 시간대. 기본값을 두지 않은 것은 의도다 — "새벽 3시"가
    ///   KST인지 UTC인지는 등록 줄만 보고 알 수 있어야 한다.
    /// - `run`: `async fn(Arc<AppState>) -> Result<(), AppError>`.
    ///
    // [아키텍처] job 함수는 `Arc<AppState>`를 통째로 받고, 실제 로직은 그 안에서 필요한
    // 포트만 `&dyn`으로 받는 함수에 넘긴다(domain/example_job.rs 참고). 진입점이 state를
    // 받으므로 job이 나중에 포트를 더 써도 등록 줄은 그대로고, 로직 쪽은 좁은 시그니처라
    // mock 하나로 테스트가 끝난다.
    pub fn add<F, Fut>(
        &mut self,
        name: &'static str,
        cron: &'static str,
        tz: Tz,
        run: F,
    ) -> &mut Self
    where
        F: Fn(Arc<AppState>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), AppError>> + Send + 'static,
    {
        self.jobs.push(Job {
            name,
            cron,
            tz,
            run: Box::new(move |state| Box::pin(run(state))),
        });
        self
    }
}
