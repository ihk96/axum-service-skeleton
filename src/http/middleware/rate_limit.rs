// [아키텍처] 레이트 리밋 설정과 그 뒷정리. HTTP 레이어의 관심사이므로 http/ 아래 둔다.
//
// [설계] 앱에서 거는 이유. 앞단 프록시(nginx, API 게이트웨이)에서도 걸 수 있지만, 그쪽은
// 대개 서비스 전체에 한 가지 규칙만 걸린다. 앱에 두면 엔드포인트별로 다르게 걸 수 있다
// (지금도 /health, /ready는 제외한다 — http/mod.rs 참고). 앞단에 이미 충분한 리밋이 있다면
// http/mod.rs에서 레이어를 떼어내도 된다.
//
// [주의] 버킷은 **인스턴스 메모리**에 있다. 인스턴스가 N개면(blue/green 전환 중 포함) 실제
// 허용량도 최대 N배가 된다. 정확한 전역 한도가 필요하면 Redis 같은 공유 저장소 기반으로
// 바꾸거나 앞단에서 건다 — 여기서 막으려는 건 "한 클라이언트의 폭주" 정도다.
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Request},
    response::{IntoResponse, Response},
};
use governor::middleware::NoOpMiddleware;
use tokio_util::sync::CancellationToken;
use tower_governor::{
    GovernorError, GovernorLayer, governor::GovernorConfigBuilder, key_extractor::KeyExtractor,
};

use crate::{
    config::{AppConfig, ClientIpHeader},
    error::AppError,
};

/// 버킷을 비우는 주기. 짧게 잡을 이유가 없다 — 메모리 회수가 목적이지 정확성에 영향을
/// 주지 않는다(오래된 버킷은 어차피 가득 찬 상태라, 지워지든 남아 있든 판정이 같다).
const PRUNE_INTERVAL: Duration = Duration::from_secs(300);

/// 이 서비스가 쓰는 레이트 리밋 설정의 구체 타입.
// [Rust 특징] GovernorConfig는 키 추출 방식과 미들웨어를 타입 파라미터로 받는다. 그대로
// 쓰면 시그니처마다 이 긴 타입이 반복되므로 별칭을 둔다.
pub type Config = tower_governor::governor::GovernorConfig<ClientIpKeyExtractor, NoOpMiddleware>;

/// 설정에서 레이트 리밋 구성을 만든다. 값이 유효하지 않으면 기동을 실패시킨다.
pub fn config(cfg: &AppConfig) -> anyhow::Result<Arc<Config>> {
    anyhow::ensure!(
        cfg.rate_limit_burst > 0,
        "RATE_LIMIT_BURST must be at least 1 (got {})",
        cfg.rate_limit_burst
    );
    anyhow::ensure!(
        cfg.rate_limit_per_second > 0,
        "RATE_LIMIT_PER_SECOND must be at least 1 (got {})",
        cfg.rate_limit_per_second
    );

    // [주의] tower_governor의 `per_second(n)`은 "초당 n개"가 **아니라** "n초마다 1개를
    // 보충한다"는 간격이다. 이름만 보고 쓰면 의도와 정확히 반대로 설정된다(초당 5개를
    // 원하면서 per_second(5)를 넣으면 5초당 1개가 된다).
    //
    // 설정값은 사람이 읽는 대로 "초당 몇 개"로 두고, 여기서 간격으로 뒤집는다.
    let period = Duration::from_secs(1) / cfg.rate_limit_per_second;

    let config = GovernorConfigBuilder::default()
        .period(period)
        .burst_size(cfg.rate_limit_burst)
        // [핵심] 프록시 뒤에서 peer IP만 보면 모든 요청이 프록시 IP 하나로 묶여서, 한 사람이
        // 전체를 막아버릴 수 있다. 그래서 프록시가 실어 준 헤더에서 클라이언트 IP를 읽는다 —
        // 어느 헤더를 믿을지는 배포마다 다르므로 설정(CLIENT_IP_HEADER)으로 받는다.
        //
        // [주의] 크레이트의 SmartIpKeyExtractor를 쓰지 않았다. 그쪽은 x-forwarded-for의 **맨
        // 왼쪽** 값을 먼저 보고 없으면 x-real-ip를 보는데, 둘 다 위조 경로가 된다 — nginx의
        // 흔한 설정(덧붙이기)에서 맨 왼쪽은 클라이언트가 보낸 값이고, 프록시가 세팅하지 않는
        // 헤더는 클라이언트가 보낸 그대로 통과하기 때문이다.
        .key_extractor(ClientIpKeyExtractor(cfg.client_ip_header))
        .finish()
        .context("failed to build the rate limiter configuration")?;

    Ok(Arc::new(config))
}

/// 업무 라우트에 붙일 레이트 리밋 레이어. 한도를 넘으면 다른 에러와 같은 JSON으로 429를 낸다.
pub fn layer(config: Arc<Config>) -> GovernorLayer<ClientIpKeyExtractor, NoOpMiddleware, Body> {
    // [설명] 크레이트 기본 응답은 평문("Too Many Requests! Wait for 3s")이라, 클라이언트가 이
    // 경로만 다른 형식으로 처리해야 한다. AppError로 통일하고, 크레이트가 실어 준 헤더
    // (retry-after, x-ratelimit-*)는 그대로 옮겨 붙인다.
    GovernorLayer::new(config).error_handler(|err| match err {
        GovernorError::TooManyRequests { headers, .. } => {
            with_headers(AppError::TooManyRequests.into_response(), headers)
        }
        GovernorError::UnableToExtractKey => AppError::Internal(anyhow::anyhow!(
            "rate limiter could not determine the client IP"
        ))
        .into_response(),
        GovernorError::Other { msg, headers, .. } => with_headers(
            AppError::Internal(anyhow::anyhow!(
                "rate limiter error: {}",
                msg.unwrap_or_default()
            ))
            .into_response(),
            headers,
        ),
    })
}

