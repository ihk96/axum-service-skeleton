// [아키텍처] 저장소 계층. 도메인 영역마다 하위 모듈을 두고, 그 안을 두 축으로 나눈다.
//   - store(`XxxStore`): 제어(쓰기)와 그에 딸린 검증 조회. 트랜잭션에 참여할 수 있다.
//   - views(`XxxViews`): 화면/응답용 조회 전용. 트랜잭션에 참여하지 않고 항상 풀을 쓴다.
// 바깥(usecase/http)은 이 둘을 entry.rs의 진입점(`Store`, `Views`)을 통해 꺼내 쓴다.
//
// trait의 모양을 정하는 기준은 **테이블이 아니라 쓰임새**다. 소비자가 한 번의 작업에
// 필요로 하는 단위로 메서드를 만들 것. 쿼리 단위로 잘게 쪼개면 재사용은 좋아 보이지만,
// 소비자 쪽 테스트가 매번 여러 스텁을 세워야 하고 조합 로직이 소비자에게 흩어진다.
// 포트와 그 구현을 한 파일에 두는 것도 같은 이유다 — 모양을 정할 때 둘을 같이 봐야 한다.
//
// 이 계층의 impl 블록에는 SQL을 쓰는 것 외에 판단하는 로직이 없으므로 의도적으로 단위
// 테스트 대상에서 제외한다 — SQL이 맞는지는 실제 DB에 붙여봐야만 알 수 있는 종류의 문제다.
pub mod example;
pub mod health;
pub mod job_run;

mod entry;
pub use entry::*;

use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    time::Duration,
};

use sqlx::{
    Executor, PgConnection, PgPool, Postgres, Transaction, pool::PoolConnection,
    postgres::PgPoolOptions,
};
use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard};

use crate::{config::AppConfig, error::AppError};

pub use health::HealthStore;
pub use job_run::JobRunStore;

/// 설정에 따라 Postgres 커넥션 풀을 만든다.
// [아키텍처] 이 함수가 bin이 아니라 여기에 있는 이유는, 서버와 CLI 등 모든 진입점이 같은
// 풀 설정을 쓰게 만들기 위해서다. 이 코드가 bin 안에 있으면 다른 bin이 재사용할 수 없어서
// 결국 복사되고, 복사본은 시간이 지나면 원본과 갈라진다(타임아웃 하나가 한쪽에만 빠지는 식).
//
// [설명] 두 타임아웃이 막는 상황이 서로 다르다. acquire_timeout은 "풀에서 커넥션을 얻기까지"
// 기다리는 시간을, after_connect로 심는 Postgres statement_timeout은 "이미 커넥션을 잡은 뒤
// 개별 쿼리가 끝나기까지"의 시간을 제한한다 — 전자만 두면 커넥션은 받았는데 쿼리가 영원히
// 안 끝나는 경우를 놓친다.
//
// 마이그레이션은 의도적으로 여기서 실행하지 않는다. 스키마를 바꾸는 건 서버 부팅 경로
// (server::run)의 책임이고, 운영자가 돌린 CLI가 조용히 스키마를 변경하면 놀라운 동작이 된다.
pub async fn pg_pool(cfg: &AppConfig) -> Result<PgPool, sqlx::Error> {
    let statement_timeout = format!(
        "SET statement_timeout = '{}s'",
        cfg.db_statement_timeout_secs
    );

    PgPoolOptions::new()
        .max_connections(cfg.db_max_connections)
        .acquire_timeout(Duration::from_secs(cfg.db_acquire_timeout_secs))
        // [Rust 특징] Box::pin(async move { ... }) — 클로저가 반환하는 async 블록의 타입은
        // 컴파일러가 만드는 익명 타입이라 크기를 알 수 없다. 힙에 올려 크기가 고정된
        // 트레잇 객체(Pin<Box<dyn Future>>)로 만들어야 시그니처가 성립한다.
        .after_connect(move |conn, _meta| {
            let statement_timeout = statement_timeout.clone();
            Box::pin(async move {
                conn.execute(statement_timeout.as_str()).await?;
                Ok(())
            })
        })
        .connect(&cfg.database_url)
        .await
}

