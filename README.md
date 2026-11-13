# axum-service-skeleton

axum + sqlx(Postgres) + tokio 백엔드 서비스의 골격. 복사해서 새 서비스를 시작하기 위한 것이고,
Docker 컨테이너로 배포하는 것(blue/green 교체 포함)을 전제로 한다.

포트(저장소 등)를 전부 trait + `automock`으로 두어서 **테스트는 DB 없이 돌고**, 빌드도 커밋된
`.sqlx/` 캐시만 있으면 DB 없이 성립한다.

> **먼저 읽을 것.** 이 저장소에는 `.sqlx/` 캐시가 들어 있지 않아서, 클론 직후에는
> `cargo build`도 `cargo test`도 실패한다. 아래 [최초 1회: 쿼리 캐시 만들기](#최초-1회-쿼리-캐시-만들기)를
> 한 번 거치면 그 뒤로는 DB 없이 돈다. 이유도 그 절에 적어뒀다.

## 들어 있는 것

| 영역 | 내용 |
|---|---|
| 계층 | `domain/`(순수 규칙) → `usecase/`(조립·트랜잭션) → `store/`(저장소) / `http/`(HTTP 어댑터) |
| 저장소 | 쓰기·검증의 `Store`, 조회 전용의 `Views` 두 진입점. 트랜잭션은 usecase가 `store.begin()` → `commit()` |
| HTTP | 요청 ID, 요청 로그, 패닉 → 500, CORS(설정 시), 요청 타임아웃(503), IP별 레이트 리밋 |
| 에러 | 모든 에러 응답이 `{"error": "..."}` 한 형식(핸들러 에러, 요청 파싱 실패, 429 포함). 5xx 상세는 로그에만 |
| 운영 | `/health`(liveness), `/ready`(readiness), graceful shutdown, 기동 시 마이그레이션(롤백 가능) |
| 배치 | 크론식 스케줄러. 인스턴스가 여럿이어도 발화당 한 번만 실행 |
| 배포 | 멀티스테이지 Dockerfile, root가 아닌 계정으로 실행, `.env`와 데이터 폴더 두 볼륨 |

구조와 각 결정의 이유는 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)에 있다.

## 시작하기

```bash
cp -r axum-service-skeleton my-service
cd my-service
```

그리고 세 곳의 이름을 바꾼다:

1. `Cargo.toml`의 `name`
2. `src/bin/server.rs`의 `use axum_service_skeleton::...`
3. `README.md` 제목

이미지 태그(`docker-compose.yml`의 `my-service`)는 배포 환경에 맞게 정한다.

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
cargo run              # 서버 (= cargo run --bin server). 이건 DB가 필요하다
```

`cargo run`은 기동하면서 마이그레이션을 적용하고 DB에 붙으므로 `.env`의 `DATABASE_URL`이 가리키는
Postgres가 떠 있어야 한다. 빌드와 테스트만 DB 없이 된다.

`.env`는 gitignore 대상이고, 개발 머신에서는 sqlx-cli와 `cargo run`이 함께 읽는다. 나머지
설정은 전부 기본값이 있어서 `DATABASE_URL` 한 줄이면 충분하다. 기본값을 바꿔 보고 싶을 때만
`.env.example`에서 필요한 줄을 옮겨 온다. 앱은 **작업 디렉토리의 `.env`** 하나만 읽으므로
(상위 디렉토리를 찾아 올라가지 않는다), 프로젝트 루트에서 실행한다.

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

마이그레이션을 쓸 때는 **구버전 코드와 공존할 수 있게** 쓴다(컬럼 삭제·이름 변경을 한 번에 하지
않는 등). blue/green 배포와 롤백이 그걸 전제로 한다 — 규칙은 ARCHITECTURE.md의
"마이그레이션 규칙"에 있다.

### SQL은 어디까지 검증되나

매크로가 잡아주는 건 컬럼 이름·타입·NULL 허용 여부, 즉 "SQL이 스키마와 맞는가"까지다.
FK/cascade/unique 같은 **제약의 런타임 동작**은 여전히 실제 DB에 붙여야 확인되며, 그래서
`store/example/store.rs`의 unique 위반 → `Conflict` 번역 같은 코드가 계속 필요하다.

매크로를 쓸 수 없는 경우도 둘 있다. 인자에 따라 SQL 자체가 달라지는 **동적 쿼리**(조건부
WHERE, 가변 SET 절)는 컴파일 시점에 검증할 문자열이 없으므로 `sqlx::QueryBuilder`로
조립하고, 그 타입에는 `#[derive(sqlx::FromRow)]`를 단다. 그리고 `store/health.rs`의
`select 1`처럼 스키마를 참조하지 않는 쿼리는 검증할 것이 없어 그대로 둔다.

