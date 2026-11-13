// [아키텍처] 이 서비스만의 규칙이 모여 있는 곳. 저장소도 HTTP도 모르는 순수한 코드만 둔다 —
// 값 검증·정규화, 상태 전이 규칙, 계산 같은 것들. 입력을 받아 판단하고 결과를 돌려줄 뿐이라
// 테스트에 mock조차 필요 없다.
//
// 저장소를 불러야 하는 순간 그 코드는 usecase/의 몫이다. 이 경계를 지키면 규칙이 어느
// 입구(HTTP/CLI/스케줄 job)에서든 같은 모양으로 재사용되고, 규칙만 따로 빠르게 테스트된다.
//
// 스켈레톤을 복사한 뒤 지울 것은 아래 example과 그에 딸린 usecase/example*.rs,
// store/example/, http/routes/example.rs, migrations/0001_init.sql이다.
pub mod example;
