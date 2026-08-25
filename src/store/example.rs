// [예제] 포트 하나 + 그 Postgres 구현. 이 파일은 지울 대상이지만, 새 저장소를 추가할 때의
// 모양을 그대로 따라 쓸 수 있게 최소 형태로 남겨뒀다. 체크리스트는 셋뿐이다:
//   1. trait에 `#[cfg_attr(test, mockall::automock)]` — 소비자가 DB 없이 테스트된다
//   2. `Send + Sync` 상한 — `Arc<dyn ...>`으로 스레드를 넘나들려면 필요
//   3. 메서드 단위는 소비자가 한 번의 작업에 필요로 하는 단위 (테이블 단위가 아니다)
//
// [설명] SQL은 sqlx의 query_as! 매크로로 쓴다. 문자열을 그냥 실행하는
// sqlx::query_as::<_, T>()와 달리, 이쪽은 컴파일 타임에 DB로 SQL을 보내 컬럼 이름·타입·
// NULL 허용 여부가 구조체와 맞는지 검증한다 — 오타나 스키마 변경이 배포 후 첫 요청이
// 아니라 빌드 시점에 드러난다.
//
// [주의] 그 대가로 매크로는 **문자열 리터럴만** 받는다. format!으로 조립한 SQL은 물론
// concat!이나 macro_rules!로 만든 문자열도 리터럴로 인정되지 않으므로, 컬럼 목록을 상수로
// 빼서 여러 쿼리가 공유하게 하는 식의 정리가 불가능하다. 각 쿼리가 컬럼을 풀어쓴다.
//
// [주의] 인자에 따라 SQL 자체가 달라지는 동적 쿼리(조건부 WHERE, 가변 SET 절)는 매크로로
// 쓸 수 없다 — 컴파일 시점에 검증할 문자열이 아직 존재하지 않기 때문이다. 그런 쿼리는
// sqlx::QueryBuilder로 조립하는데, 그때는 결과 매핑을 위해 그 타입에 #[derive(sqlx::FromRow)]
// 가 필요하다. 반대로 매크로만 쓰는 아래 ExampleRecord에는 그 derive가 없다 — query_as!는
// FromRow를 거치지 않고 검증된 컬럼 목록으로 구조체를 직접 조립하기 때문이다.
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::error::AppError;

use super::PgStore;

#[derive(Debug, Clone, Serialize)]
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
        //
        // [설명] 매크로로 써도 에러 처리 흐름은 그대로다 — query_as!가 돌려주는 것도 결국
        // 같은 sqlx::Error다. 컴파일 타임 검증이 보장하는 건 "SQL이 스키마와 맞는가"까지이고,
        // unique 위반 같은 제약의 런타임 동작은 여전히 여기서 받아 번역해야 한다.
        sqlx::query_as!(
            ExampleRecord,
            "insert into examples (name) values ($1) returning id, name, created_at",
            name
        )
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
        Ok(sqlx::query_as!(
            ExampleRecord,
            "select id, name, created_at from examples order by created_at desc"
        )
        .fetch_all(&self.pool)
        .await?)
    }
}
