// [아키텍처] 저장소의 두 진입점. AppState는 도메인 저장소로 이 둘만 들고 있다.
//
// - `Store`: 제어(쓰기)와 그에 딸린 검증 조회. 도메인별 store를 바로 꺼내 쓰거나, 여러 작업을
//   한 단위로 묶어야 하면 `begin()`으로 트랜잭션(`Tx`)을 연다. `Tx`에서 꺼낸 store들은 모두
//   같은 트랜잭션을 공유하고, `commit()` 없이 drop되면 롤백된다.
// - `Views`: 화면/응답용 조회 전용. 트랜잭션에 참여하지 않고 항상 풀을 쓴다.
//
// [설계] `Store`와 `Tx`를 다른 타입으로 둔 이유: `commit()`은 `Tx`에만, `begin()`은 `Store`에만
// 있어서 "트랜잭션을 열지 않고 커밋"하거나 "트랜잭션 안에서 또 여는" 실수가 컴파일 단계에서
// 막힌다. accessor가 참조가 아니라 `Box`를 돌려주는 건 mockall로 그대로 mock하기 위해서다.
//
// **새 store/view를 추가할 때는 아래 `entries!` 목록에 한 줄을 넣고, 그 trait을 위 `use`에
// 추가하면 된다.** trait 정의와 Postgres 구현은 매크로(entry/macros.rs)가 목록으로부터 만든다.
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::{
    PgStore,
    example::{ExampleStore, ExampleViews},
};
use crate::error::AppError;

// macro_rules는 쓰는 곳보다 먼저 정의돼야 해서 별도 파일로 뺐다.
#[macro_use]
mod macros;

entries! {
    stores {
        examples: ExampleStore,
    }
    views {
        examples: ExampleViews,
    }
}

/// 트랜잭션 모드의 `PgStore`.
struct PgTx(PgStore);
