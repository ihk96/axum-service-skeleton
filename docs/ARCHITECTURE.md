# 구조와 그 이유

## 모듈 트리

```
# 폴더를 가른 기준: "다른 프로젝트로 옮겼을 때 내용이 그대로인가."
src/
  lib.rs          모듈 선언
  config.rs       환경변수 기반 설정(AppConfig) — 서버/CLI 어느 진입점이든 씀
  error.rs        단일 에러 타입(AppError) + axum IntoResponse 매핑
  state.rs        핸들러와 백그라운드 태스크가 공유하는 AppState(store / views 두 축).
                  Clone 없이 `AppState::new`가 `Arc<Self>`를 반환
  sync_util.rs    poison된 Mutex도 복구해서 잠그는 `lock()` 헬퍼
  server/         "이 프로세스가 HTTP 서버라서" 필요한 것들
    mod.rs        부팅 절차 `run(cfg)`
    shutdown.rs   SIGINT/SIGTERM을 기다리는 Future
  scheduler/      크론식 배치의 실행기. 무엇을 언제 돌릴지는 모른다(usecase/jobs.rs가 안다)
    registry.rs   등록 창구 `JobRegistry` — 바깥이 보는 유일한 면
    runner.rs     실행 기반: 크론식 파싱 + job별 루프 + 분산 락
    prune.rs      실행 기록을 청소하는 job. 스케줄러가 자기를 스스로 등록한다
  store/          저장소. `pg_pool()`도 여기
    mod.rs        pg_pool, PgStore(풀 모드 / 트랜잭션 모드), conn()
    entry.rs      진입점 `Store` / `Tx` / `Views`와 그 목록(`entries!`)
    entry/macros.rs  목록으로부터 trait과 Postgres 구현을 만드는 매크로
    health.rs     readiness용 ping 포트 (인프라)
    job_run.rs    스케줄러의 실행 선점/기록 포트 (인프라)
    example/      [예제] 도메인 영역 하나
      store.rs      제어(쓰기) — ExampleStore
      views.rs      조회 전용 — ExampleViews
  http/           HTTP 어댑터
    mod.rs        라우터 조립 + 레이어 스택(request id, trace, panic, CORS, timeout)
                  + 업무 라우트에만 붙는 rate limit
    extract.rs    추출기(Json/Path) — 파싱 실패도 AppError 형식으로
    middleware/   레이어 설정. rate_limit.rs(IP별 버킷 + 정리 태스크 + JSON 429)
    routes/       핸들러. 요청 파싱 → usecase 호출 → 직렬화만
      health.rs     /health(liveness) / /ready(readiness)
      example.rs    [예제] 핸들러
  domain/         순수 규칙. 저장소도 HTTP도 모른다
    example.rs      [예제] 이름 정규화/검증
  usecase/        요청 하나를 조립한다. domain 규칙 + store/views + 트랜잭션
    example.rs      [예제] 생성(트랜잭션 없음) / 이름 변경(트랜잭션) / 목록
    example_job.rs  [예제] 스케줄 job
    jobs.rs         스케줄 job 등록부 — 이 서비스가 무엇을 언제 돌리는지의 전부
  bin/            진입점들. "밖에서 앱을 어떻게 호출하는가"만 안다
    server.rs     HTTP 서버 (`cargo run`). .env 로딩, 로거, 설정 읽기까지만 하고 server::run에 넘김
migrations/       sqlx 마이그레이션
```

## 왜 이렇게 생겼는지

### lib + thin bin

`src/main.rs`가 아니라 `src/bin/server.rs`이고, 부팅 절차는 `server::run`으로 lib에 있다.
bin 크레이트에 있는 코드는 (1) 같은 패키지의 다른 bin이 재사용할 수 없고 (2) `tests/`의
통합 테스트가 import할 수 없기 때문이다. 실제로 이 규칙을 어기면 곧 드러난다 — CLI를 하나
추가하는 순간 풀 생성 코드가 복사되고, 그 복사본에서 타임아웃 설정이 빠진다.

그래서 `store::pg_pool`도 lib에 있다. 진입점이 몇 개든 커넥션 풀 설정은 한 곳에서만 정한다.

### 모듈 분리 기준

