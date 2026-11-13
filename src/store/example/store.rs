// [예제] store(쓰기 쪽) 포트 + 그 Postgres 구현. 체크리스트:
//   1. trait에 `#[cfg_attr(test, mockall::automock)]` — 소비자가 DB 없이 테스트된다
//   2. `Send + Sync` 상한 — `Box<dyn ...>`/`Arc<dyn ...>`로 스레드를 넘나들려면 필요
//   3. 메서드 단위는 소비자가 한 번의 작업에 필요로 하는 단위 (테이블 단위가 아니다)
//   4. 쿼리는 `&mut *self.conn().await?`로 보낸다 — 트랜잭션 안에서 불리면 그 트랜잭션을 탄다
//
// [설명] SQL은 sqlx의 query! 계열 매크로로 쓴다. 문자열을 그냥 실행하는
// sqlx::query_as::<_, T>()와 달리, 이쪽은 컴파일 타임에 SQL을 검증해 컬럼 이름·타입·
// NULL 허용 여부가 구조체와 맞는지 확인한다 — 오타나 스키마 변경이 배포 후 첫 요청이
// 아니라 빌드 시점에 드러난다.
//
// [주의] 그 대가로 매크로는 **문자열 리터럴만** 받는다. format!으로 조립한 SQL은 물론
// concat!이나 macro_rules!로 만든 문자열도 리터럴로 인정되지 않으므로, 각 쿼리가 컬럼을
// 풀어쓴다. 인자에 따라 SQL 자체가 달라지는 동적 쿼리는 sqlx::QueryBuilder로 조립하고,
// 그 결과 타입에는 #[derive(sqlx::FromRow)]를 단다.
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{error::AppError, store::PgStore};

/// 쓰기 쪽이 다루는 example 한 건. 응답용 모양은 views.rs의 `ExampleView`다.
// [설명] Serialize를 달지 않았다. 이 타입이 응답으로 그대로 나가기 시작하면 테이블 컬럼이
// 곧 API 스펙이 되어, 컬럼 하나를 추가하는 것만으로 응답이 바뀐다.
#[derive(Debug, Clone)]
pub struct ExampleRecord {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait ExampleStore: Send + Sync {
    async fn insert_example(&self, name: &str) -> Result<Uuid, AppError>;
    /// 수정하기 전에 행을 잠그고 읽는다(`SELECT ... FOR UPDATE`). 트랜잭션 안에서만 의미가 있다.
    async fn get_example_for_update(&self, id: Uuid) -> Result<Option<ExampleRecord>, AppError>;
    async fn update_example_name(&self, id: Uuid, name: &str) -> Result<(), AppError>;
}

#[async_trait::async_trait]
impl ExampleStore for PgStore {
    async fn insert_example(&self, name: &str) -> Result<Uuid, AppError> {
        // [설명] unique 제약 위반을 도메인이 이해하는 에러로 번역하는 것까지가 이 계층의
        // 일이다. 위쪽(usecase/http)이 sqlx 에러 코드를 알아야 하는 상황을 만들지 않는다.
        sqlx::query_scalar!("insert into examples (name) values ($1) returning id", name)
            .fetch_one(&mut *self.conn().await?)
            .await
            .map_err(|err| conflict_on_unique(err, name))
    }

    async fn get_example_for_update(&self, id: Uuid) -> Result<Option<ExampleRecord>, AppError> {
        // [설명] FOR UPDATE는 이 행을 트랜잭션이 끝날 때까지 잠근다. 같은 행을 동시에 고치는
        // 두 요청이 있으면 나중 요청은 여기서 기다렸다가 먼저 요청이 커밋한 값을 읽는다 —
        // "읽고 → 판단하고 → 쓰는" 흐름이 다른 인스턴스와 엇갈리지 않게 하는 장치다.
        // 트랜잭션 밖(풀 모드)에서 부르면 잠금이 쿼리 직후 풀려서 의미가 없다.
        Ok(sqlx::query_as!(
            ExampleRecord,
            "select id, name, created_at from examples where id = $1 for update",
            id
        )
        .fetch_optional(&mut *self.conn().await?)
        .await?)
    }

    async fn update_example_name(&self, id: Uuid, name: &str) -> Result<(), AppError> {
        sqlx::query!("update examples set name = $1 where id = $2", name, id)
            .execute(&mut *self.conn().await?)
            .await
            .map_err(|err| conflict_on_unique(err, name))?;
        Ok(())
    }
}

fn conflict_on_unique(err: sqlx::Error, name: &str) -> AppError {
    match &err {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::Conflict(format!("example already exists: {name}"))
        }
        _ => AppError::Database(err),
    }
}
