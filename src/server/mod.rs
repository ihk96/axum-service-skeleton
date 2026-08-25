// [아키텍처] HTTP 서버를 구성하는 것들만 모아둔 모듈 — 부팅 절차(run)와 종료 신호
// 처리(shutdown). 둘 다 "이 프로세스가 HTTP 서버라서" 필요한 것들이고, config/error/state처럼
// 어느 진입점이든 쓰는 것들과 구분해 여기 둔다. kafka 컨슈머 같은 다른 진입점이 생기면
// 그쪽도 같은 층위에 나란히 놓을 자리가 된다(consumer/).
//
// 라우터는 여기 없다. `/health`, `/ready`도 결국 HTTP 라우트이므로 다른 라우트와 함께
// routes/ 아래에 있다 — "라우터 조립은 한 곳"이라는 원칙에 라우트의 성격에 따라 예외를
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
    example_task,
    routes,
    state::AppState,
    store::{self, PgStore},
};

/// 서버를 기동하고, 종료 신호를 받아 정상 종료할 때까지 돌린다.
pub async fn run(cfg: AppConfig) -> anyhow::Result<()> {
    let db = store::pg_pool(&cfg).await?;

    // [설명] `sqlx::migrate!()`가 컴파일 타임에 ./migrations의 SQL을 바이너리에 내장시키고,
    // 여기서 순서대로 실행해 스키마를 맞춘다 — 별도 마이그레이션 도구 없이 기동 자체가
    // 스키마를 동기화한다. 스키마를 바꾸는 권한은 이 부팅 경로에만 두므로, 풀을 공유하는
    // CLI는 이 단계를 거치지 않는다.
    sqlx::migrate!().run(&db).await?;

    // [아키텍처] 여기가 포트에 실제 구현(어댑터)을 꽂는 유일한 지점이다. "저장소가
    // Postgres다"를 아는 곳은 이 함수뿐이고, 그 아래 모든 코드는 trait만 본다.
    let store = Arc::new(PgStore::new(db));
    let state = AppState::new(store);

    // [아키텍처] 종료 신호를 하나의 토큰으로 만들어 서버와 백그라운드 태스크가 함께 본다.
    // 이 토큰이 이 함수에서 가장 중요한 값이다 — 종료가 "프로세스가 죽는 것"이 아니라
    // "모두가 하던 일을 마치고 각자 멈추는 것"이 되게 하는 유일한 연결고리다.
    let shutdown = shutdown::listen();

    // [설명] 백그라운드 태스크는 tokio::spawn으로 던지되 JoinHandle을 들고 있는다. 태스크가
    // 여러 개로 늘어도 여기 추가되는 건 state.clone()과 토큰 clone을 넘기는 한 줄뿐이다 —
    // 태스크마다 필요한 포트를 골라 각각 clone할 일이 없다(그게 state를 통째로 넘기는 이유).
    // 핸들이 여러 개가 되면 아래 기다리는 부분을 Vec로 모아 join하면 된다.
    let sweep_task = tokio::spawn(example_task::run_periodic(
        state.clone(),
        Duration::from_secs(cfg.example_sweep_interval_secs),
        shutdown.clone(),
    ));

    let app = routes::build(state, Duration::from_secs(cfg.request_timeout_secs));

    let listener = tokio::net::TcpListener::bind(&cfg.server_addr).await?;
    tracing::info!(addr = %cfg.server_addr, "listening");
    // [아키텍처] graceful shutdown — 토큰이 취소되면 새 연결은 받지 않되, 이미 진행 중인
    // 요청은 끝까지 처리한 뒤 이 await가 반환된다.
    axum::serve(listener, app)
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
    let grace = Duration::from_secs(cfg.shutdown_timeout_secs);
    match tokio::time::timeout(grace, sweep_task).await {
        Ok(Ok(())) => tracing::info!("background tasks stopped cleanly"),
        Ok(Err(err)) => tracing::error!(error = %err, "background task panicked"),
        Err(_) => tracing::warn!(
            timeout_secs = cfg.shutdown_timeout_secs,
            "background task did not stop in time, exiting anyway"
        ),
    }

    Ok(())
}
