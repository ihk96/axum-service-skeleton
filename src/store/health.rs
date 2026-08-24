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
    async fn ping(&self) -> Result<(), AppError> {
        sqlx::query("select 1").execute(&self.pool).await?;
        Ok(())
    }
}