"다른 프로젝트로 옮겼을 때 내용이 그대로인가." 그대로인 것은 루트/`server/`/`store/mod.rs`/
`http/mod.rs`에, 이 서비스에서만 의미가 있는 것은 `domain/`과 `usecase/`(그리고 그에 딸린
`store/<영역>/`, `http/routes/<영역>.rs`)에 둔다.

서비스 코드를 두 층으로 나눈 기준은 **저장소를 부르는가**다.

- `domain/`은 순수 함수만 둔다 — 값 검증·정규화, 상태 전이 규칙, 계산. 입력을 받아 판단하고
  결과를 돌려줄 뿐이라 mock조차 필요 없이 테스트된다.
- `usecase/`는 요청 하나를 처음부터 끝까지 조립한다 — domain 규칙을 적용하고, store로 쓰고,
  필요하면 트랜잭션으로 묶고, views로 응답 모양을 읽는다.

두 층에 같은 이름의 모듈(`example`)이 생기므로 경로에 층 이름을 남긴다(`crate::domain::example`,
`crate::usecase::example`). 어느 층을 부르는지가 import만 봐도 드러난다.

### 스케줄러

`scheduler/`(기반) + `usecase/jobs.rs`(등록)로 "매일 03:00 KST" 같은 **시각 기반** 배치를 돌린다.
인스턴스가 몇 개든 발화당 한 번만 돈다.

**여기로 오지 않는 것도 있다.** 인스턴스마다 각자 돌아야 하는 상시 루프(kafka 컨슈머, 로컬
캐시 갱신)는 스케줄러가 아니라 `server::run`에서 직접 spawn하는 태스크로 둔다 — 그쪽은 발화
시각이라는 개념이 없고, 중복 실행을 막는 것이 오히려 잘못이다. 지금 그런 태스크는 없지만
자리는 `server::run`의 `tasks`에 남아 있다.

스케줄러 쪽에서 결정한 것들:

- **크론 크레이트가 아니라 크론식 파서만 쓴다.** `croner`에서 빌려 오는 건 "다음 발화 시각"
  계산뿐이고, 루프와 종료는 서버와 같은 방식(`CancellationToken` + `select!`)으로 직접 짠다.
  스케줄러 크레이트를 통째로 쓰면 자기 실행기와 종료 방식이 딸려 와서 이 서비스의 graceful
  shutdown과 겹친다.
- **중복 실행은 DB가 막는다.** 발화 시각은 크론식에서 나오므로 모든 인스턴스가 같은 값을
  계산한다. 그 `(job 이름, 발화 시각)`을 기본키로 하는 `scheduled_job_runs`에 먼저 INSERT한
  인스턴스만 실행하고 나머지는 충돌을 받고 넘어간다. 리더 선출도 락 서비스도 없다.
  advisory lock은 job이 짧으면 시계 편차만큼 중복 창이 생겨서 택하지 않았다.
- **job마다 태스크 하나.** 한 루프에서 다 돌리면 오래 걸리는 job이 다른 job의 발화를 민다.
- **job 본체는 `tokio::spawn`으로 한 겹 감싼다.** 패닉이 `JoinError`로 잡히므로 job 하나의
  버그가 그 job을 프로세스 수명 내내 멈추지 않는다.
- **놓친 발화는 따라잡지 않는다.** 다음 발화는 항상 "지금" 기준으로 계산한다. 프로세스가
  꺼져 있던 동안의 배치가 기동 직후 한꺼번에 쏟아지는 쪽이 대개 더 위험하다.
- **등록과 기반을 파일로 갈랐다.** 등록 창구(`registry.rs`)를 통과시키므로 `usecase/jobs.rs`가
  아는 이름은 `JobRegistry`와 `Tz` 둘뿐이고, 실행기 타입은 하나도 새어나가지 않는다.
  `Scheduler::build`가 등록부를 **값이 아니라 함수로** 받는 것도 같은 이유다 — 스케줄러가
  자기 유지보수 job(prune)을 먼저 등록한 뒤 창구를 넘기므로, 그 job이 등록부에 보이지 않는다.

job을 추가할 때 손대는 파일은 `usecase/` 아래 둘(job 함수, `jobs.rs` 등록 한 줄)뿐이다.
`server/mod.rs`도 `scheduler/`도 건드리지 않는다.

