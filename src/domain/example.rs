// [예제] 도메인 함수의 모양. 특징은 두 가지다:
//   - 저장소를 `&dyn ExampleStore`로 **좁게** 받는다. `&AppState`를 받지 않는 게 중요한데,
//     그래야 시그니처가 "이 함수는 이것만 건드린다"는 문서가 되고 테스트도 mock 하나로 끝난다.
//   - HTTP를 모른다. 실패는 `AppError`로 표현하고, 그게 몇 번 상태 코드가 되는지는 error.rs가 안다.
use crate::{
    error::AppError,
    store::{ExampleRecord, ExampleStore},
};

/// 이름을 검증하고 새 example을 만든다.
pub async fn create_example(store: &dyn ExampleStore, name: &str) -> Result<ExampleRecord, AppError> {
    let name = name.trim();

    // [설명] 이런 검증이 도메인 계층에 있는 이유 — HTTP 핸들러에 두면 같은 규칙이 CLI나
    // 컨슈머 경로에서 빠진다. 입구가 여러 개인 서비스에서 규칙은 항상 안쪽에 둔다.
    if name.is_empty() {
        return Err(AppError::BadRequest("name must not be empty".into()));
    }
    if name.chars().count() > 64 {
        return Err(AppError::BadRequest("name must be 64 characters or fewer".into()));
    }

    store.insert_example(name).await
}

pub async fn list_examples(store: &dyn ExampleStore) -> Result<Vec<ExampleRecord>, AppError> {
    store.list_examples().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::example::MockExampleStore;
    use uuid::Uuid;

    // [테스트 시나리오] 검증이 저장소에 닿기 전에 걸러내는지 확인한다. mock에 아무
    // expectation도 걸지 않았다는 것 자체가 "저장소를 건드리지 않았다"는 검증이다 —
    // 빈 mock은 호출되는 순간 패닉하기 때문.
    #[tokio::test]
    async fn a_blank_name_is_rejected_without_touching_the_store() {
        let store = MockExampleStore::new();

        for name in ["", "   "] {
            let err = create_example(&store, name).await.unwrap_err();
            assert!(matches!(err, AppError::BadRequest(_)), "got {err:?}");
        }
    }

    // [테스트 시나리오] 이름 양끝 공백이 저장 전에 정리되는지 — 저장소에 실제로 전달된
    // 값을 `withf`로 들여다본다.
    #[tokio::test]
    async fn the_name_is_trimmed_before_it_reaches_the_store() {
        let mut store = MockExampleStore::new();
        store
            .expect_insert_example()
            .withf(|name| name == "hello")
            .times(1)
            .returning(|name| {
                Ok(ExampleRecord {
                    id: Uuid::new_v4(),
                    name: name.to_string(),
                    created_at: chrono::Utc::now(),
                })
            });

        let created = create_example(&store, "  hello  ").await.unwrap();

        assert_eq!(created.name, "hello");
    }
}
