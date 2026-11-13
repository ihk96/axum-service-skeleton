// [아키텍처] HTTP 서버의 진입점. src/bin/ 아래 파일들은 각각 Cargo가 만드는 별개의 실행
// 파일이 되고, "이 프로그램을 어떻게 시작하고 호출하는가"라는 관심사끼리 한자리에 모인다.
// CLI를 추가하려면 이 옆에 파일을 하나 더 두면 된다(그때 Cargo.toml의 default-run 확인).
//
// 이 파일이 아는 것은 딱 세 가지, 전부 "밖에서 앱을 두드리는 방법"이다: .env를 환경변수로
// 올리고, 로거를 세우고, 설정을 읽는다. 그 이후는 전부 라이브러리 쪽(server::run)이 한다.
use axum_service_skeleton::{config::AppConfig, server};

// [Rust 특징] `#[tokio::main]`은 이 함수를 감싸 `fn main() { 런타임 생성 후 block_on(...) }`
// 형태로 바꿔치기한다. Rust는 async fn만으로는 실행되지 않고 반드시 런타임이 필요하다.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // [아키텍처] 설정의 창구는 작업 디렉토리의 .env다. 로컬에서는 프로젝트 루트의 .env,
    // 컨테이너에서는 compose가 /app/.env로 마운트한 파일을 entrypoint가 appuser 전용으로
    // 복사해 둔 사본이다(docker-entrypoint.sh가 작업 디렉토리도 그쪽으로 옮긴다).
    // 앱이 직접 읽으므로 compose의 environment에 변수를 하나하나 옮겨 적을 필요가 없고,
    // .env.example에 적힌 값은 전부 그대로 적용된다.
    //
    // [설명] 파일이 **없는** 것은 괜찮다(값을 셸이나 오케스트레이터가 직접 주입하는 환경).
    // 하지만 **있는데 못 읽는** 것은 기동 실패다 — 권한 문제나 문법 오류를 조용히 넘기면
    // "DATABASE_URL must be set"처럼 엉뚱한 에러로만 드러나서 원인을 찾기 어렵다.
    //
    // [주의] dotenvy는 이미 존재하는 환경변수를 덮어쓰지 않는다. 그래서 이미지에 ENV로 박힌
    // 값은 .env로 바꿀 수 없다 — 지금은 entrypoint가 먼저 써야 하는 DATA_DIR 하나뿐이다.
    //
    // [주의] `dotenvy::dotenv()`가 아니라 경로를 못 박은 `from_path`를 쓴다. 전자는 `.env`라는
    // **파일**을 찾아 부모 디렉토리까지 거슬러 올라가서, (1) 엉뚱한 상위 디렉토리의 .env를 읽을
    // 수 있고 (2) 호스트에 .env가 없어 도커가 그 자리에 빈 디렉토리를 만들어 붙인 경우를
    // "파일 없음"으로 넘겨 버린다. 경로를 고정하면 디렉토리는 읽기 실패로 드러난다.
    match dotenvy::from_path(".env") {
        Ok(()) => {}
        Err(err) if err.not_found() => {}
        Err(err) => {
            return Err(anyhow::anyhow!(
                "failed to load .env (is it a readable file? see .env.example): {err}"
            ));
        }
    }

    // [설명] RUST_LOG가 비어 있으면 info로 시작한다. 기본값을 이미지 ENV가 아니라 여기 두는
    // 이유는 위 [주의] 때문이다 — 이미지에 박으면 .env의 RUST_LOG가 무시된다.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // [설명] `?` — from_env()가 Err면 그 자리에서 main도 Err를 반환하며 종료한다. anyhow의
    // Debug 구현이 원인 체인(`Caused by:`)까지 출력해준다.
    server::run(AppConfig::from_env()?).await
}
