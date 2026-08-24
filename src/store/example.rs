// [예제] 포트 하나 + 그 Postgres 구현. 이 파일은 지울 대상이지만, 새 저장소를 추가할 때의
// 모양을 그대로 따라 쓸 수 있게 최소 형태로 남겨뒀다. 체크리스트는 셋뿐이다:
//   1. trait에 `#[cfg_attr(test, mockall::automock)]` — 소비자가 DB 없이 테스트된다
//   2. `Send + Sync` 상한 — `Arc<dyn ...>`으로 스레드를 넘나들려면 필요
//   3. 메서드 단위는 소비자가 한 번의 작업에 필요로 하는 단위 (테이블 단위가 아니다)
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::error::AppError;

use super::PgStore;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ExampleRecord {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait ExampleStore: Send + Sync {
    async fn insert_example(&self, name: &str) -> Result<ExampleRecord, AppError>;
    async fn list_examples(&self) -> Result<Vec<ExampleRecord>, AppError>;
}

#[async_trait::async_trait]
impl ExampleStore for PgStore {
    async fn insert_example(&self, name: &str) -> Result<ExampleRecord, AppError> {
        // [설명] unique 제약 위반을 도메인이 이해하는 에러로 번역하는 것까지가 이 계층의
        // 일이다. 위쪽(domain/routes)이 sqlx 에러 코드를 알아야 하는 상황을 만들지 않는다.
        sqlx::query_as::<_, ExampleRecord>(
            "insert into examples (name) values ($1) returning id, name, created_at",
        )
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| match &err {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                AppError::Conflict(format!("example already exists: {name}"))
            }
            _ => AppError::Database(err),
        })
    }

    async fn list_examples(&self) -> Result<Vec<ExampleRecord>, AppError> {
        Ok(sqlx::query_as::<_, ExampleRecord>(
            "select id, name, created_at from examples order by created_at desc",
        )
        .fetch_all(&self.pool)
        .await?)
    }
}
