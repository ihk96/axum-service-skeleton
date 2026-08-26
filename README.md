# axum-service-skeleton

axum + sqlx + tokio 백엔드 서비스의 골격. 복사해서 새 서비스를 시작하기 위한 것이다.
포트를 전부 `automock`으로 바꿔 끼우므로 **테스트는 DB 없이 돌고**, 빌드도 커밋된 `.sqlx/`
캐시만 있으면 DB 없이 성립한다.

> **먼저 읽을 것.** 이 저장소에는 `.sqlx/` 캐시가 들어 있지 않아서, 클론 직후에는
> `cargo build`도 `cargo test`도 실패한다. 아래 [최초 1회: 쿼리 캐시 만들기](#최초-1회-쿼리-캐시-만들기)를
> 한 번 거치면 그 뒤로는 DB 없이 돈다. 이유도 그 절에 적어뒀다.

## 시작하기

```bash
cp -r axum-service-skeleton my-service
cd my-service
```

그리고 세 곳의 이름을 바꾼다:

1. `Cargo.toml`의 `name`
2. `src/bin/server.rs`의 `use axum_service_skeleton::...`
3. `README.md` 제목

### 최초 1회: 쿼리 캐시 만들기

SQL은 sqlx의 `query!` 계열 매크로로 쓰여 있어 **컴파일 타임에 스키마와 대조**된다. 그 대조는
저장소에 커밋된 `.sqlx/` 캐시를 보고 이뤄지는데, 이 스켈레톤에는 캐시가 들어 있지 않다 —
새 서비스는 어차피 예제 테이블을 지우고 자기 스키마를 쓸 것이므로, 남의 스키마로 만든 캐시를
물려받아 봐야 곧바로 무효가 되기 때문이다.

그래서 클론 직후 한 번은 DB를 붙여 캐시를 만들어야 하며, **그 전에는 `cargo build`가 실패한다.**

```bash
cargo install sqlx-cli --no-default-features --features rustls,postgres

# 1. DB 준비 (이미 쓰는 Postgres가 있으면 database만 하나 파면 된다)
#    OWNER를 반드시 지정할 것 — Postgres 15부터 public 스키마에 아무나 테이블을
#    만들 수 없어서, 소유자가 아니면 마이그레이션이 permission denied로 실패한다.
#      CREATE ROLE my_service LOGIN PASSWORD '...';
#      CREATE DATABASE my_service OWNER my_service;

# 2. 접속 정보를 .env에 (DATABASE_URL 한 줄이면 된다)
echo 'DATABASE_URL=postgres://my_service:...@127.0.0.1:5432/my_service' > .env

# 3. 스키마 적용 → 캐시 생성
sqlx migrate run
cargo sqlx prepare -- --all-targets
```

생성된 `.sqlx/`는 **커밋한다.** 이 캐시가 곧 "DB 없이 빌드할 수 있는" 근거다.

### 이후의 일상

```bash
cargo test             # DB 없이 통과해야 정상
cargo build            # DB 없이 통과해야 정상
cargo run              # 서버 (= cargo run --bin server)
```

`.env`는 gitignore 대상이고 `DATABASE_URL` 한 줄이면 충분하다. sqlx-cli가 DB에 붙기 위한
것이지 애플리케이션 설정 창구가 아니다 — 배포 시 주입할 값의 목록은 `.env.example`에 있고,
그쪽을 개발 머신으로 복사해 올 이유는 없다.

### SQL을 고칠 때

쿼리나 마이그레이션을 건드렸다면 DB를 켜고 두 단계를 거친다. **순서가 중요하다** — 매크로는
그 시점 DB에 실제로 존재하는 스키마를 보고 타입을 뽑으므로, 마이그레이션이 먼저 적용되어
있어야 한다.

```bash
sqlx migrate run                      # 1. 스키마를 최신으로
cargo sqlx prepare -- --all-targets   # 2. .sqlx/ 캐시 갱신 → 커밋
```

`--all-targets`를 생략하면 lib 타겟만 스캔해서, 테스트나 `src/bin/`에 매크로가 들어갔을 때
그 쿼리가 캐시에서 빠진다.

**놓치기 쉬운 경우가 하나 있다.** SQL 문자열은 그대로인데 마이그레이션으로 컬럼 타입만
바뀌면, 캐시가 낡아도 빌드는 통과하고 런타임에 터진다. 마이그레이션을 추가했다면 쿼리를
안 고쳤더라도 `prepare`를 다시 돌릴 것. CI에서 `cargo sqlx prepare --check`로 강제할 수
있지만, 그러려면 CI에도 Postgres가 필요하다.

### SQL은 어디까지 검증되나

매크로가 잡아주는 건 컬럼 이름·타입·NULL 허용 여부, 즉 "SQL이 스키마와 맞는가"까지다.
FK/cascade/unique 같은 **제약의 런타임 동작**은 여전히 실제 DB에 붙여야 확인되며, 그래서
`store/example.rs`의 unique 위반 → `Conflict` 번역 같은 코드가 계속 필요하다.

매크로를 쓸 수 없는 경우도 둘 있다. 인자에 따라 SQL 자체가 달라지는 **동적 쿼리**(조건부
WHERE, 가변 SET 절)는 컴파일 시점에 검증할 문자열이 없으므로 `sqlx::QueryBuilder`로
조립하고, 그 타입에는 `#[derive(sqlx::FromRow)]`를 남긴다. 그리고 `store/health.rs`의
`select 1`처럼 스키마를 참조하지 않는 쿼리는 검증할 것이 없어 그대로 둔다.

오프라인 설정은 [.cargo/config.toml](.cargo/config.toml)에 있다. 셸 환경변수가 아니라 이
파일에 두는 이유는 rust-analyzer가 띄우는 `cargo check`도 같은 설정을 읽어야 하기 때문이다 —
셸에만 걸면 터미널 빌드는 되는데 에디터에는 매크로마다 에러가 뜬다.

## Docker

```bash
docker build -t my-service .
docker run --rm -p 8080:8080 -e DATABASE_URL=postgres://... my-service
```

빌드 환경에 Postgres가 필요 없다. `.cargo/config.toml`과 `.sqlx/`가 함께 복사되어, 매크로가
DB 대신 캐시를 보고 검증한다. 바꿔 말하면 **캐시를 커밋하지 않았다면 이미지 빌드도 실패한다.**

`docker-compose.yml`은 **위에서 구운 이미지로 컨테이너를 띄우는 예시**다. 이미지를 만드는 일은
하지 않으니 `docker build`가 먼저다. 그대로 쓰기보다 배포 환경에 맞게 고쳐 쓰는 것을 전제로 한
가이드이고, 지울 곳/주입할 곳은 파일 안에 주석으로 적어뒀다.

DB 서비스는 들어 있지 않다 — 이미 있는 Postgres에 붙는 것을 전제로 하며, 접속 정보는
`DATABASE_URL`로 주입한다. compose가 DB까지 띄우게 하려면 postgres 서비스와 볼륨을 직접
추가하고 `depends_on`을 걸면 된다.

```bash
docker compose up -d   # 이미지가 이미 있어야 하고, .env에 DATABASE_URL이 있어야 한다
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
src/domain/example_job.rs   스케줄 job
src/routes/example.rs       HTTP 핸들러
migrations/0001_init.sql    예제 테이블
```

지울 때 함께 손볼 곳: `src/domain/mod.rs`, `src/routes/mod.rs`, `src/store/mod.rs`의 모듈 선언,
`src/state.rs`의 필드, 그리고 `src/domain/jobs.rs`의 등록 항목(파일 자체는 남긴다).

`scheduler/`와 `migrations/0002_scheduled_job_runs.sql`, `src/store/job_run.rs`는 예제가 아니라
골격이다. 배치를 쓰지 않기로 했다면 셋을 함께 지우고 `AppState`의 `job_run_store`와
`server/mod.rs`의 `Scheduler` 두 줄을 걷어낸다.

그리고 **예제를 지운 뒤 `cargo sqlx prepare`를 다시 돌린다.** 캐시에는 지워진 쿼리의
검증 데이터가 그대로 남아 있어서, 갱신하지 않으면 존재하지 않는 테이블의 정보를 계속
커밋하게 된다.

## 설계 규칙

전체 구조와 그 이유는 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)에 있다. 다섯 줄로 줄이면:

1. **로직은 lib, bin은 껍데기.** bin 안의 코드는 다른 bin도 통합 테스트도 재사용할 수 없다.
2. **모듈은 "다른 프로젝트로 옮겼을 때 내용이 그대로인가"로 가른다.** 그대로면 골격, 아니면 `domain/`.
3. **`AppState`는 `Clone` 없이 `new() -> Arc<Self>`.** 공유는 항상 Arc clone.
4. **백그라운드 진입점은 `Arc<AppState>`, 로직은 좁은 `&dyn` 포트.** 스케줄 job도 태스크도 같다.
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