오프라인 설정은 [.cargo/config.toml](.cargo/config.toml)에 있다. 셸 환경변수가 아니라 이
파일에 두는 이유는 rust-analyzer가 띄우는 `cargo check`도 같은 설정을 읽어야 하기 때문이다 —
셸에만 걸면 터미널 빌드는 되는데 에디터에는 매크로마다 에러가 뜬다.

## 새 기능 추가하기

예제(`example`)가 이 순서의 견본이다. 안쪽(규칙)에서 바깥쪽(HTTP)으로 만든다.

1. **스키마** — `migrations/`에 SQL 추가(구버전과 공존하게), `sqlx migrate run`
2. **규칙** — `domain/<영역>.rs`에 검증·계산 같은 순수 함수. 저장소를 부르면 여기가 아니다
3. **저장소** — `store/<영역>/store.rs`(쓰기·검증), `views.rs`(응답용 조회)에 trait + 구현.
   `store/mod.rs`에 모듈을 선언하고, `store/entry.rs`의 `entries!` 목록에 한 줄씩 등록한다
4. **유스케이스** — `usecase/<영역>.rs`. 여러 쓰기를 묶어야 하면 `&dyn Store`를 받아
   `begin()` → `commit()`, 아니면 도메인 store/view를 하나씩 받는다
5. **HTTP** — `http/routes/<영역>.rs`에 핸들러, `http/routes/mod.rs`에 모듈 선언,
   `http/mod.rs`의 `build()`에 `.merge()` 한 줄. 요청 body/경로는 axum의 것 대신
   `http::extract`의 `Json`/`Path`로 받는다(파싱 실패도 같은 에러 형식이 되도록)
6. **캐시** — `cargo sqlx prepare -- --all-targets` 후 `.sqlx/`까지 커밋

AppState와 기존 테스트는 건드리지 않는다. 배치 job이라면 5 대신 `usecase/`에 job 함수를 두고
`usecase/jobs.rs`에 한 줄 등록한다.

## Docker

### 이미지 빌드

```bash
docker build -t my-service .
```

빌드 환경에 Postgres가 필요 없다. `.cargo/config.toml`과 `.sqlx/`가 함께 복사되어, 매크로가
DB 대신 캐시를 보고 검증한다. 바꿔 말하면 **캐시를 커밋하지 않았다면 이미지 빌드도 실패한다.**

Windows에서 클론해 빌드해도 된다. `.gitattributes`가 셸 스크립트를 항상 LF로 체크아웃하게
고정해 두었다 — CRLF로 바뀐 `docker-entrypoint.sh`는 컨테이너 안에서 실행되지 않는다.

Dockerfile에는 크레이트 이름이 없다(bin 이름 `server`만 참조한다). 그래서 위 "이름 바꿀 세 곳"에
Dockerfile은 없다. 대신 **bin을 추가하면** 두 곳을 손봐야 한다: 캐시 레이어의 더미 파일과 최종
스테이지의 `COPY`. 그렇게 넣어둔 CLI는 서버가 도는 컨테이너에 대고
`docker exec <container> <cli> ...`로 바로 실행할 수 있다.

### 실행

```bash
cp .env.example .env   # 값 채우기 (배포 서버라면 그 서버에서)
docker compose up -d   # 이미지가 이미 있어야 한다

# compose 없이 띄운다면
docker run --rm -p 8080:8080 -v "$PWD/.env:/app/.env:ro" -v "$PWD/data:/var/lib/app" my-service
```

`docker-compose.yml`은 **이미 구운 이미지로 컨테이너를 띄우는 예시**다. 이미지를 만드는 일은
하지 않으니 `docker build`가 먼저다. 그대로 쓰기보다 배포 환경에 맞게 고쳐 쓰는 것을 전제로 한
가이드이고, 고칠 곳은 파일 안에 주석으로 적어뒀다. DB 서비스는 들어 있지 않다 — 이미 있는
Postgres에 붙는 것을 전제로 한다(compose가 DB까지 띄우게 하려면 postgres 서비스와 볼륨을 직접
추가하고 `depends_on`을 건다).