### AppState

Rust에는 런타임 리플렉션이 없어서 Spring식 컴포넌트 스캐닝이 원리적으로 불가능하다
(proc macro는 자기가 붙은 토큰 스트림만 본다). DI 크레이트를 쓰더라도 "어떤 구현을 어디에
꽂을지"를 결국 손으로 적어야 하므로, 생태계의 결론은 **명시적으로 조립하는 컨테이너 구조체**다.

여기서 세 가지를 고정했다:

- **`Clone`을 달지 않고 `new()`가 `Arc<Self>`를 반환한다.** 그러면 "구조체를 clone할지 Arc를
  clone할지"라는 선택지가 사라진다. 필드가 전부 `Arc`라 struct clone도 싸지만, 설정 문자열
  같은 맨 필드가 하나라도 늘면 그때부터 비용이 생기고 — 무엇보다 두 방식이 공존하는 상태가
  헷갈린다.
- **필드는 전부 포트(`Arc<dyn ...>`)다.** 프로덕션에서는 여러 필드가 같은 어댑터 하나를
  가리키고, 테스트에서는 필드마다 mock을 꽂는다.
- **도메인 저장소는 `store`와 `views` 두 필드뿐이다.** 도메인 store/view가 늘어나도 AppState는
  그대로이고, 그래서 그걸 조립하는 테스트도 깨지지 않는다. `health_store`/`job_run_store`는
  도메인이 아니라 서버·스케줄러용 인프라 포트라 따로 둔다.

### 진입점은 state, 로직은 좁은 포트

백그라운드에서 도는 코드에서 특히 중요하다. 스케줄 job의 진입점
`example_job::run(state: Arc<AppState>)`은 state를 통째로 받고, 실제 로직인
`summarize(views: &dyn ExampleViews)`는 필요한 포트만 받는다. 양쪽 이득을 다 가지려는 것이다:

- 진입점이 state를 받으므로, job이 나중에 포트를 더 쓰게 되어도 등록 줄(`usecase/jobs.rs`)이나
  `server::run`의 호출부는 손대지 않는다. 필요한 포트를 골라 각각 clone하는 작업도 없다.
- 로직은 좁은 시그니처라 테스트가 mock 하나로 끝나고, 시그니처 자체가 "이 함수는 이것만
  건드린다"는 문서가 된다.

껍데기가 로직까지 품기 시작하면 이 이득이 사라진다 — 그게 이 규칙에서 지켜야 할 선이다.

### 저장소: Store와 Views, 그리고 트랜잭션

저장소는 두 축이다.

- **Store**(`XxxStore`) — 제어(쓰기)와 그에 딸린 검증 조회. 트랜잭션에 참여할 수 있다.
- **Views**(`XxxViews`) — 응답용 조회 전용. 응답 모양(`XxxView`, Serialize)을 돌려주고,
  트랜잭션에 참여하지 않고 항상 풀을 쓴다.

바깥은 이것들을 `store/entry.rs`의 진입점으로 꺼낸다.

| 진입점 | 메서드 | 쓰임 |
|---|---|---|
| `Store` | `examples()` 등 + `begin()` | 트랜잭션 없이 바로 쓰거나, 필요하면 트랜잭션을 연다 |
| `Tx` | 같은 accessor + `commit()` | 여기서 꺼낸 store들은 모두 같은 트랜잭션. commit 없이 drop되면 롤백 |
| `Views` | `examples()` 등 | 항상 풀 |

```rust
let tx = store.begin().await?;
let examples = tx.examples();
examples.get_example_for_update(id).await?;   // SELECT ... FOR UPDATE
examples.update_example_name(id, &name).await?;
tx.commit().await?;
let view = views.get_example(id).await?;       // view는 커밋한 뒤에 읽는다
```

결정한 것들:

- **`Store`와 `Tx`를 다른 타입으로 뒀다.** `commit()`은 `Tx`에만, `begin()`은 `Store`에만 있어서
  "트랜잭션을 열지 않고 커밋"하거나 "트랜잭션 안에서 또 여는" 실수가 컴파일 단계에서 막힌다.
