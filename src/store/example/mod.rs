// [예제] 도메인 영역 하나의 저장소. 이 폴더는 지울 대상이지만, 새 영역을 추가할 때의 모양을
// 그대로 따라 쓸 수 있게 최소 형태로 남겨뒀다.
//   store.rs  제어(쓰기)와 검증 조회 — `ExampleStore`
//   views.rs  응답용 조회 전용 — `ExampleViews`
// 만든 뒤에는 store/entry.rs의 `entries!` 목록에 한 줄씩 등록한다.
mod store;
mod views;

pub use store::*;
pub use views::*;
