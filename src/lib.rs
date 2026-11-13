// [아키텍처] "lib + thin bin" 구조. 실제 로직은 전부 이 라이브러리 크레이트에
// 있고, src/bin/ 아래 실행 파일들은 그것을 가져다 쓰는 얇은 껍데기다. 서버의
// 부팅 절차조차 bin이 아니라 server/에 있다 — bin 안의 코드는 (1) 같은 패키지의
// 다른 bin이 재사용할 수 없고 (2) tests/의 통합 테스트가 import할 수 없기 때문.
//
// 모듈을 가른 기준은 **"다른 프로젝트로 옮겼을 때 내용이 그대로인가"**다. 그대로인
// 골격은 루트/server/store·http의 조립 지점에 두고, 이 서비스에서만 의미가 있는 것은
// domain/과 usecase/로(그리고 그에 딸린 store/http의 영역별 파일로) 내린다. 이 스켈레톤에서
// 지울 대상도 곧 그 아래의 example 파일들이다.

// 어느 진입점(서버/CLI/컨슈머)이든 쓰는 크레이트 전역 기반.
pub mod config;
pub mod error;
pub mod state;
pub mod sync_util;

// "이 프로세스가 HTTP 서버라서" 필요한 것들. 다른 진입점이 생기면 같은 층위에
// 나란히 놓는다(예: consumer/).
pub mod server;

// 크론식 배치의 실행기. server/와 나란한 층위다 — "이 프로세스가 배치를 돌려서" 필요한
// 것들이고, 무엇을 언제 돌릴지(usecase/jobs.rs)는 알지 않는다.
pub mod scheduler;

// 저장소. 진입점은 둘이다 — 제어(쓰기)의 `Store`와 조회 전용의 `Views`(store/entry.rs).
// 밖으로 나가는 통합이 늘어나면 store/의 형제로 추가한다 — 이때 폴더 이름은 기술이
// 아니라 포트 이름으로 짓는다(`redis/`가 아니라 `cache/`, `kafka/`가 아니라 `events/`).
// 포트의 존재 의의가 "기술은 바뀔 수 있다"는 것이기 때문.
pub mod store;

// HTTP 어댑터. 라우트 그룹(routes/)과, 라우트가 아닌 부속인 미들웨어 설정(middleware/)이
// 이 아래 모여 있다. 핸들러가 하는 일은 요청 파싱 → usecase 호출 → 응답 직렬화까지다.
pub mod http;

// 이 서비스만의 코드는 두 층이다. 기능이 늘어날 때 커지는 폴더는 이 둘이어야 한다.
//   domain/   순수 규칙. 저장소도 HTTP도 모른다
//   usecase/  요청 하나를 조립한다. domain 규칙 + store/views + 트랜잭션
// 두 층에 같은 이름의 모듈(example)이 있으므로 경로에 층 이름을 남긴다
// (`crate::domain::example`, `crate::usecase::example`) — 어느 층을 부르는지가 곧 드러난다.
pub mod domain;
pub mod usecase;