- **트랜잭션은 store 구현과 분리돼 있다.** 도메인 store trait에는 begin/commit이 없다. 같은
  `PgStore`가 `tx`가 None이면 풀로, Some이면 그 트랜잭션으로 쿼리를 보내므로(`conn()`),
  트랜잭션용 trait이나 SQL을 따로 만들지 않는다.
- **accessor는 참조가 아니라 `Box`를 돌려준다.** 참조를 돌려주는 메서드는 mockall로 mock하기
  번거롭다. `Box`면 `MockStore`/`MockTx`/`MockViews`가 그대로 생성된다.
- **accessor 목록은 한 곳에만 적는다.** `Store`와 `Tx`의 accessor가 같아야 하고 Postgres 구현도
  셋이라, `entries!` 매크로가 목록 하나로부터 전부 만든다. 손으로 맞추면 언젠가 갈라진다.
- **제네릭(연관 타입 `type Tx`)이나 downcast는 쓰지 않았다.** 연관 타입은 usecase와 AppState까지
  제네릭을 번지게 하고, downcast는 trait object에서 구체 트랜잭션을 런타임에 꺼내야 한다.

store 구현 규칙(`store/mod.rs`에도 적어뒀다):

1. 쿼리는 `&mut *self.conn().await?`로 보낸다. views 구현만 `&self.pool`을 쓴다.
2. 메서드 안에서 여러 쿼리를 원자적으로 묶으려면 `self.pool.begin()`이 아니라
   `sqlx::Connection::begin(&mut *conn)`. 트랜잭션 모드에서는 SAVEPOINT가 된다.
3. `conn`을 쥔 채로 `self`의 다른 메서드를 부르지 않는다(트랜잭션 모드에서는 Mutex라 교착).

Postgres라서 주의할 것:

- **트랜잭션 안에서 쿼리 하나가 실패하면 그 트랜잭션은 이후 쿼리를 전부 거부한다.** unique
  위반을 잡아 재시도하는 코드를 트랜잭션 안에서 돌리려면 그 쿼리를 SAVEPOINT(규칙 2)로 감싸거나
  `ON CONFLICT`로 에러가 나지 않게 쓴다.
- **동시 쓰기가 실제로 일어난다.** 인스턴스가 여럿이면(blue/green 전환 중 포함) "조회해서 없으면
  insert" 같은 흐름은 경쟁한다. unique 제약으로 막는 게 가장 확실하고, 읽고 고치는 흐름은
  `SELECT ... FOR UPDATE`로 행을 잠근다(`usecase::example::rename_example` 참고).
- **트랜잭션은 커넥션 하나를 붙잡는다.** 검증·해싱·외부 API 호출처럼 DB가 필요 없는 일은
  `begin()` 전에 끝낸다.

새 도메인 영역을 추가할 때:

1. `store/<영역>/store.rs`, `views.rs`에 trait(`automock`) + Postgres 구현, `mod.rs`로 묶기
2. `store/mod.rs`에 `pub mod <영역>;`
3. `store/entry.rs`의 `entries!` 목록에 한 줄씩, 그리고 그 trait을 `use`에 추가
4. 마이그레이션 → `cargo sqlx prepare -- --all-targets`

AppState도, 기존 테스트도 건드리지 않는다.

### 포트 trait의 입도

**테이블이 아니라 쓰임새 기준.** 소비자가 한 번의 작업에 필요로 하는 단위로 메서드를 만든다.
쿼리 단위로 잘게 쪼개면 재사용은 좋아 보이지만, 소비자 쪽 테스트가 매번 여러 스텁을 세워야
하고 조합 로직이 소비자에게 흩어진다. 포트와 그 Postgres 구현을 한 파일에 두는 것도 같은
이유다 — 모양을 정할 때 둘을 같이 봐야 한다.

`ports/`와 `adapters/`를 폴더로 가르는 clean architecture 스타일은 쓰지 않았다. 포트당 구현이
하나인 상황에서 폴더를 가르면 변경마다 두 곳을 열게 되고 얻는 것이 없다.

### 테스트 전략

