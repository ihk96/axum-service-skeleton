// [아키텍처] 이 서비스만의 로직이 모여 있는 곳. 폴더를 가른 기준은 **다른 프로젝트로
// 옮겼을 때 내용이 그대로인가**다. 그대로인 것(config, error, state, server, store의 골격)은
// 밖에 있고, 이 서비스에서만 의미가 있는 것은 전부 여기 있다. 그래서 기능이 늘어날 때
// 커지는 폴더는 여기 하나여야 한다.
//
// 이 계층은 HTTP도 SQL도 모른다. 각 모듈은 필요한 저장소를 `&dyn XStore` 포트로 받고,
// 그 포트를 누가 구현했는지는 신경 쓰지 않는다 — 덕분에 테스트가 mock만으로 돌고,
// 호출부는 routes/(HTTP)든 bin/(CLI)든 백그라운드 태스크든 상관없다.
//
// 스켈레톤을 복사한 뒤 지울 것은 아래 두 파일과 그에 딸린 store/example.rs,
// routes/example.rs, migrations/0001_init.sql이다.
pub mod example;
pub mod example_task;
