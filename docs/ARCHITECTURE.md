# 구조와 그 이유

## 모듈 트리

```
# 폴더를 가른 기준: "다른 프로젝트로 옮겼을 때 내용이 그대로인가."
src/
  lib.rs          모듈 선언. `mod domain` + `pub use domain::*`로 도메인 모듈을 루트에 재노출해,
                  폴더를 나눠도 소비자 경로는 `crate::example`처럼 짧게 유지됨
  config.rs       환경변수 기반 설정(AppConfig) — 서버/CLI 어느 진입점이든 씀
  error.rs        단일 에러 타입(AppError) + axum IntoResponse 매핑
  state.rs        핸들러와 백그라운드 태스크가 공유하는 AppState. Clone 없이
                  `AppState::new`가 `Arc<Self>`를 반환
  sync_util.rs    poison된 Mutex도 복구해서 잠그는 `lock()` 헬퍼
  server/         "이 프로세스가 HTTP 서버라서" 필요한 것들
    mod.rs        부팅 절차 `run(cfg)`
    shutdown.rs   SIGINT/SIGTERM을 기다리는 Future
  store/          저장소 포트(trait)와 그 Postgres 어댑터. `pg_pool()`도 여기
    mod.rs        pg_pool, PgStore, re-export
    health.rs     readiness용 ping 포트
    example.rs    [예제] 포트 + 구현
  routes/         HTTP 핸들러(어댑터). 요청 파싱 → domain 호출 → 직렬화만
    mod.rs        라우터 조립 + TraceLayer
    health.rs     /health(liveness) / /ready(readiness)
    example.rs    [예제] 핸들러
  domain/         이 서비스만의 로직. HTTP도 SQL도 모른다
    example.rs      [예제] 도메인 함수
    example_task.rs [예제] 백그라운드 태스크
  bin/            진입점들. "밖에서 앱을 어떻게 호출하는가"만 안다
    server.rs     HTTP 서버 (`cargo run`)
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

"다른 프로젝트로 옮겼을 때 내용이 그대로인가." 그대로인 것은 루트/`server/`/`store/mod.rs`에,
이 서비스에서만 의미가 있는 것은 `domain/`에 둔다. 기능이 늘어날 때 커지는 폴더가 하나로
모이므로, 어느 파일이 골격이고 어느 파일이 이 서비스 것인지 트리만 봐도 구분된다.

경로가 길어지는 건 `lib.rs`의 `mod domain` + `pub use domain::*`로 막는다. 비공개 `mod`와
재노출을 조합하는 이유는 공개 경로를 하나만 남기기 위해서다 — `pub mod domain`으로 하면
`crate::domain::example`과 `crate::example`이 동시에 공개돼 사용법이 갈린다.

### AppState

Rust에는 런타임 리플렉션이 없어서 Spring식 컴포넌트 스캐닝이 원리적으로 불가능하다
(proc macro는 자기가 붙은 토큰 스트림만 본다). DI 크레이트를 쓰더라도 "어떤 구현을 어디에
꽂을지"를 결국 손으로 적어야 하므로, 생태계의 결론은 **명시적으로 조립하는 컨테이너 구조체**다.

여기서 두 가지를 고정했다:

- **`Clone`을 달지 않고 `new()`가 `Arc<Self>`를 반환한다.** 그러면 "구조체를 clone할지 Arc를
  clone할지"라는 선택지가 사라진다. 필드가 전부 `Arc`라 struct clone도 싸지만, 설정 문자열
  같은 맨 필드가 하나라도 늘면 그때부터 비용이 생기고 — 무엇보다 두 방식이 공존하는 상태가
  헷갈린다.
- **필드는 전부 포트(`Arc<dyn ...>`)다.** 프로덕션에서는 여러 필드가 같은 어댑터 하나를
  가리키고, 테스트에서는 필드마다 mock을 꽂는다.

### 진입점은 state, 로직은 좁은 포트

백그라운드 태스크에서 특히 중요하다. `run_periodic(state: Arc<AppState>, interval)`은
`Arc<AppState>`를 통째로 받고, 실제 로직인 `sweep(store: &dyn ExampleStore)`는 필요한 포트만
받는다. 양쪽 이득을 다 가지려는 것이다:

- 진입점이 state를 받으므로, 태스크가 나중에 포트를 더 쓰게 되어도 `server::run`의 호출부는
  손대지 않는다. 태스크를 추가할 때 필요한 포트를 골라 각각 clone하는 작업도 없다.
- 로직은 좁은 시그니처라 테스트가 mock 하나로 끝나고, 시그니처 자체가 "이 함수는 이것만
  건드린다"는 문서가 된다.

껍데기가 로직까지 품기 시작하면 이 이득이 사라진다 — 그게 이 규칙에서 지켜야 할 선이다.

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

공용 빌더(`test_support::StateBuilder` 같은 것)는 두지 않았다. 포트가 두세 개일 때는 테스트가
자기 조립을 직접 들고 있는 편이 위에서 아래로 읽히기 때문이다. 포트가 5~6개를 넘어가면
"관심 없는 필드 채우기"가 테스트마다 반복되고 포트 추가가 모든 테스트를 깨뜨리므로, 그
시점에 `#[cfg(test)] pub mod test_support`로 빌더를 뽑아내면 된다.

저장소 impl 블록은 의도적으로 단위 테스트하지 않는다 — SQL이 맞는지는 실제 DB에 붙여봐야만
알 수 있는 종류의 문제고, mock으로 검증하면 "내가 쓴 SQL을 내가 확인"하는 동어반복이 된다.

### 라우터 조립

새 라우트 그룹을 추가하려면 `routes/mod.rs`에 `.merge()` 한 줄을 넣어야 한다. 중앙 파일을
건드리는 게 의도된 트레이드오프다. Rust에는 애노테이션 스캐닝이 없고, 링크 타임 등록
(linkme/inventory)은 등록 순서 제어가 안 되며 모듈을 아무도 `use`하지 않으면 링커가 떼버려도
컴파일러가 알려주지 않는다. 한 줄 충돌은 3초에 해결되지만, 등록이 안 된 라우트를 런타임에
발견하는 건 그렇지 않다.

### 운영

- **graceful shutdown**: SIGINT/SIGTERM에 새 연결을 멈추고 진행 중 요청을 끝낸다.
  한 걸음 더 나가려면 readiness를 먼저 false로 바꿔 LB가 빠질 시간을 준다
  (`server/shutdown.rs` 주석 참고).
- **타임아웃 두 종류**: `acquire_timeout`은 풀에서 커넥션을 기다리는 시간,
  Postgres `statement_timeout`은 이미 잡은 커넥션에서 쿼리가 도는 시간. 전자만 두면 커넥션은
  받았는데 쿼리가 안 끝나는 경우를 놓친다.
- **liveness에 의존성을 넣지 않는다**: DB 장애는 재시작으로 회복되지 않으므로, liveness에서
  DB를 찌르면 장애 때 crash loop가 난다.
- **Mutex poison 복구**: 요청 하나의 패닉이 공유 상태를 프로세스 수명 내내 못 쓰게 만들지
  않도록 `sync_util::lock()`을 거친다.