포트가 trait이고 `#[cfg_attr(test, mockall::automock)]`이 붙어 있으므로, 모든 테스트가 DB도
소켓도 없이 돈다. 각 테스트는 필요한 mock을 직접 만들고 `AppState`를 직접 조립하는데
(그래서 필드가 `pub`이다), 이때 **관심 없는 필드는 expectation 없는 빈 mock으로** 채우는 게
관례다. 빈 mock은 호출되는 순간 패닉하므로 이건 편의가 아니라 **"이 요청 경로는 그 포트를
건드리지 않는다"는 자동 검증**이다 — `/health`가 저장소를 보지 않는다는 규칙이 이렇게
고정된다. 편하다고 permissive한 mock(`.returning(|| Ok(()))`)을 넣으면 그 성질이 사라진다.

usecase 테스트는 대부분 도메인 store/view mock 하나씩이면 끝난다(`MockExampleStore`,
`MockExampleViews`). 트랜잭션을 쓰는 usecase는 mock 세 겹을 직접 잇는다.

```rust
let mut examples = MockExampleStore::new();        // 1. 도메인 store
examples.expect_update_example_name()...;

let mut tx = MockTx::new();                         // 2. 트랜잭션: examples를 내주고, commit 여부를 정함
tx.expect_examples().return_once(move || Box::new(examples));
tx.expect_commit().times(1).returning(|| Ok(()));   //    실패 케이스면 .never()
let mut store = MockStore::new();                   // 3. begin()하면 tx를 내줌
store.expect_begin().return_once(move || Ok(Box::new(tx)));
```

commit을 빠뜨리거나 실패 경로에서 커밋하는 버그가 `times(1)`/`never()`로 잡힌다. 이 연결을
감싸는 공용 헬퍼는 일부러 두지 않았다 — 테스트만 읽어도 무슨 일이 일어나는지 보이는 쪽을 택했다.

라우트 테스트는 `http::build()`가 아니라 그 라우트 그룹만 세운다(`routes().with_state(...)`).
확인하려는 건 라우트 등록·역직렬화·에러 매핑이고, 레이트 리밋 같은 공통 레이어는 커넥션
정보(peer IP)가 있어야 세울 수 있어서 `oneshot` 요청과 맞지 않는다.

AppState가 `store`/`views` 두 축으로 고정돼 있으므로 도메인 store/view가 늘어나도 기존 테스트의
조립 코드는 그대로다. 그래서 공용 빌더(`test_support::StateBuilder` 같은 것)는 두지 않았다.

저장소 impl 블록은 의도적으로 단위 테스트하지 않는다 — SQL이 맞는지는 실제 DB에 붙여봐야만
알 수 있는 종류의 문제고, mock으로 검증하면 "내가 쓴 SQL을 내가 확인"하는 동어반복이 된다.

### 라우터 조립

새 라우트 그룹을 추가하려면 `http/routes/mod.rs`에 모듈 선언을, `http/mod.rs`의 `build()`에
`.merge()` 한 줄을 넣어야 한다. 중앙 파일을
건드리는 게 의도된 트레이드오프다. Rust에는 애노테이션 스캐닝이 없고, 링크 타임 등록
(linkme/inventory)은 등록 순서 제어가 안 되며 모듈을 아무도 `use`하지 않으면 링커가 떼버려도
컴파일러가 알려주지 않는다. 한 줄 충돌은 3초에 해결되지만, 등록이 안 된 라우트를 런타임에
발견하는 건 그렇지 않다.

`build()`의 공통 레이어 스택은 바깥부터 request id → trace → panic → CORS → timeout → request id
전파 순이다(`ServiceBuilder`는 먼저 쓴 레이어가 바깥이라 코드 순서가 곧 통과 순서다). 레이트
리밋은 이 스택이 아니라 업무 라우트 그룹에만 붙으므로, 이 스택 안쪽·핸들러 바로 앞에서 걸린다.
그 밖에:

