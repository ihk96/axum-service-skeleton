// [아키텍처] HTTP 서버를 구성하는 것들만 모아둔 모듈 — 부팅 절차(run)와 종료 신호
// 처리(shutdown). 둘 다 "이 프로세스가 HTTP 서버라서" 필요한 것들이고, config/error/state처럼
// 어느 진입점이든 쓰는 것들과 구분해 여기 둔다. kafka 컨슈머 같은 다른 진입점이 생기면
// 그쪽도 같은 층위에 나란히 놓을 자리가 된다(consumer/).
//
// 라우터는 여기 없다. `/health`, `/ready`도 결국 HTTP 라우트이므로 다른 라우트와 함께
// http/routes/ 아래에 있다 — "라우터 조립은 한 곳"이라는 원칙에 라우트의 성격에 따라 예외를
// 두면서까지 깨뜨릴 이유가 없다.
//
// 이 코드가 bin이 아니라 lib에 있는 이유가 이 구조의 핵심이다. bin 크레이트에 있는 코드는
// (1) 같은 패키지의 다른 bin이 재사용할 수 없고 (2) tests/ 아래 통합 테스트가 import할 수
// 없다. 그래서 src/bin/*.rs는 "이 프로그램을 밖에서 어떻게 호출하는가"만 알고, 실제 하는
// 일은 전부 이쪽으로 넘긴다 — 헥사고날 용어로 bin은 driving adapter의 얇은 껍데기일 뿐이다.
mod shutdown;

use std::{sync::Arc, time::Duration};

use crate::{
    config::AppConfig,
    http,
    scheduler::Scheduler,
    state::AppState,
    store::{self, PgStore},
    usecase::jobs,
};

