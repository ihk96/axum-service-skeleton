# axum-service-skeleton

axum + sqlx + tokio 백엔드 서비스의 골격. 복사해서 새 서비스를 시작하기 위한 것이고,
**DB 없이 `cargo test`가 돈다**.

## 시작하기

```bash
cp -r axum-service-skeleton my-service
cd my-service
```

그리고 세 곳의 이름을 바꾼다:

1. `Cargo.toml`의 `name`
2. `src/bin/server.rs`의 `use axum_service_skeleton::...`
3. `README.md` 제목

```bash
cp .env.example .env   # DATABASE_URL 수정
cargo test             # DB 없이 통과해야 정상
cargo run              # 서버 (= cargo run --bin server)
```

## Docker

```bash
docker build -t my-service .
docker run --rm -p 8080:8080 -e DATABASE_URL=postgres://... my-service
```

`docker-compose.yml`은 **위에서 구운 이미지로 컨테이너를 띄우는 예시**다. 이미지를 만드는 일은
하지 않으니 `docker build`가 먼저다. 그대로 쓰기보다 배포 환경에 맞게 고쳐 쓰는 것을 전제로 한
가이드이고, 지울 곳/주입할 곳은 파일 안에 주석으로 적어뒀다.

```bash
docker compose up -d            # 이미지가 이미 있어야 한다
docker compose up -d postgres   # 개발 중에는 DB만 이걸로 띄우고 서버는 cargo run
```

위의 **"이름 바꿀 세 곳"에 Dockerfile은 없다** — 크레이트 이름이 아니라 bin 이름(`server`)만
참조하기 때문이다. 대신 **bin을 추가하면** 두 곳을 손봐야 한다: 캐시 레이어의 더미 파일과
최종 스테이지의 `COPY`. 그렇게 넣어둔 CLI는 서버가 도는 컨테이너에 대고
`docker exec <container> <cli> ...`로 바로 실행할 수 있다.

`.dockerignore`에 `.env`가 들어 있다. 이미지 레이어에 비밀값이 박히지 않게 하기 위한 것이고,
컨테이너 설정은 주입된 환경변수로 넣는다(`dotenvy`는 기존 환경변수를 덮어쓰지 않는다).

## 지울 것

`example`이 붙은 파일들이 예제다. 새 기능을 추가할 때 모양을 그대로 따라 쓰고, 다 쓴 뒤엔 지운다.

```
src/store/example.rs        포트 trait + Postgres 구현
src/domain/example.rs       도메인 로직 (포트를 &dyn으로 받음)
src/domain/example_task.rs  백그라운드 태스크
src/routes/example.rs       HTTP 핸들러
migrations/0001_init.sql    예제 테이블
```

지울 때 함께 손볼 곳: `src/domain/mod.rs`, `src/routes/mod.rs`, `src/store/mod.rs`의 모듈 선언,
`src/state.rs`의 필드, `src/config.rs`의
`example_sweep_interval_secs`, `src/server/mod.rs`의 `tokio::spawn`.

## 설계 규칙

전체 구조와 그 이유는 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)에 있다. 다섯 줄로 줄이면:

1. **로직은 lib, bin은 껍데기.** bin 안의 코드는 다른 bin도 통합 테스트도 재사용할 수 없다.
2. **모듈은 "다른 프로젝트로 옮겼을 때 내용이 그대로인가"로 가른다.** 그대로면 골격, 아니면 `domain/`.
3. **`AppState`는 `Clone` 없이 `new() -> Arc<Self>`.** 공유는 항상 Arc clone.
4. **백그라운드 태스크 진입점은 `Arc<AppState>`, 로직은 좁은 `&dyn` 포트.**
5. **포트는 trait + `automock`.** 그래서 DB 없이 테스트가 돈다.

## 들어있지 않은 것

의도적으로 뺐다. 필요할 때 넣는 게 낫다고 본 것들:

- **인증** — API key / JWT / OIDC 중 무엇이냐에 따라 모양이 완전히 달라진다. axum의
  `FromRequestParts`로 extractor를 만들어 "검증된 상태"를 타입으로 표현하는 패턴을 쓸 것.
- **아웃바운드 HTTP** — reqwest를 포트(`trait Upstream`) 뒤에 두고 어댑터로 감싼다.
  `store/`와 구조가 같다.
- **캐시 / 이벤트** — 폴더 이름은 기술이 아니라 포트 이름으로 (`redis/`가 아니라 `cache/`,
  `kafka/`가 아니라 `events/`). 단 kafka는 프로듀서가 아웃바운드, 컨슈머는 인바운드라
  한 폴더에 섞으면 안 된다 — 컨슈머는 `server/` 옆에 `consumer/`로 둔다.
- **`/metrics`, `/version`** — 운영용 엔드포인트. 노출 범위 때문에 별도 포트(9090 등)에
  올리는 경우가 많아서, 그 결정과 함께 넣는 게 낫다.
- **`tests/` 통합 테스트** — `server::run`이 lib에 있으므로 가능하지만 실제 Postgres가
  필요하다. 지금은 모든 테스트가 lib 안의 단위 테스트이고, DB 없이 돈다.
- **공용 테스트 헬퍼(`test_support`)** — 각 테스트가 필요한 mock을 직접 만들고 `AppState`를
  직접 조립한다. 포트가 5~6개를 넘어가면 "관심 없는 필드 채우기"가 반복되므로 그때 빌더를
  뽑아내는 게 낫다 — `#[cfg(test)] pub mod`으로 두면 크레이트 전체가 공유할 수 있다.