- **레이트 리밋은 업무 라우트에만 건다.** `/health`, `/ready`까지 묶으면 감시 트래픽이 한도를
  갉아먹고, 부하가 몰린 순간 헬스체크가 429를 받아 "죽었다"고 판정될 수 있다. 버킷은 인스턴스
  메모리에 있으므로 인스턴스가 N개면 실제 허용량도 최대 N배다 — 정확한 전역 한도가 필요하면
  앞단에서 건다. 클라이언트 IP는 `CLIENT_IP_HEADER`로 고른 헤더 **하나**에서만 읽는다
  (x-forwarded-for는 맨 오른쪽 값). 프록시는 자기가 세팅하지 않은 헤더를 클라이언트가 보낸 그대로
  통과시키므로, 여러 헤더를 순서대로 믿는 방식(크레이트의 `SmartIpKeyExtractor`)은 위조 경로가
  된다. 헤더가 없는 요청은 peer IP로 떨어진다. 오래된 버킷을 비우는 정리
  태스크는 `server::run`이 다른 백그라운드 태스크와 같은 종료 토큰으로 돌린다.
- **CORS는 `CORS_ALLOWED_ORIGINS`가 비어 있으면 레이어 자체를 붙이지 않는다.** 서버 간 호출이나
  같은 오리진의 프록시 뒤라면 그게 맞다. 앞단(게이트웨이)이 CORS를 처리한다면 여기서는 끈다 —
  양쪽에서 헤더를 붙이면 브라우저가 거부한다. `permissive()`나 와일드카드는 일부러 지원하지 않는다.
- **설정이 잘못되면 기동이 실패한다.** CORS 오리진 형식, 레이트 리밋 0 같은 값은 `build()`와
  `rate_limit::config()`가 `?`로 막는다.
- **에러 응답은 형식이 하나다(`{"error": "..."}`).** 핸들러 에러는 `AppError`가, 요청 파싱 실패는
  `http/extract.rs`의 `Json`/`Path`가(axum 기본 추출기는 평문을 낸다), 429는 레이트 리밋 레이어의
  에러 핸들러가 같은 형식으로 만든다. 핸들러는 axum의 `Json`/`Path` 대신 `http::extract`의 것을 쓴다.
  5xx(`Database`, `Internal`)는 응답에 상세를 싣지 않고 `"internal server error"`만 보낸다 — sqlx
  에러 원문에는 제약·컬럼 이름이 담겨 스키마가 새어나가므로, 상세는 로그에만 남긴다(`error.rs`).
- **요청 타임아웃은 503이다.** 408은 "클라이언트가 요청을 늦게 보냈다"는 뜻이라 서버 처리 지연과
  맞지 않고, 408을 보고 자동 재시도하는 클라이언트가 있어 느린 서버에 부하를 더 얹는다.
- **`into_make_service_with_connect_info`로 서빙한다.** IP 추출기가 헤더가 없을 때 peer 주소로
  떨어지는데, 이게 없으면 폴백이 없어 헤더 없는 요청이 전부 500이 된다.

### 설정

설정은 `AppConfig::from_env()`가 기동 시점에 환경변수를 **한 번** 읽어 구조체로 만들고, 이후로는
그 값을 전달해서 쓴다. 코드 곳곳에서 `env::var`를 부르지 않는다.

- **값이 없으면 기본값, 있는데 해석할 수 없으면 기동 실패.** `DB_MAX_CONNECTIONS=1O`(알파벳 O)
  같은 오타가 조용히 기본값으로 넘어가면, 설정을 바꿨다고 믿는 사람과 실제로 도는 값이 어긋난다.
  CORS 오리진 형식, 레이트 리밋 0, 스케줄러 보관 기간 같은 값도 같은 규칙으로 기동 시점에 걸러낸다.
- **값의 출처는 작업 디렉토리의 `.env` 하나다.** `bin/server.rs`가 `dotenvy::from_path(".env")`로
  읽는다. `dotenvy::dotenv()`를 쓰지 않은 이유는 그쪽이 상위 디렉토리까지 `.env` *파일*을 찾아
  올라가서, 엉뚱한 `.env`를 읽거나 "`.env` 자리에 디렉토리가 있다"를 "파일 없음"으로 넘겨 버리기
  때문이다. 파일이 **없는** 것은 괜찮지만(값을 직접 주입하는 환경), **있는데 못 읽는** 것은 기동 실패다.
- **우선순위는 "프로세스에 이미 있는 환경변수 > `.env` > 기본값"이다**(dotenvy는 덮어쓰지 않는다).
  그래서 이미지에 `ENV`로 박은 값은 `.env`로 바꿀 수 없다. Dockerfile에 `ENV`를 두지 않는 이유고,
  `RUST_LOG`의 기본값(`info`)도 이미지가 아니라 코드에 있다. 예외는 entrypoint가 앱보다 먼저 써야
  하는 `DATA_DIR` 하나다.