컨테이너에는 볼륨 두 개를 붙인다.

| 컨테이너 경로 | 용도 |
|---|---|
| `/app/.env` (읽기 전용) | 설정. entrypoint가 읽어 앱에 넘긴다 |
| `/var/lib/app` | 서비스가 파일을 남기는 유일한 위치(`DATA_DIR`) |

### 설정이 적용되는 방식

**설정은 compose의 `environment`로 옮겨 적지 않는다.** `.env`를 통째로 마운트하고 앱
(`bin/server.rs`)이 기동할 때 읽으므로, `.env.example`에 있는 값은 전부 그 파일 하나로 적용되고
설정이 늘어도 compose는 그대로다. 값을 바꿨다면 컨테이너를 재시작하면 된다(이미지를 다시 구울
필요가 없다).

예외는 앱보다 먼저 도는 entrypoint가 쓰는 `DATA_DIR` 하나다. 이미지 ENV로 정해져 있어서
`.env`에 적어도 적용되지 않는다 — dotenvy는 이미 있는 환경변수를 덮어쓰지 않기 때문이다.
바꾸려면 compose의 `environment`로 넘기고 볼륨 경로도 같이 바꾼다. 같은 이유로 Dockerfile에
`ENV`를 늘리지 않는다(늘리면 그 값은 `.env`로 바꿀 수 없게 된다).

**호스트 `.env`의 권한은 신경 쓰지 않아도 된다**(600이어도, 소유자가 누구여도 된다). 앱은 uid
10001로 돌지만, 그 전에 root로 도는 entrypoint가 마운트된 `/app/.env`를 읽어 앱 계정 전용
사본(`/run/app/.env`)을 만들고 작업 디렉토리를 그쪽으로 옮겨 주기 때문이다. 호스트의 원본은
건드리지 않는다.

`.env`는 `.dockerignore`에도 들어 있다. 이미지 레이어에 비밀값이 박히지 않게 하기 위해서다.

### 배포 전에 확인할 것

- **`.env`가 호스트에 있는가.** 없으면 도커가 그 자리에 빈 디렉토리를 만들어 붙이고, entrypoint가
  원인을 출력하고 기동을 멈춘다.
- **`DATABASE_URL`의 호스트가 컨테이너에서 닿는가.** 개발 머신 기준의 `127.0.0.1`은 컨테이너
  안에서는 컨테이너 자신이다.
- **`CLIENT_IP_HEADER`가 앞단 프록시와 맞는가.** 레이트 리밋의 키가 되는 클라이언트 IP를 이
  헤더에서 읽는다. 프록시가 **직접 세팅하는** 헤더를 고른다(기본값 `x-forwarded-for`).
  세팅하지 않는 헤더를 고르면 클라이언트가 IP를 위조할 수 있다.
- **CORS가 필요한가.** 브라우저가 다른 오리진에서 직접 부를 때만 `CORS_ALLOWED_ORIGINS`를
  채운다. 앞단 게이트웨이가 CORS를 처리한다면 비워 둔다(양쪽에서 붙이면 브라우저가 거부한다).
- **종료 대기 시간.** 도커는 SIGTERM 후 기본 10초 만에 SIGKILL을 보낸다. 긴 요청이나 배치가
  있다면 compose의 `stop_grace_period`(주석 처리돼 있다)를 살린다.
- **이번 배포에 마이그레이션이 있다면** 구버전 코드와 공존할 수 있는가(롤백 가능성).

### 파일을 디스크에 남길 때

서비스가 쓸 수 있는 디렉토리는 `DATA_DIR` 하나다(`AppConfig::data_dir`). 로컬에서는 `./data`,
컨테이너에서는 `/var/lib/app`이며, 업로드 보관처럼 용도가 여럿이면 그 아래에 하위 디렉토리를
파서 쓴다. 이 경로를 실제로 쓰는 기능은 아직 없으므로, 기동 시점에 존재 여부나 쓰기 권한을
검사하지 않는다 — 파일을 쓰는 기능을 처음 붙일 때 그 검사를 `config.rs`에 더하면 된다.

