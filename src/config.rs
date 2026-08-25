use std::env;

// [아키텍처] 설정을 구조체 하나로 모아 기동 시점에 한 번만 환경변수를 읽고, 이후로는
// 이 값을 전달해서 쓴다(12-factor 스타일) — 코드 곳곳에서 `env::var(...)`를 산발적으로
// 부르지 않는다. bin이 이걸 읽어 server::run 등에 넘기므로, 환경변수를 아는 곳은
// 이 파일과 각 bin의 main뿐이다.
//
// 값의 출처(.env / 셸 export / 컨테이너 주입)는 여기서 알지 않는다. bin에서 부르는
// `dotenvy::dotenv()`가 .env를 프로세스 환경변수로 올려주고, dotenvy는 이미 존재하는
// 환경변수를 덮어쓰지 않으므로 우선순위는 항상 "주입된 환경변수 > .env > 아래 기본값"이다.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub server_addr: String,
    pub database_url: String,
    pub db_max_connections: u32,
    /// 풀에서 커넥션을 **기다리는** 최대 시간. DB가 죽거나 풀이 고갈된 상태에서
    /// 요청이 무한정 멈춰 있지 않게 한다.
    pub db_acquire_timeout_secs: u64,
    /// 요청 하나가 핸들러에서 끝나기까지 허용하는 시간. 초과하면 408로 끊는다.
    /// 아래 DB 타임아웃들이 막지 못하는 구간(외부 HTTP 호출 등)을 덮는 마지막 그물이라,
    /// 대개 그 값들보다 넉넉해야 한다 — 더 짧으면 DB 타임아웃이 발동할 기회가 없다.
    pub request_timeout_secs: u64,
    /// 커넥션마다 Postgres `statement_timeout`으로 적용된다. 커넥션은 이미 잡았는데
    /// 개별 쿼리가 안 끝나는 경우를 막는다 — acquire_timeout이 커버하지 못하는 구간.
    pub db_statement_timeout_secs: u64,
    /// 종료 신호를 받은 뒤 백그라운드 태스크가 끝나기를 기다리는 상한(초). 이 시간이
    /// 지나면 기다리기를 포기하고 프로세스를 내린다 — 응답하지 않는 태스크 때문에 영영
    /// 종료되지 않으면 오케스트레이터가 결국 SIGKILL로 때리므로, graceful하게 만든 의미가
    /// 사라지기 때문이다. 쿠버네티스라면 terminationGracePeriodSeconds보다 짧게 잡을 것.
    pub shutdown_timeout_secs: u64,
    /// 예제 백그라운드 태스크의 주기. domain/example_task.rs와 함께 지울 대상.
    pub example_sweep_interval_secs: u64,
}

impl AppConfig {
    // [설명] DATABASE_URL만 필수(없으면 `?`로 즉시 에러)이고 나머지는 기본값을 갖는다 —
    // 로컬에서 환경변수 하나만 설정해도 바로 뜨게 하려는 의도.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            server_addr: env::var("SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            database_url: env::var("DATABASE_URL")?,
            request_timeout_secs: env_or("REQUEST_TIMEOUT_SECS", 30),
            db_max_connections: env_or("DB_MAX_CONNECTIONS", 10),
            db_acquire_timeout_secs: env_or("DB_ACQUIRE_TIMEOUT_SECS", 10),
            db_statement_timeout_secs: env_or("DB_STATEMENT_TIMEOUT_SECS", 30),
            shutdown_timeout_secs: env_or("SHUTDOWN_TIMEOUT_SECS", 10),
            example_sweep_interval_secs: env_or("EXAMPLE_SWEEP_INTERVAL_SECS", 30),
        })
    }
}

// [Rust 특징] 제네릭 + 트레잇 바운드(`T: FromStr`)로 "문자열에서 파싱 가능한 아무 타입"을
// 한 헬퍼로 처리한다. 파싱 실패와 부재를 모두 `.ok()`로 뭉개고 기본값으로 폴백하는데,
// 이는 "설정이 잘못돼도 조용히 기본값으로 뜬다"는 정책이다 — 오타를 즉시 드러내고 싶다면
// 여기서 에러를 표면화하도록 바꿀 것.
fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