- **컨테이너에서는** compose가 호스트의 `.env`를 `/app/.env`로 마운트하고, root로 시작한 entrypoint가
  그걸 앱 계정 전용 사본(`/run/app/.env`)으로 복사한 뒤 작업 디렉토리를 옮겨 준다. 그래서 호스트
  `.env`의 권한과 상관없이 동작하고, 앱 코드는 로컬과 같은 규칙("작업 디렉토리의 `.env`")으로 돈다.
  compose의 `environment`에 변수를 옮겨 적지 않으므로, 설정이 늘어도 compose는 그대로다.

설정 항목의 전체 목록과 설명은 `.env.example`에 있다. 항목을 추가·변경하면 `config.rs`와 함께
`.env.example`도 고친다.

### 마이그레이션 규칙

배포는 blue/green이라 **구버전과 신버전이 같은 DB에 동시에 붙는 시간**이 있고, 문제가 생기면
구버전으로 되돌린다. 그래서 마이그레이션은 "신버전이 적용한 스키마 위에서 구버전 코드도 그대로
돈다"를 지켜야 한다.

- **추가만 한 번에 한다.** 새 테이블, nullable 컬럼, 기본값이 있는 컬럼, 인덱스(`create index
  concurrently`는 트랜잭션 밖에서만 되므로 별도 마이그레이션으로).
- **삭제·이름 변경·타입 변경은 여러 배포로 나눈다.** 예를 들어 컬럼 이름을 바꾸려면
  (1) 새 컬럼 추가 + 양쪽에 쓰기 → (2) 읽기를 새 컬럼으로 → (3) 옛 컬럼을 안 쓰는 버전이 완전히
  자리 잡은 뒤에 삭제. 한 번에 바꾸면 그 사이에 떠 있는 구버전이 깨지고, 롤백도 깨진다.
- **NOT NULL 추가는 기본값과 함께.** 구버전은 그 컬럼을 모르고 insert하므로, 기본값이 없으면
  구버전의 insert가 실패한다.
- **이미 적용된 마이그레이션 파일은 고치지 않는다.** 체크섬이 달라져 모든 버전이 기동에 실패한다.
  고칠 게 있으면 새 마이그레이션으로.

코드 쪽에서는 `server::run`이 `set_ignore_missing(true)`로 마이그레이터를 돌린다. 신버전이 적용한
마이그레이션을 구버전 바이너리가 모르는 상태(= 롤백 직후)에서도 기동을 거부하지 않게 하는
설정이고, 위 규칙은 그렇게 뜬 구버전이 실제로 동작하게 하는 쪽이다.

### 운영

- **graceful shutdown**: SIGINT/SIGTERM에 새 연결을 멈추고 진행 중 요청을 끝낸 뒤, 백그라운드
  태스크(스케줄러, 레이트 리밋 정리)를 `SHUTDOWN_TIMEOUT_SECS`까지 기다린다. 도커의 기본 종료
  대기(10초)가 이보다 짧으면 SIGKILL로 잘리므로 compose의 `stop_grace_period`로 늘린다.
  readiness를 먼저 내려 로드밸런서가 빠질 시간을 주는 기법은 넣지 않았다 — 트래픽 전환을 사람이
  nginx에서 먼저 하는 지금의 배포에서는 필요 없다(`server/shutdown.rs` 주석 참고).
- **타임아웃 두 종류**: `acquire_timeout`은 풀에서 커넥션을 기다리는 시간,
  Postgres `statement_timeout`은 이미 잡은 커넥션에서 쿼리가 도는 시간. 전자만 두면 커넥션은
  받았는데 쿼리가 안 끝나는 경우를 놓친다.
- **liveness에 의존성을 넣지 않는다**: DB 장애는 재시작으로 회복되지 않으므로, liveness에서
  DB를 찌르면 장애 때 crash loop가 난다.
- **Mutex poison 복구**: 요청 하나의 패닉이 공유 상태를 프로세스 수명 내내 못 쓰게 만들지
  않도록 `sync_util::lock()`을 거친다.