/// 이 서비스의 모든 저장소 trait을 한 몸에 구현하는 Postgres 어댑터.
// [설명] trait은 영역별로 나눠져 있지만 구현체는 하나다 — 커넥션 풀이 하나뿐이고, 영역이
// 달라도 같은 트랜잭션 경계/설정을 공유하기 때문. 소비자 입장에서는 여전히 자기 영역의
// trait 하나만 보이므로, 구현이 뭉쳐 있다는 사실이 위로 새어나가지 않는다.
//
// [아키텍처] 같은 구조체가 `tx`가 None이면 풀로, Some이면 그 트랜잭션으로 쿼리를 보낸다
// (트랜잭션 모드는 entry.rs의 `Store::begin`이 만든다). 그래서 store 구현은 `self.conn()`만
// 쓰면 되고 트랜잭션용 SQL을 따로 둘 필요가 없다. views 구현은 `&self.pool`을 직접 쓴다.
//
// store 구현을 쓸 때 지킬 규칙:
//   1. 쿼리는 `&mut *self.conn().await?`로 보낸다. `&self.pool`로 보내면 트랜잭션 안에서
//      불려도 트랜잭션 밖으로 나가 버린다.
//   2. 메서드 안에서 여러 쿼리를 원자적으로 묶어야 하면 `self.pool.begin()`이 아니라
//      `sqlx::Connection::begin(&mut *conn)`을 쓴다. 트랜잭션 모드에서는 SAVEPOINT가 된다.
//   3. `conn`을 쥔 채로 `self`의 다른 메서드를 부르지 않는다. 트랜잭션 모드에서는 Mutex라
//      재진입하면 교착된다 — 먼저 drop할 것.
//
// [주의] Postgres는 트랜잭션 안에서 쿼리 하나가 실패하면 그 트랜잭션의 이후 쿼리를 **전부**
// 거부한다(`current transaction is aborted`). unique 위반을 잡아 재시도하는 식의 코드를
// 트랜잭션 안에서 돌리려면, 실패할 수 있는 쿼리를 규칙 2의 SAVEPOINT로 감싸거나
// `ON CONFLICT`로 에러 자체가 나지 않게 쓴다.
#[derive(Clone)]
pub struct PgStore {
    pub(crate) pool: PgPool,
    // 한 트랜잭션에서 꺼낸 여러 store가 같은 트랜잭션을 공유하도록 Arc로 들고, store trait이
    // `&self`라서 Mutex로 감쌌다. commit하면 Option이 비워져 이후 쿼리는 에러가 된다.
    pub(crate) tx: Option<Arc<Mutex<Option<Transaction<'static, Postgres>>>>>,
}

impl PgStore {
    // PgPool 자체가 내부적으로 Arc라 clone이 저렴하므로, 소유권을 받아 들고 있어도 부담이 없다.
    pub fn new(pool: PgPool) -> Self {
        Self { pool, tx: None }
    }

    /// 쿼리를 보낼 커넥션. 트랜잭션 모드면 그 트랜잭션을, 아니면 풀 커넥션을 준다.
    pub(crate) async fn conn(&self) -> Result<Conn<'_>, AppError> {
        Ok(match &self.tx {
            Some(tx) => Conn::Tx(
                MutexGuard::try_map(tx.lock().await, |t| t.as_mut()).map_err(|_| {
                    AppError::Internal(anyhow::anyhow!("transaction already finished"))
                })?,
            ),
            None => Conn::Pool(self.pool.acquire().await?),
        })
    }
}

pub(crate) enum Conn<'a> {
    Pool(PoolConnection<Postgres>),
    Tx(MappedMutexGuard<'a, Transaction<'static, Postgres>>),
}

impl Deref for Conn<'_> {
    type Target = PgConnection;
    fn deref(&self) -> &PgConnection {
        match self {
            Conn::Pool(c) => c,
            Conn::Tx(t) => t,
        }
    }
}

impl DerefMut for Conn<'_> {
    fn deref_mut(&mut self) -> &mut PgConnection {
        match self {
            Conn::Pool(c) => c,
            Conn::Tx(t) => t,
        }
    }
}
