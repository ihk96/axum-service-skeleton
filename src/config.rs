use std::env;

use anyhow::Context;

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
    /// 이 프로세스가 스케줄 job(domain/jobs.rs)을 돌릴지 여부. 기본은 켜짐이다.
    ///
    /// 중복 실행은 DB 락이 막으므로 여러 인스턴스가 켜 두어도 안전하다. 이 스위치는
    /// "배치는 전용 인스턴스에서만 돌린다"처럼 배치를 **의도적으로 분리 배치**할 때,
    /// 또는 로컬에서 잠깐 꺼둘 때 쓴다.
    pub scheduler_enabled: bool,
    /// 스케줄러의 실행 기록(scheduled_job_runs)을 며칠 보관할지. 스케줄러가 내부적으로
    /// 등록하는 정리 job이 이 값을 쓴다.
    ///
    /// [주의] 그 기록은 이력이자 **락**이다. 발화 주기 근처로 짧게 잡으면 방금 끝난 발화의
    /// 행이 지워지면서 같은 발화를 다른 인스턴스가 다시 선점할 수 있다. 일 단위로 넉넉히.
    pub scheduler_run_retention_days: i64,
}

impl AppConfig {
    // [설명] DATABASE_URL만 필수이고 나머지는 기본값을 갖는다 — 로컬에서 환경변수 하나만
    // 설정해도 바로 뜨게 하려는 의도.
    //
    // [아키텍처] 설정 오류는 **기동 시점에 전부 드러난다**. 값이 없으면 기본값을 쓰지만,
    // 값이 있는데 해석할 수 없으면 그 자리에서 에러다. 잘못된 설정으로 뜬 프로세스는
    // 겉보기에 정상이라 한참 뒤에 엉뚱한 증상으로만 발견되므로, 아예 뜨지 않는 편이 낫다.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            server_addr: env::var("SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            // [설명] `?`만 쓰면 "environment variable not found"라고만 나와서 어느 변수인지
            // 알 수 없다. 설정 실패 메시지는 운영자가 읽고 바로 고칠 수 있어야 한다.
            database_url: env::var("DATABASE_URL")
                .context("DATABASE_URL must be set (see .env.example)")?,
            request_timeout_secs: env_or("REQUEST_TIMEOUT_SECS", 30)?,
            db_max_connections: env_or("DB_MAX_CONNECTIONS", 10)?,
            db_acquire_timeout_secs: env_or("DB_ACQUIRE_TIMEOUT_SECS", 10)?,
            db_statement_timeout_secs: env_or("DB_STATEMENT_TIMEOUT_SECS", 30)?,
            shutdown_timeout_secs: env_or("SHUTDOWN_TIMEOUT_SECS", 10)?,
            // [설명] bool도 같은 헬퍼를 그대로 쓴다 — `FromStr`이 있으므로. 대신 받는
            // 값은 "true"/"false"뿐이다. "1"이나 "yes"를 넣으면 기본값으로 조용히
            // 폴백하지 않고 기동 에러가 난다(그게 이 헬퍼의 규칙이다).
            scheduler_enabled: env_or("SCHEDULER_ENABLED", true)?,
            scheduler_run_retention_days: env_or("SCHEDULER_RUN_RETENTION_DAYS", 30)?,
        })
    }
}

// [Rust 특징] 제네릭 + 트레잇 바운드(`T: FromStr`)로 "문자열에서 파싱 가능한 아무 타입"을
// 한 헬퍼로 처리한다.
//
// [설명] 환경변수 부재와 파싱 실패를 다르게 다루는 것이 요점이다. 없으면 기본값이지만,
// 있는데 숫자가 아니면 에러다 — `DB_MAX_CONNECTIONS=1O`(0 대신 알파벳 O) 같은 오타가
// 조용히 기본값으로 폴백하면, 설정을 바꿨다고 믿는 운영자와 실제로 도는 값이 어긋난 채
// 아무도 모르는 상태가 된다. 그 어긋남은 부하가 오르고 나서야 증상으로 드러난다.
fn env_or<T>(key: &str, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match env::var(key) {
        Ok(raw) => raw
            .parse()
            .map_err(|err| anyhow::anyhow!("{key}: {err} (got {raw:?})")),
        Err(env::VarError::NotPresent) => Ok(default),
        // [설명] 값이 있긴 한데 유효한 유니코드가 아닌 경우. 드물지만 조용히 넘기면
        // "설정했는데 반영이 안 된다"는 가장 찾기 어려운 종류의 문제가 된다.
        Err(env::VarError::NotUnicode(_)) => {
            Err(anyhow::anyhow!("{key}: value is not valid unicode"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [설명] 환경변수는 프로세스 전역이라 테스트가 병렬로 돌면 서로 간섭한다. 그래서
    // 이 서비스가 실제로 읽는 키(DB_MAX_CONNECTIONS 등)는 건드리지 않고, 테스트 전용
    // 이름을 따로 만들어 쓴다 — 다른 테스트가 같은 키를 보지 않으므로 병렬로 돌아도 안전하다.
    #[test]
    fn a_missing_variable_falls_back_to_the_default() {
        let value = env_or::<u64>("SKELETON_TEST_DEFINITELY_UNSET", 7).unwrap();

        assert_eq!(value, 7);
    }

    // [테스트 시나리오] 값이 있는데 해석되지 않으면 기본값으로 넘어가지 않고 에러여야 한다.
    // 이 성질이 깨지면 오타 난 설정이 조용히 무시되므로, 실패 메시지에 키 이름과 실제
    // 입력이 함께 담기는 것까지 확인한다.
    #[test]
    fn an_unparsable_value_is_an_error_not_a_silent_default() {
        // SAFETY: 이 키는 이 테스트에서만 쓰며, 아래에서 곧바로 제거한다.
        unsafe { env::set_var("SKELETON_TEST_BAD_NUMBER", "1O") };

        let result = env_or::<u64>("SKELETON_TEST_BAD_NUMBER", 10);

        unsafe { env::remove_var("SKELETON_TEST_BAD_NUMBER") };

        let message = result.unwrap_err().to_string();
        assert!(message.contains("SKELETON_TEST_BAD_NUMBER"), "{message}");
        assert!(message.contains("1O"), "{message}");
    }
}
