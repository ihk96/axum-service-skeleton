// [아키텍처] HTTP 서버의 진입점. src/bin/ 아래 파일들은 각각 Cargo가 만드는 별개의 실행
// 파일이 되고, "이 프로그램을 어떻게 시작하고 호출하는가"라는 관심사끼리 한자리에 모인다.
// CLI를 추가하려면 이 옆에 파일을 하나 더 두면 된다(그때 Cargo.toml의 default-run 확인).
//
// 이 파일이 아는 것은 딱 세 가지, 전부 "밖에서 앱을 두드리는 방법"이다: .env를 환경변수로
// 올리고, 로거를 세우고, 설정을 읽는다. 그 이후는 전부 llm 라이브러리 쪽(server::run)이 한다.
use axum_service_skeleton::{config::AppConfig, server};

// [Rust 특징] `#[tokio::main]`은 이 함수를 감싸 `fn main() { 런타임 생성 후 block_on(...) }`
// 형태로 바꿔치기한다. Rust는 async fn만으로는 실행되지 않고 반드시 런타임이 필요하다.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // .env가 있으면 환경변수로 로드(없어도 에러 아님). 로컬 개발 편의용이고, 배포 환경에서는
    // 보통 오케스트레이터가 주입한다. dotenvy는 **이미 존재하는 환경변수를 덮어쓰지 않으므로**,
    // 컨테이너에 주입된 값이 마운트된 .env보다 항상 우선한다.
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // [설명] `?` — from_env()가 Err면 그 자리에서 main도 Err를 반환하며 종료한다. anyhow의
    // Debug 구현이 원인 체인(`Caused by:`)까지 출력해준다.
    server::run(AppConfig::from_env()?).await
}