컨테이너는 서비스를 root로 돌리지 않는다(uid 10001의 `appuser`). 그런데 호스트 디렉토리를
bind mount로 붙이면 이미지를 구울 때 맞춰둔 소유권이 마운트에 덮이므로, **소유권은 마운트가
끝난 뒤에야 맞출 수 있다.** 그래서 컨테이너는 root로 시작해 `docker-entrypoint.sh`가
`DATA_DIR`의 소유권을 정리한 뒤 스스로 `appuser`로 내려앉는다(postgres 같은 공식 이미지들과
같은 방식이다). 덕분에 **호스트에서 `mkdir`이나 `chown`을 미리 칠 일이 없다.**

알아둘 것 셋:

- **호스트 쪽 소유자도 10001로 바뀐다.** bind mount라 컨테이너가 바꾼 소유권이 호스트에 그대로
  보인다. 호스트에서 일반 계정으로 그 안에 파일을 쓰거나 지우려면 `sudo`가 필요하다. 호스트에서
  파일을 들여다볼 일이 없다면 이름 있는 볼륨(`appdata:/var/lib/app`)으로 바꾸는 편이 깔끔하다.
- **소유권 검사는 `DATA_DIR` 루트만 본다.** 파일이 수십만 개 쌓인 뒤에도 재시작할 때마다 전체를
  순회하면 기동이 느려지기 때문이다. 밖에서 root로 파일을 밀어넣어 하위 소유자가 섞였다면 한
  번은 `chown -R 10001:10001`로 맞춰준다.
- **root_squash가 걸린 NFS** 같은 곳에서는 컨테이너의 root도 chown을 못 해 기동에 실패한다.
  로컬 디스크나 이름 있는 볼륨이면 해당 없다.

### root가 아닌 상태로 시작하는 경우

compose의 `user:`로 uid를 지정하거나, **쿠버네티스**에서 `securityContext.runAsNonRoot`를 걸면
entrypoint가 root가 아니므로 위의 두 가지 정리(데이터 폴더 소유권, `.env` 사본)를 하지 않는다.
그때는 앱이 마운트된 `/app/.env`를 직접 읽고, 데이터 폴더도 그대로 쓴다. 그래서:

- `/app/.env`가 그 uid로 **읽을 수 있어야** 한다(읽지 못하면 원인을 출력하고 기동에 실패한다).
- 데이터 폴더 소유권은 띄우는 쪽이 책임진다. 쿠버네티스라면 `fsGroup`이 그 일을 해준다.
- `runAsNonRoot`는 "이미지가 root로 시작한다"는 이유로 파드를 거부하므로 Dockerfile에
  `USER appuser`를 되살린다.
- 쿠버네티스에서는 `/health`가 liveness probe, `/ready`가 readiness probe 자리다.

## 지울 것

`example`이 붙은 파일들이 예제다. 새 기능을 추가할 때 모양을 그대로 따라 쓰고, 다 쓴 뒤엔 지운다.

```
src/store/example/            store(쓰기) / views(조회) trait + Postgres 구현
src/domain/example.rs         순수 규칙 (이름 정규화)
src/usecase/example.rs        유스케이스 (트랜잭션 없는 생성 / 트랜잭션으로 이름 변경 / 목록)
src/usecase/example_job.rs    스케줄 job
src/http/routes/example.rs    HTTP 핸들러
migrations/0001_init.sql      예제 테이블
```

지울 때 함께 손볼 곳: `src/domain/mod.rs`, `src/usecase/mod.rs`, `src/http/routes/mod.rs`,
`src/store/mod.rs`의 모듈 선언, `src/http/mod.rs`의 `.merge()`, `src/store/entry.rs`의
`entries!` 목록과 `use`, 그리고 `src/usecase/jobs.rs`의 등록 항목(파일 자체는 남긴다).

`scheduler/`와 `migrations/0002_scheduled_job_runs.sql`, `src/store/job_run.rs`는 예제가 아니라
골격이다. 배치를 쓰지 않기로 했다면 셋을 함께 지우고 `AppState`의 `job_run_store`와
`server/mod.rs`의 `Scheduler` 두 줄을 걷어낸다.

그리고 **예제를 지운 뒤 `cargo sqlx prepare`를 다시 돌린다.** 캐시에는 지워진 쿼리의
검증 데이터가 그대로 남아 있어서, 갱신하지 않으면 존재하지 않는 테이블의 정보를 계속
커밋하게 된다.

## 설계 규칙

