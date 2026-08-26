// [아키텍처] "lib + thin bin" 구조. 실제 로직은 전부 이 라이브러리 크레이트에
// 있고, src/bin/ 아래 실행 파일들은 그것을 가져다 쓰는 얇은 껍데기다. 서버의
// 부팅 절차조차 bin이 아니라 server/에 있다 — bin 안의 코드는 (1) 같은 패키지의
// 다른 bin이 재사용할 수 없고 (2) tests/의 통합 테스트가 import할 수 없기 때문.
//
// 모듈을 가른 기준은 **"다른 프로젝트로 옮겼을 때 내용이 그대로인가"**다. 그대로인
// 골격은 루트/server/store의 조립 지점에 두고, 이 서비스에서만 의미가 있는 것은
// 전부 domain/ 아래로 내린다. 기능이 늘어날 때 커지는 폴더가 하나로 모이게 하려는
// 의도이고, 이 스켈레톤에서 지울 대상도 곧 domain/과 그에 딸린 example 파일들이다.

// 어느 진입점(서버/CLI/컨슈머)이든 쓰는 크레이트 전역 기반.
pub mod config;
pub mod error;
pub mod state;
pub mod sync_util;

// "이 프로세스가 HTTP 서버라서" 필요한 것들. 다른 진입점이 생기면 같은 층위에
// 나란히 놓는다(예: consumer/).
pub mod server;

// 크론식 배치의 실행기. server/와 나란한 층위다 — "이 프로세스가 배치를 돌려서" 필요한
// 것들이고, 무엇을 언제 돌릴지(domain/jobs.rs)는 알지 않는다.
pub mod scheduler;

// 포트(trait)와 그 어댑터. 도메인은 이 trait들만 보고 구현체는 모른다.
// 밖으로 나가는 통합이 늘어나면 여기 형제로 추가한다 — 이때 폴더 이름은 기술이
// 아니라 포트 이름으로 짓는다(`redis/`가 아니라 `cache/`, `kafka/`가 아니라
// `events/`). 포트의 존재 의의가 "기술은 바뀔 수 있다"는 것이기 때문.
pub mod store;

// HTTP 핸들러(어댑터). 요청 파싱 → domain 함수 호출 → 응답 직렬화만 한다.
pub mod routes;

// [Rust 특징] `mod domain`(비공개) + `pub use domain::*` — 파일은 폴더로 정리하면서
// 소비자 경로는 `crate::example`처럼 짧게 유지한다. `pub mod domain`으로 하면
// `crate::domain::example`과 `crate::example` 두 경로가 동시에 공개돼 사용법이 갈린다.
mod domain;
pub use domain::*;
