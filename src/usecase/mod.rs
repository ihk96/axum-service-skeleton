// [아키텍처] 유스케이스 계층. 요청 하나가 하는 일을 처음부터 끝까지 조립한다 — domain의
// 규칙을 적용하고, store로 쓰고, 필요하면 트랜잭션으로 묶고, views로 응답 모양을 읽는다.
// HTTP도 SQL도 모른다. 호출부가 routes(HTTP)든 bin(CLI)이든 스케줄 job이든 상관없다.
//
// 저장소는 필요한 것만 **좁게** 받는다.
//   - 트랜잭션이 필요 없으면 도메인 store/view를 하나씩: `&dyn ExampleStore`, `&dyn ExampleViews`
//   - 여러 쓰기를 한 단위로 묶어야 하면 진입점을: `&dyn Store` → `begin()` → `commit()`
// `&AppState`를 통째로 받지 않는 게 중요하다. 시그니처가 "이 함수는 이것만 건드린다"는
// 문서가 되고, 테스트도 그 mock만 세우면 끝난다.
//
// 파일은 기능 단위로 둔다(읽기/쓰기로 쪼개지 않는다). 요청 body는 여기의 `XxxInput`을
// http가 그대로 역직렬화해 쓰고, 서버가 채우는 값(사용자 id 등)은 Input이 아니라 인자로 받는다 —
// 클라이언트가 보낸 body로 그 값을 덮어쓸 수 없게 하기 위해서다.
pub mod example;
pub mod example_job;

// 스케줄 job 등록부. 예제가 아니라 이 서비스의 배치 목록이므로 파일 자체는 남기고
// 안의 예제 항목만 지운다.
pub mod jobs;