fn with_headers(mut response: Response, headers: Option<HeaderMap>) -> Response {
    if let Some(headers) = headers {
        response.headers_mut().extend(headers);
    }
    response
}

/// 설정한 헤더에서 클라이언트 IP를 읽고, 헤더가 없으면 커넥션의 peer IP로 떨어지는 키 추출기.
// [설명] peer IP 폴백은 프록시를 거치지 않은 요청(로컬에서 직접 부르는 경우)을 위한 것이다.
// 헤더를 빼고 보내도 키가 프록시 IP로 묶일 뿐이라 남의 버킷을 쓸 수는 없다. peer IP는
// server::run의 `into_make_service_with_connect_info`가 실어 준다.
#[derive(Debug, Clone, Copy)]
pub struct ClientIpKeyExtractor(ClientIpHeader);

impl KeyExtractor for ClientIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let headers = req.headers();
        let from_header = match self.0 {
            ClientIpHeader::ForwardedFor => header_str(headers, "x-forwarded-for")
                .and_then(|value| value.rsplit(',').next())
                .and_then(|ip| ip.trim().parse().ok()),
            ClientIpHeader::RealIp => {
                header_str(headers, "x-real-ip").and_then(|ip| ip.trim().parse().ok())
            }
            ClientIpHeader::None => None,
        };

        from_header
            .or_else(|| {
                req.extensions()
                    .get::<ConnectInfo<SocketAddr>>()
                    .map(|info| info.0.ip())
            })
            .ok_or(GovernorError::UnableToExtractKey)
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// 오래된 IP 버킷을 주기적으로 비운다. 종료 신호를 받으면 끝난다.
// [주의] 이 태스크가 없으면 IP별 버킷이 **계속 쌓이기만 한다.** governor의 keyed
// limiter는 스스로 줄어들지 않기 때문이다. 요청량이 적어도 시간이 지나면서 서로 다른
// IP가 누적되므로, 오래 켜 두는 서비스라면 반드시 있어야 한다.
//
// [설명] 크레이트의 공식 예제는 이것을 `std::thread::spawn`의 무한 루프로 띄운다. 그러면
// 종료 신호를 볼 방법이 없어서 프로세스가 내려갈 때까지 남는데, 이 서비스는 종료를
// 토큰 하나로 다루므로 같은 규칙에 맞춘다(server/mod.rs).
pub async fn prune(config: Arc<Config>, shutdown: CancellationToken) {
    let mut ticker = tokio::time::interval(PRUNE_INTERVAL);
    // 첫 tick은 즉시 발화하므로 한 번 흘려보낸다 — 기동하자마자 빈 맵을 청소할 이유가 없다.
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let limiter = config.limiter();
                let before = limiter.len();
                limiter.retain_recent();
                tracing::debug!(before, after = limiter.len(), "pruned rate limit buckets");
            }
            _ = shutdown.cancelled() => {
                tracing::info!("rate limit pruner stopped");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(headers: &[(&str, &str)], peer: Option<&str>) -> Request<()> {
        let mut builder = Request::builder();
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let mut req = builder.body(()).unwrap();
        if let Some(peer) = peer {
            req.extensions_mut()
                .insert(ConnectInfo::<SocketAddr>(peer.parse().unwrap()));
        }
        req
    }

    fn key(source: ClientIpHeader, req: &Request<()>) -> IpAddr {
        ClientIpKeyExtractor(source).extract(req).unwrap()
    }

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    // [테스트 시나리오] 덧붙이는 프록시 뒤에서 클라이언트가 x-forwarded-for를 위조해도, 키는
    // 프록시가 덧붙인 맨 오른쪽 값이다.
    #[test]
    fn forwarded_for_uses_the_rightmost_entry() {
        let req = request(&[("x-forwarded-for", "6.6.6.6, 203.0.113.7")], None);

        assert_eq!(key(ClientIpHeader::ForwardedFor, &req), ip("203.0.113.7"));
    }

    // [테스트 시나리오] x-forwarded-for를 고른 배포에서는 클라이언트가 보낸 x-real-ip를 보지 않는다.
    #[test]
    fn the_unselected_header_is_ignored() {
        let req = request(
            &[("x-real-ip", "6.6.6.6"), ("x-forwarded-for", "203.0.113.7")],
            None,
        );

        assert_eq!(key(ClientIpHeader::ForwardedFor, &req), ip("203.0.113.7"));
    }

    #[test]
    fn real_ip_reads_x_real_ip() {
        let req = request(&[("x-real-ip", "203.0.113.7")], None);

        assert_eq!(key(ClientIpHeader::RealIp, &req), ip("203.0.113.7"));
    }

    // [테스트 시나리오] 헤더가 없거나 none이면 peer IP로 떨어진다.
    #[test]
    fn falls_back_to_the_peer_ip() {
        let spoofed = request(&[("x-forwarded-for", "6.6.6.6")], Some("10.0.0.1:5000"));
        let without_header = request(&[], Some("10.0.0.1:5000"));

        assert_eq!(key(ClientIpHeader::None, &spoofed), ip("10.0.0.1"));
        assert_eq!(key(ClientIpHeader::RealIp, &without_header), ip("10.0.0.1"));
    }
}
