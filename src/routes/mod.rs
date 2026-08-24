// [아키텍처] HTTP 레이어의 조립 지점. 각 하위 모듈이 자기 라우트 그룹을
// `Router<Arc<AppState>>`로 만들어 반환하면, 여기서 하나로 merge하고 공통 State/미들웨어를
// 딱 한 번 붙인다 — 라우트가 늘어나도 이 파일에 추가되는 건 `.merge()` 한 줄뿐이다.
//
// 새 라우트 그룹을 추가할 때 중앙 파일을 한 줄 건드려야 하는 건 의도된 트레이드오프다.
// Rust에는 런타임 리플렉션이 없어 애노테이션 스캐닝이 불가능하고, 링크 타임 등록
// (linkme/inventory)은 등록 순서 제어가 안 되고 컴파일러 검증도 못 받는다. 대신 이 파일
// 하나를 읽으면 서비스가 노출하는 라우트 전부를 알 수 있다.
mod example;
mod health;

use std::sync::Arc;

use axum::Router;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::state::AppState;

pub fn build(state: Arc<AppState>) -> Router {
    Router::new()
        .merge(health::routes())
        .merge(example::routes())
        .with_state(state)
        .layer(
            // TraceLayer의 기본 레벨은 DEBUG인데, 그러면 `RUST_LOG=info`로는 안 보이고
            // tower_http 전체에 debug를 켜야 해서 관련 없는 내부 로그까지 딸려온다 —
            // 그래서 여기서 명시적으로 INFO로 낮춘다.
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
}
