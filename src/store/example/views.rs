// [예제] views(조회 전용) 포트 + 그 Postgres 구현.
//
// [설명] store와의 차이는 두 가지다.
//   - 응답 모양의 타입(`XxxView`)을 돌려준다. Serialize를 달아 http가 그대로 내보낼 수 있다.
//     조인·집계처럼 화면이 원하는 모양은 여기서 SQL로 만든다.
//   - 트랜잭션에 참여하지 않는다. 그래서 `self.conn()`이 아니라 `&self.pool`을 쓴다.
//     트랜잭션 안에서 쓴 데이터는 커밋 전에는 여기서 보이지 않으므로, usecase는
//     **커밋한 뒤에** view를 조회한다.
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{error::AppError, store::PgStore};

#[derive(Debug, Clone, Serialize)]
pub struct ExampleView {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait ExampleViews: Send + Sync {
    async fn get_example(&self, id: Uuid) -> Result<Option<ExampleView>, AppError>;
    async fn list_examples(&self) -> Result<Vec<ExampleView>, AppError>;
}

#[async_trait::async_trait]
impl ExampleViews for PgStore {
    async fn get_example(&self, id: Uuid) -> Result<Option<ExampleView>, AppError> {
        Ok(sqlx::query_as!(
            ExampleView,
            "select id, name, created_at from examples where id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn list_examples(&self) -> Result<Vec<ExampleView>, AppError> {
        Ok(sqlx::query_as!(
            ExampleView,
            "select id, name, created_at from examples order by created_at desc"
        )
        .fetch_all(&self.pool)
        .await?)
    }
}
