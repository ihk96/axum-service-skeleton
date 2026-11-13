// `entries!` 매크로 정의. 목록은 entry.rs에 있다.
//
// 목록 하나로부터 `Store`/`Tx`/`Views` trait과 Postgres 구현을 만든다. 펼쳐진 코드의 이름
// (PgStore, AppError, Arc 등)은 호출하는 쪽(entry.rs)의 use를 따른다.
macro_rules! entries {
    (
        stores { $($s:ident: $st:ident),* $(,)? }
        views { $($v:ident: $vt:ident),* $(,)? }
    ) => {
        #[cfg_attr(test, mockall::automock)]
        #[async_trait]
        pub trait Store: Send + Sync {
            $(fn $s(&self) -> Box<dyn $st>;)*
            /// 트랜잭션을 연다. commit 없이 drop되면 롤백된다.
            async fn begin(&self) -> Result<Box<dyn Tx>, AppError>;
        }

        /// 하나의 트랜잭션. 여기서 꺼낸 store들은 모두 같은 트랜잭션을 공유한다.
        #[cfg_attr(test, mockall::automock)]
        #[async_trait]
        pub trait Tx: Send + Sync {
            $(fn $s(&self) -> Box<dyn $st>;)*
            async fn commit(self: Box<Self>) -> Result<(), AppError>;
        }

        #[cfg_attr(test, mockall::automock)]
        pub trait Views: Send + Sync {
            $(fn $v(&self) -> Box<dyn $vt>;)*
        }

        // Postgres 구현: 모든 accessor는 같은 PgStore의 clone(풀/트랜잭션 공유)을 내준다.
        #[async_trait]
        impl Store for PgStore {
            $(fn $s(&self) -> Box<dyn $st> { Box::new(self.clone()) })*
            async fn begin(&self) -> Result<Box<dyn Tx>, AppError> {
                let tx = self.pool.begin().await?;
                Ok(Box::new(PgTx(PgStore {
                    pool: self.pool.clone(),
                    tx: Some(Arc::new(Mutex::new(Some(tx)))),
                })))
            }
        }

        #[async_trait]
        impl Tx for PgTx {
            $(fn $s(&self) -> Box<dyn $st> { Box::new(self.0.clone()) })*
            async fn commit(self: Box<Self>) -> Result<(), AppError> {
                let Some(tx) = &self.0.tx else { return Ok(()) };
                // 꺼내서 비워 두면, 아직 살아 있는 store clone은 이후 쿼리에서 에러가 난다.
                if let Some(tx) = tx.lock().await.take() {
                    tx.commit().await?;
                }
                Ok(())
            }
        }

        impl Views for PgStore {
            $(fn $v(&self) -> Box<dyn $vt> { Box::new(self.clone()) })*
        }
    };
}
