use crate::error::AppError;

use super::PgStore;

/// readiness 프로브가 쓰는 "저장소에 실제로 닿는가" 확인.
// [설명] 메서드 하나짜리 작은 trait이지만 굳이 분리한다. readiness가 실제 업무 쿼리에
// 의존하면 "readiness를 위해 목록을 읽는" 식의 엉뚱한 결합이 생기기 쉽기 때문이다.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait HealthStore: Send + Sync {
    async fn ping(&self) -> Result<(), AppError>;
}

#[async_trait::async_trait]
impl HealthStore for PgStore {
    // [설명] 이 프로젝트에서 유일하게 매크로(query!)를 쓰지 않는 쿼리다. `select 1`은
    // 스키마의 어떤 객체도 참조하지 않아 컴파일 타임에 검증할 것이 없고, 매크로로 바꾸면
    // 컬럼 이름이 `?column?`이라 Rust 식별자가 못 되어 별칭을 억지로 붙여야 한다.
    // 얻는 것 없이 노이즈만 늘어서 그대로 뒀다.
    async fn ping(&self) -> Result<(), AppError> {
        sqlx::query("select 1").execute(&self.pool).await?;
        Ok(())
    }
}
