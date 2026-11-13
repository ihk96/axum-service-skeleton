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

# [핵심] .cargo/config.toml에는 SQLX_OFFLINE=true가 들어 있다. sqlx의 query! 계열 매크로는
# 기본적으로 컴파일 타임에 DATABASE_URL로 실제 DB에 접속해 SQL을 검증하는데, 이미지 빌드
# 환경에는 그런 DB가 없다(있어서도 안 된다 — 빌드가 특정 개발 머신의 DB에 의존하게 되므로).
# 이 설정이 있어야 매크로가 대신 아래에서 복사하는 .sqlx/ 캐시를 보고 검증한다.
#
# [설계] 같은 값을 여기서 ENV SQLX_OFFLINE=true로 박을 수도 있다. 다만 그러면 동일한 설정이
# 두 군데에 존재해 한쪽만 고치는 사고가 나므로, "이 도커 빌드의 사정"이 아니라 "프로젝트의
# 성질"에 해당하는 값은 프로젝트가 이미 가진 설정 파일을 그대로 가져오는 쪽을 택했다.
COPY .cargo ./.cargo

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

# `cargo sqlx prepare`가 만들어 저장소에 커밋해 둔 쿼리 검증 캐시. 각 쿼리의 SQL 문자열과
# 그때 DB가 알려준 컬럼 타입/NULL 허용 여부가 JSON으로 들어 있어서, 매크로가 DB 없이도
# 같은 검증을 수행할 수 있다.
#
# [주의] SQL을 고칠 때마다 갱신해야 한다(`cargo sqlx prepare -- --all-targets`). 갱신을
# 빠뜨린 채 쿼리를 바꾸면 캐시에 없는 쿼리라며 여기서 빌드가 실패하므로 대개 바로 드러난다.
# 다만 SQL은 그대로인데 마이그레이션으로 컬럼 타입만 바뀐 경우는 빌드가 통과하고 런타임에
# 터지므로, 마이그레이션을 추가했다면 쿼리를 안 고쳤더라도 prepare를 다시 돌릴 것.
COPY .sqlx ./.sqlx

# 진짜 빌드. 캐시 레이어 트릭의 유일한 함정이 여기 있다 — Cargo는 소스 파일의 **수정 시각**을
# 보고 다시 빌드할지 정하는데, COPY는 빌드 컨텍스트의 수정 시각을 그대로 가져온다. 그래서 우리
# 소스가 방금 만든 더미 산출물보다 "오래된" 것으로 보이면, Cargo가 이미 빌드된 것으로 착각해
# 빈 main()이 든 바이너리가 그대로 최종 이미지에 실린다.
#
# [설계] 그래서 산출물을 지우는 대신 소스를 전부 touch해 확실히 새것으로 만든다. 산출물을
# 지우려면 크레이트/bin 이름을 알아야 하지만(target/release/deps/<크레이트 이름>-*), 이 방식은
# 이름을 몰라도 되므로 프로젝트 이름을 바꾸거나 bin을 추가해도 이 줄은 그대로다.
RUN find src -name '*.rs' -exec touch {} + \
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
# 이 계정이 쓸 공간은 아래 DATA_DIR 하나뿐이므로 홈 디렉토리는 만들지 않는다.
#
# [설명] uid를 숫자로 고정한다. 자동 할당된 uid는 베이스 이미지의 패키지 구성에 따라
# 달라질 수 있는데, bind mount된 호스트 디렉토리의 소유권은 이름이 아니라 숫자로
# 비교되므로 이미지를 다시 구웠더니 권한이 어긋나는 일이 생길 수 있다.
RUN useradd --system --user-group --no-create-home --uid 10001 appuser

# 서비스가 파일을 쓸 유일한 위치. 업로드 저장처럼 디스크에 남겨야 하는 것이 생기면
# 전부 이 아래에 둔다(`$DATA_DIR/uploads` 같은 식으로) — 컨테이너가 권한을 책임지는
# 범위가 여기 하나로 정해져 있어야 entrypoint가 할 일이 단순해진다.
#
# [설계] 같은 값을 config.rs도 DATA_DIR로 읽는다. 앱이 쓰는 경로와 컨테이너가
# 준비하는 경로가 어긋나면 런타임에야 드러나므로, 양쪽이 같은 환경변수 하나를 본다.
# 기본값이 코드 쪽은 ./data인데 여기서 덮어쓰는 이유는, 로컬 `cargo run`은 작업
# 디렉토리 옆에 두는 편이 편하고 컨테이너는 FHS를 따르는 편이 낫기 때문이다.
ENV DATA_DIR=/var/lib/app
RUN mkdir -p "$DATA_DIR" && chown appuser:appuser "$DATA_DIR"

# 바이너리를 PATH(/usr/local/bin)에 넣는다. CLI bin을 추가했다면 여기에 한 줄 더
# 두면 되고, 그러면 서버가 도는 컨테이너에 대고 `docker exec <container> <cli> ...`로
# 바로 실행할 수 있다 — docker exec는 셸을 거치지 않고 PATH에서 실행 파일을 찾는다.
COPY --from=builder /app/target/release/server /usr/local/bin/server

# [주의] chmod를 여기서 한 번 더 건다. 체크아웃하는 환경(특히 Windows)에 따라 실행
# 비트가 보존되지 않아, 이것이 없으면 "permission denied"로 컨테이너가 뜨지 않는다.
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# [주의] 여기에 USER appuser가 없는 것은 실수가 아니다. bind mount로 붙은 호스트
# 디렉토리는 위 chown을 덮어버리므로, 소유권은 마운트가 끝난 뒤에야 맞출 수 있다.
# 그래서 컨테이너는 root로 시작하고 entrypoint가 정리한 뒤 스스로 appuser로 내려앉는다
# (docker-entrypoint.sh 참고). 서버 프로세스 자체는 결국 appuser로 돈다.
#
# [주의] 쿠버네티스에서 securityContext.runAsNonRoot를 걸면 "이미지가 root로 시작한다"는
# 이유로 파드가 아예 뜨지 못한다. 그런 환경이라면 여기에 USER appuser를 되살리고,
# 대신 볼륨 소유권은 fsGroup으로 맡기면 된다 — 쿠버네티스는 그 일을 대신 해준다.

EXPOSE 8080

# [설명] 설정은 이미지에 박지 않고, 컨테이너가 뜰 때 마운트되는 /app/.env를 앱이 직접
# 읽는다(bin/server.rs). 그래서 작업 디렉토리를 그 자리로 맞춰 둔다 — 앱은 "작업 디렉토리의
# .env"를 찾으므로 로컬 `cargo run`과 컨테이너가 같은 규칙으로 돈다.
#
# [주의] 여기에 ENV를 늘리지 말 것. dotenvy는 이미 있는 환경변수를 덮어쓰지 않으므로, 이미지
# ENV로 박은 값은 .env로 바꿀 수 없게 된다(RUST_LOG 기본값을 코드로 옮긴 이유). 예외는
# 앱보다 먼저 도는 entrypoint가 써야 하는 위의 DATA_DIR뿐이다.
WORKDIR /app

# [설명] ENTRYPOINT는 권한 정리 스크립트이고, 실제로 무엇을 실행할지는 여전히 CMD가
# 정한다. 스크립트가 마지막에 `exec "$@"`로 CMD를 그대로 실행하므로, `docker run
# <image> <cli> ...`처럼 인자를 주면 통째로 덮어쓰는 성질도 그대로다 — 같은 이미지로
# CLI를 돌리는 경우에도 권한 정리는 똑같이 거쳐 간다.
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["server"]
