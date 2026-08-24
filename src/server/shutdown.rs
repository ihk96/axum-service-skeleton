/// SIGINT(Ctrl+C) 또는 SIGTERM을 받는 즉시 완료되는 Future. `axum::serve`가 이걸 기다림으로써
/// 새 연결은 그만 받되 진행 중인 요청은 끝까지 처리하고 종료할 수 있다.
// [설명] 프로덕션에서 한 걸음 더 나가려면: SIGTERM을 받은 즉시 종료를 시작하는 대신 먼저
// readiness를 false로 바꾸고 몇 초 기다린 뒤 종료하는 게 좋다. 그러지 않으면 로드밸런서가
// 아직 이 인스턴스를 정상으로 알고 새 요청을 몇 초간 더 보내고, 그 요청들이 연결 거부를
// 맞는다. 그렇게 하려면 AppState에 `AtomicBool` 하나를 두고 readiness 핸들러와 이 함수가
// 공유하면 된다.
pub(super) async fn signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    // [Rust 특징] `#[cfg(unix)]` / `#[cfg(not(unix))]` — 컴파일 타임 조건부 컴파일이라
    // 선택되지 않은 쪽은 아예 컴파일되지 않는다. SIGTERM은 유닉스 계열 개념이므로 Windows
    // 등에서는 절대 완료되지 않는 `pending()`으로 대체해 select!의 타입을 양쪽 다 맞춘다.
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    // [Rust 특징] tokio::select! — 여러 Future 중 가장 먼저 완료되는 것 하나를 기다리고
    // 나머지는 취소한다.
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received, draining in-flight requests");
}
