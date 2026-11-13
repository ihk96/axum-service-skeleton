// [예제] 유스케이스의 두 가지 모양.
//   - create_example: 쓰기 한 번이면 트랜잭션 없이 도메인 store를 바로 받는다.
//   - rename_example: 읽고 → 판단하고 → 쓰는 흐름이라 트랜잭션으로 묶는다(`&dyn Store`).
// 둘 다 쓰기가 끝난 뒤 views로 응답 모양을 읽는다.
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    domain::example::normalize_name,
    error::AppError,
    store::{
        Store,
        example::{ExampleStore, ExampleView, ExampleViews},
    },
};

#[derive(Debug, Clone, Deserialize)]
pub struct CreateExampleInput {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RenameExampleInput {
    pub name: String,
}

pub async fn create_example(
    examples: &dyn ExampleStore,
    views: &dyn ExampleViews,
    input: CreateExampleInput,
) -> Result<ExampleView, AppError> {
    let name = normalize_name(&input.name)?;
    let id = examples.insert_example(&name).await?;

    views.get_example(id).await?.ok_or(AppError::NotFound)
}

/// 이름을 바꾼다. 행을 잠그고 읽은 뒤 바꾸므로, 같은 example을 동시에 고치는 요청이 엇갈리지 않는다.
pub async fn rename_example(
    store: &dyn Store,
    views: &dyn ExampleViews,
    id: Uuid,
    input: RenameExampleInput,
) -> Result<ExampleView, AppError> {
    // 검증처럼 DB가 필요 없는 일은 트랜잭션을 열기 전에 끝낸다 — 트랜잭션은 커넥션 하나를
    // 붙잡고 있으므로, 그 안에서 하는 일이 길수록 풀이 빨리 마른다.
    let name = normalize_name(&input.name)?;

    let tx = store.begin().await?;
    let examples = tx.examples();

    examples
        .get_example_for_update(id)
        .await?
        .ok_or(AppError::NotFound)?;
    examples.update_example_name(id, &name).await?;

    tx.commit().await?;

    // view는 트랜잭션 밖(풀)에서 읽으므로 커밋한 뒤에 조회한다.
    views.get_example(id).await?.ok_or(AppError::NotFound)
}

pub async fn list_examples(views: &dyn ExampleViews) -> Result<Vec<ExampleView>, AppError> {
    views.list_examples().await
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::store::{
        MockStore, MockTx,
        example::{ExampleRecord, MockExampleStore, MockExampleViews},
    };

    fn view(id: Uuid, name: &str) -> ExampleView {
        ExampleView {
            id,
            name: name.to_string(),
            created_at: Utc::now(),
        }
    }

    fn record(id: Uuid, name: &str) -> ExampleRecord {
        ExampleRecord {
            id,
            name: name.to_string(),
            created_at: Utc::now(),
        }
    }

    // ---- create_example ----

    // [테스트 시나리오] 검증이 저장소에 닿기 전에 걸러내는지. mock에 아무 expectation도 걸지
    // 않았다는 것 자체가 "저장소를 건드리지 않았다"는 검증이다 — 빈 mock은 호출되는 순간 패닉한다.
    #[tokio::test]
    async fn create_rejects_a_blank_name_without_touching_the_store() {
        let result = create_example(
            &MockExampleStore::new(),
            &MockExampleViews::new(),
            CreateExampleInput { name: "  ".into() },
        )
        .await;

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    // [테스트 시나리오] 정리된 이름이 저장소에 전달되고, 응답은 views에서 읽은 모양이다.
    #[tokio::test]
    async fn create_stores_the_trimmed_name_and_returns_the_view() {
        let id = Uuid::new_v4();

        let mut examples = MockExampleStore::new();
        examples
            .expect_insert_example()
            .withf(|name| name == "hello")
            .times(1)
            .returning(move |_| Ok(id));

        let mut views = MockExampleViews::new();
        views
            .expect_get_example()
            .withf(move |got| *got == id)
            .times(1)
            .returning(|id| Ok(Some(view(id, "hello"))));

        let created = create_example(
            &examples,
            &views,
            CreateExampleInput {
                name: "  hello  ".into(),
            },
        )
        .await
        .unwrap();

        assert_eq!(created.name, "hello");
    }

    // ---- rename_example ----
    //
    // 트랜잭션 테스트의 모양: 도메인 store mock(examples) → 그걸 내주는 MockTx(tx) →
    // begin()하면 tx를 내주는 MockStore(store). commit 여부는 tx의 expectation으로 검증한다.

    // [테스트 시나리오] 성공 — 잠그고 읽은 뒤 바꾸고, 커밋한 다음 view를 읽는다.
    #[tokio::test]
    async fn rename_updates_and_commits() {
        let id = Uuid::new_v4();

        let mut examples = MockExampleStore::new();
        examples
            .expect_get_example_for_update()
            .times(1)
            .returning(|id| Ok(Some(record(id, "old"))));
        examples
            .expect_update_example_name()
            .withf(move |got, name| *got == id && name == "new")
            .times(1)
            .returning(|_, _| Ok(()));

        let mut tx = MockTx::new();
        tx.expect_examples().return_once(move || Box::new(examples));
        tx.expect_commit().times(1).returning(|| Ok(()));
        let mut store = MockStore::new();
        store.expect_begin().return_once(move || Ok(Box::new(tx)));

        let mut views = MockExampleViews::new();
        views
            .expect_get_example()
            .returning(|id| Ok(Some(view(id, "new"))));

        let renamed = rename_example(
            &store,
            &views,
            id,
            RenameExampleInput { name: "new".into() },
        )
        .await
        .unwrap();

        assert_eq!(renamed.name, "new");
    }

    // [테스트 시나리오] 없는 example — NotFound, 커밋하지 않는다(drop되며 롤백).
    #[tokio::test]
    async fn rename_of_a_missing_example_is_not_found_and_not_committed() {
        let mut examples = MockExampleStore::new();
        examples
            .expect_get_example_for_update()
            .returning(|_| Ok(None));

        let mut tx = MockTx::new();
        tx.expect_examples().return_once(move || Box::new(examples));
        tx.expect_commit().never();
        let mut store = MockStore::new();
        store.expect_begin().return_once(move || Ok(Box::new(tx)));

        let result = rename_example(
            &store,
            &MockExampleViews::new(),
            Uuid::new_v4(),
            RenameExampleInput { name: "new".into() },
        )
        .await;

        assert!(matches!(result, Err(AppError::NotFound)));
    }

    // [테스트 시나리오] 검증 실패 — 트랜잭션을 아예 열지 않는다.
    #[tokio::test]
    async fn rename_with_a_blank_name_does_not_begin_a_transaction() {
        let mut store = MockStore::new();
        store.expect_begin().never();

        let result = rename_example(
            &store,
            &MockExampleViews::new(),
            Uuid::new_v4(),
            RenameExampleInput { name: " ".into() },
        )
        .await;

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}