전체 구조와 그 이유는 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)에 있다. 여덟 줄로 줄이면:

1. **로직은 lib, bin은 껍데기.** bin 안의 코드는 다른 bin도 통합 테스트도 재사용할 수 없다.
2. **모듈은 "다른 프로젝트로 옮겼을 때 내용이 그대로인가"로 가른다.** 그대로면 골격, 아니면
   `domain/`(순수 규칙)과 `usecase/`(저장소를 부르는 조립).
3. **`AppState`는 `Clone` 없이 `new() -> Arc<Self>`.** 공유는 항상 Arc clone.
4. **백그라운드 진입점은 `Arc<AppState>`, 로직은 좁은 `&dyn` 포트.** 스케줄 job도 태스크도 같다.
5. **포트는 trait + `automock`.** 그래서 DB 없이 테스트가 돈다.
6. **저장소는 Store(쓰기·검증)와 Views(조회) 두 축.** 트랜잭션은 usecase가 `store.begin()` →
   `commit()`으로 쥐고, view는 커밋한 뒤에 읽는다.
7. **에러 응답은 한 형식(`{"error": ...}`).** 핸들러는 `AppError`를 반환하고, 요청은
   `http::extract`의 `Json`/`Path`로 받는다.
8. **마이그레이션은 구버전과 공존하게.** 신버전이 적용한 스키마 위에서도 구버전이 돌아야 롤백할 수 있다.

## 들어있지 않은 것

의도적으로 뺐다. 필요할 때 넣는 게 낫다고 본 것들:

- **인증** — API key / JWT / OIDC / 게이트웨이가 넣어주는 헤더 중 무엇이냐에 따라 모양이
  완전히 달라진다. axum의 `FromRequestParts`로 extractor를 만들어 "검증된 상태"를 타입으로
  표현하는 패턴을 쓰고, `http/extract.rs`에 둔다(늘어나면 폴더로).
- **아웃바운드 HTTP** — reqwest를 포트(`trait Upstream`) 뒤에 두고 어댑터로 감싼다.
  `store/`와 구조가 같다.
- **캐시 / 이벤트** — 폴더 이름은 기술이 아니라 포트 이름으로 (`redis/`가 아니라 `cache/`,
  `kafka/`가 아니라 `events/`). 단 kafka는 프로듀서가 아웃바운드, 컨슈머는 인바운드라
  한 폴더에 섞으면 안 된다 — 컨슈머는 `server/` 옆에 `consumer/`로 둔다.
- **`/metrics`, `/version`** — 운영용 엔드포인트. 노출 범위 때문에 별도 포트(9090 등)에
  올리는 경우가 많아서, 그 결정과 함께 넣는 게 낫다.
- **컨테이너 HEALTHCHECK** — 도커가 주기적으로 컨테이너 안에서 명령을 돌려 `docker ps`에
  healthy/unhealthy를 표시하는 기능. 이미지에 curl이 없어서 넣으려면 바이너리에 확인용 하위
  명령을 더해야 하고, 사람이 컨테이너 상태를 보고 전환하는 지금의 배포에서는 `/ready`를 직접
  불러 보는 것으로 충분하다.
- **종료 시 readiness 먼저 내리기** — SIGTERM을 받으면 `/ready`를 먼저 실패로 바꿔 로드밸런서가
  트래픽을 뺄 시간을 주는 기법(`server/shutdown.rs` 주석 참고). 트래픽 전환을 사람이 nginx에서
  먼저 하는 지금의 배포에서는 필요 없고, 쿠버네티스처럼 readiness를 보고 전환하는 환경에서 의미가 있다.
- **`tests/` 통합 테스트** — `server::run`이 lib에 있으므로 가능하지만 실제 Postgres가
  필요하다. 지금은 모든 테스트가 lib 안의 단위 테스트이고, DB 없이 돈다.
- **공용 테스트 헬퍼(`test_support`)** — 각 테스트가 필요한 mock을 직접 만들고 `AppState`를
  직접 조립한다. AppState의 도메인 저장소가 `store`/`views` 두 필드로 고정돼 있어서 영역이
  늘어도 조립 코드가 불어나지 않는다. 트랜잭션 mock(`MockStore` → `MockTx` → 도메인 mock)의
  연결도 테스트마다 직접 적는다 — 테스트만 읽어도 무슨 일이 일어나는지 보이게 하려는 선택이다.