/// 서버를 기동하고, 종료 신호를 받아 정상 종료할 때까지 돌린다.
pub async fn run(cfg: AppConfig) -> anyhow::Result<()> {
    let db = store::pg_pool(&cfg).await?;

    // [설명] `sqlx::migrate!()`가 컴파일 타임에 ./migrations의 SQL을 바이너리에 내장시키고,
    // 여기서 순서대로 실행해 스키마를 맞춘다 — 별도 마이그레이션 도구 없이 기동 자체가
    // 스키마를 동기화한다. 스키마를 바꾸는 권한은 이 부팅 경로에만 두므로, 풀을 공유하는
    // CLI는 이 단계를 거치지 않는다.
    //
    // [핵심] `set_ignore_missing(true)` — "DB에는 적용돼 있는데 이 바이너리는 모르는" 마이그레이션이
    // 있어도 기동한다. blue/green에서 신버전이 0003을 적용한 뒤 **구버전으로 롤백**하는 경우가
    // 정확히 그 상황인데, 기본값(false)이면 구버전이 그 자리에서 기동을 거부해 롤백 자체가 막힌다.
    //
    // [주의] 이건 "기동은 시켜준다"까지다. 구버전 코드가 새 스키마에서 실제로 **동작**하는지는
    // 마이그레이션을 쓰는 규칙이 보장해야 한다(docs/ARCHITECTURE.md의 "마이그레이션 규칙").
    // 이미 적용된 마이그레이션 파일의 내용을 고친 경우(체크섬 불일치)는 여전히 기동 실패다.
    let mut migrator = sqlx::migrate!();
    migrator.set_ignore_missing(true);
    migrator.run(&db).await?;

    // [아키텍처] 여기가 포트에 실제 구현(어댑터)을 꽂는 유일한 지점이다. "저장소가
    // Postgres다"를 아는 곳은 이 함수뿐이고, 그 아래 모든 코드는 trait만 본다.
    let store = Arc::new(PgStore::new(db));
    let state = AppState::new(store);

    // [아키텍처] 종료 신호를 하나의 토큰으로 만들어 서버와 백그라운드 태스크가 함께 본다.
    // 이 토큰이 이 함수에서 가장 중요한 값이다 — 종료가 "프로세스가 죽는 것"이 아니라
    // "모두가 하던 일을 마치고 각자 멈추는 것"이 되게 하는 유일한 연결고리다.
    let shutdown = shutdown::listen();

    // [아키텍처] 스케줄러는 여기서 **조립만** 한다. 넘기는 것은 job 목록이 아니라
    // usecase/jobs.rs의 등록 함수이고, job을 추가해도 이 줄은 그대로다.
    //
    // 조립을 spawn보다 먼저 하는 이유는 크론식이 문자열이기 때문이다. 오타는 컴파일 타임에
    // 걸리지 않으므로 여기서 `?`로 기동을 실패시킨다 — 안 그러면 "배포는 됐는데 그 배치만
    // 조용히 안 도는" 상태가 되고, 그건 다음 발화 시각이 지나야 드러난다.
    let scheduler = Scheduler::build(&cfg, jobs::register)?;

    // [설명] 레이트 리밋 설정은 라우터와 정리 태스크가 **같은 것**을 봐야 하므로 여기서
    // 한 번 만들어 둘에 나눠준다. 각자 만들면 버킷이 따로 놀아서, 정리 태스크가 비우는
    // 맵과 실제로 판정에 쓰이는 맵이 달라진다.
    let rate_limit = http::middleware::rate_limit::config(&cfg)?;

    // [설명] 백그라운드 태스크는 tokio::spawn으로 던지되 JoinHandle을 들고 있는다.
    // 상시 루프 태스크(kafka 컨슈머 등)를 붙일 자리도 여기다 — 그때 추가되는 건
    // state.clone()과 토큰 clone을 넘기는 한 줄뿐이다. 태스크마다 필요한 포트를 골라
    // 각각 clone할 일이 없다(그게 state를 통째로 넘기는 이유).
    let tasks = vec![
        tokio::spawn(scheduler.run(state.clone(), shutdown.clone())),
        tokio::spawn(http::middleware::rate_limit::prune(
            rate_limit.clone(),
            shutdown.clone(),
        )),
    ];

    let app = http::build(state, &cfg, rate_limit)?;

    let listener = tokio::net::TcpListener::bind(&cfg.server_addr).await?;
    tracing::info!(addr = %cfg.server_addr, "listening");
    // [아키텍처] graceful shutdown — 토큰이 취소되면 새 연결은 받지 않되, 이미 진행 중인
    // 요청은 끝까지 처리한 뒤 이 await가 반환된다.
    // [핵심] `into_make_service_with_connect_info`가 요청마다 커넥션의 peer 주소를
    // extension으로 실어준다. 레이트 리밋의 IP 추출기가 헤더(x-forwarded-for)를 먼저
    // 보고 없으면 이 값으로 떨어지는데, 이게 없으면 **폴백이 아예 없어서** 헤더가 붙지
    // 않은 요청이 전부 500이 된다(프록시를 거치지 않는 로컬 요청이 그렇다).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown({
        let shutdown = shutdown.clone();
        async move { shutdown.cancelled().await }
    })
    .await?;

    // [설명] 여기 도달했다는 건 서버가 멈췄다는 뜻이지만, 그 이유가 항상 종료 신호는 아니다
    // (bind가 끊기는 등의 사정도 있다). 그러니 한 번 더 취소해 둔다 — 이미 취소된 토큰을
    // 다시 취소하는 것은 무해하고, 이렇게 해야 어떤 경로로 내려왔든 태스크가 반드시 깨어난다.
    shutdown.cancel();

    // [아키텍처] 그리고 태스크가 실제로 끝날 때까지 기다린다. 이 await가 없으면 run이
    // 반환되고 프로세스가 종료되면서 태스크는 하던 일 중간에 증발한다 — 토큰만 넘기고
    // 기다리지 않으면 절반만 구현한 셈이다.
    //
    // 무한정 기다리지는 않는다. 태스크가 응답하지 않을 때 프로세스가 영영 안 죽으면
    // 오케스트레이터가 결국 SIGKILL로 때리는데, 그러면 graceful하게 만든 의미가 없다.
    // 상한을 두고 지나면 포기하고 내려간다.
    //
    // [설명] 상한은 태스크 하나가 아니라 **전체**에 건다. 태스크마다 따로 걸면 느린 태스크가
    // 여럿일 때 대기 시간이 합산되어, 설정한 값보다 훨씬 오래 매달려 있게 된다.
    let grace = Duration::from_secs(cfg.shutdown_timeout_secs);
    let joined = tokio::time::timeout(grace, async {
        for task in tasks {
            if let Err(err) = task.await {
                tracing::error!(error = %err, "background task panicked");
            }
        }
    })
    .await;

    match joined {
        Ok(()) => tracing::info!("background tasks stopped cleanly"),
        Err(_) => tracing::warn!(
            timeout_secs = cfg.shutdown_timeout_secs,
            "background tasks did not stop in time, exiting anyway"
        ),
    }

    Ok(())
}
