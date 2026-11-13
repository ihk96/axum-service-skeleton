// [아키텍처] 라우트 그룹만 모아둔다. 각 모듈은 자기 그룹을 `Router<Arc<AppState>>`로
// 만들어 반환하고, 그것들을 하나로 합쳐 공통 State/미들웨어를 붙이는 조립은 http/mod.rs가
// 한다 — 이 폴더를 열면 서비스가 노출하는 라우트 그룹 목록이 그대로 보인다.
pub mod example;
pub mod health;
