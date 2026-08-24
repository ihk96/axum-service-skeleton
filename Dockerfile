# 멀티스테이지 빌드. builder에는 Rust 툴체인(수백 MB)이 필요하지만 실행에는 전혀
# 필요 없으므로, 최종 이미지에는 컴파일된 바이너리만 옮겨 담는다. FROM이 여러 개
# 있어도 마지막 스테이지만 최종 이미지가 된다.
#
# [스켈레톤] 프로젝트 이름을 바꿔도 이 파일은 고칠 필요가 없다 — 참조하는 것이
# 크레이트 이름이 아니라 bin 이름(`server`)뿐이기 때문. 단 bin을 추가하면 아래
# 두 곳을 손봐야 한다: 캐시 레이어의 더미 파일, 그리고 최종 스테이지의 COPY.

# ---------------------------------------------------------------------------
# builder
# ---------------------------------------------------------------------------
# bookworm(Debian 12) 기반. 최종 스테이지의 debian:bookworm-slim과 배포판/glibc
# 버전이 같아야 한다 — 여기서 동적 링크된 바이너리를 그쪽에서 실행하기 때문에,
# 베이스가 어긋나면 glibc 버전 불일치로 실행이 안 될 수 있다.
FROM rust:1-bookworm AS builder

WORKDIR /app

# 의존성 캐시 레이어. 매니페스트만 먼저 복사하고 "내용이 빈 소스"로 한 번 빌드해서,
# axum/sqlx/tokio 같은 외부 크레이트의 컴파일 결과를 이 레이어에 굳혀둔다. 이후
# 우리 소스만 바뀌면 Docker는 Cargo.toml/Cargo.lock이 그대로인 걸 보고 이 레이어를
# 재사용하므로, 재빌드가 "우리 크레이트 하나"로 줄어든다. 풀빌드는 릴리즈 프로필의
# lto = true 때문에 수 분 단위로 걸린다(거슬리면 lto = "thin"이 첫 손볼 지점).
COPY Cargo.toml Cargo.lock ./
# Cargo는 src/bin/ 아래 파일 이름으로 바이너리를 인식하고, Cargo.toml의
# default-run = "server"도 그 이름이 실제로 존재할 것을 요구한다. 그래서 더미도
# 실제 bin 구성과 같은 이름이어야 이 단계가 통과한다.
RUN mkdir -p src/bin \
    && echo 'fn main() {}' > src/bin/server.rs \
    && touch src/lib.rs \
    && cargo build --release \
    && rm -rf src

# migrations/도 반드시 함께 들어와야 한다 — server/mod.rs의 `sqlx::migrate!()`는
# 런타임이 아니라 **컴파일 타임**에 이 디렉토리를 읽어 SQL을 바이너리에 내장시키는
# 매크로다. 덕분에 최종 이미지에는 migrations/를 복사하지 않아도 동작한다.
COPY src ./src
COPY migrations ./migrations

# 위 더미 빌드의 산출물을 지우고 진짜 빌드를 한다. 더미와 실제 소스의 크레이트
# 이름이 같아서, 지우지 않으면 Cargo가 이미 빌드된 것으로 착각해 빈 main()이 든
# 바이너리가 그대로 최종 이미지에 실릴 수 있다. 캐시 레이어 트릭의 유일한 함정.
RUN rm -f target/release/server \
    && rm -f target/release/deps/server-* \
    && rm -f target/release/deps/*axum_service_skeleton* \
    && cargo build --release

# ---------------------------------------------------------------------------
# runtime
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim

# apt로 아무것도 설치하지 않는다. 지금 의존성 중 시스템 라이브러리를 요구하는 것이
# 없기 때문(sqlx는 순수 Rust 드라이버이고 TLS feature도 켜지 않았다).
#
# [스켈레톤] 아웃바운드 HTTPS를 붙일 때 여기가 손볼 지점이다. reqwest를
# rustls-tls로 쓰면 루트 인증서가 크레이트에 내장되므로 이대로도 되지만,
# native-tls나 rustls-tls-native-roots를 고르면 ca-certificates 설치가 필요하다.

# 루트로 서비스를 돌리지 않기 위한 전용 계정. --system은 로그인 불가 계정을 만들고,
# 컨테이너 안에서 파일을 쓸 일이 없으니 홈 디렉토리도 만들지 않는다.
RUN useradd --system --user-group --no-create-home appuser

# 바이너리를 PATH(/usr/local/bin)에 넣는다. CLI bin을 추가했다면 여기에 한 줄 더
# 두면 되고, 그러면 서버가 도는 컨테이너에 대고 `docker exec <container> <cli> ...`로
# 바로 실행할 수 있다 — docker exec는 셸을 거치지 않고 PATH에서 실행 파일을 찾는다.
COPY --from=builder /app/target/release/server /usr/local/bin/server

USER appuser

# EXPOSE는 포트를 실제로 열어주는 게 아니라 "이 이미지는 이 포트를 쓴다"는 메타데이터다.
# 실제 바인딩은 SERVER_ADDR로 정하며 기본값이 0.0.0.0:8080(config.rs)이다 — 127.0.0.1로
# 바인딩하면 컨테이너 네트워크 네임스페이스에 갇혀서 포트 매핑이 무용지물이 된다.
EXPOSE 8080

# bin/server.rs의 EnvFilter::from_default_env()는 RUST_LOG가 비어 있으면 사실상
# 아무것도 출력하지 않는다. 컨테이너에서는 기본값을 박아둔다.
ENV RUST_LOG=info

# ENTRYPOINT가 아니라 CMD인 이유 — CMD는 `docker run <image> <cli> ...`처럼 실행 인자를
# 주면 통째로 덮어쓸 수 있다. ENTRYPOINT ["server"]로 두면 뒤에 붙는 인자가 server의
# 인자로 해석돼서 같은 이미지로 CLI를 돌리기 번거로워진다.
CMD ["server"]
