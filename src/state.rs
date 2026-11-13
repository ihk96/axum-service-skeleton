use std::sync::Arc;

use crate::store::{HealthStore, JobRunStore, Store, Views};

// [아키텍처] 요청 핸들러와 백그라운드 태스크가 함께 쓰는 공유 자원을 한데 묶은 구조체.
// 별도 DI 프레임워크 없이 이것 하나를 `Arc`로 나눠 갖는 게 Rust 웹 서비스의 관용구다
// (Rust에는 런타임 리플렉션이 없어서 Spring식 컴포넌트 스캐닝이 원리적으로 불가능하다 —
// 그래서 "명시적으로 조립하는 컨테이너"가 생태계의 결론이다).
//
// 필드가 전부 포트(`Arc<dyn ...>`)인 게 핵심이다. 프로덕션에서는 여러 필드가 같은
// `Arc<PgStore>` 하나를 가리키지만, 테스트에서는 각 필드에 그 테스트가 필요로 하는
// mock만 꽂으면 된다 — DB도 소켓도 없이.
//
// 도메인 저장소는 두 축으로만 들고 있다 — 제어(쓰기·검증)는 `store`, 조회 전용은 `views`
// (store/entry.rs). 도메인 store/view가 늘어나도 이 구조체는 그대로다. health/job_run은
// 도메인이 아니라 서버·스케줄러용 인프라 포트라 따로 둔다.
//
// 테스트는 `new()`를 쓰지 않고 이 구조체를 직접 조립한다(필드가 `pub`인 이유). `new()`는
// 모든 포트를 구현한 어댑터 하나를 받게 되어 있어 프로덕션 조립에는 딱 맞지만, 필드마다
// 다른 mock을 꽂아야 하는 테스트에는 맞지 않기 때문이다. 그때 **관심 없는 필드는
// expectation 없는 빈 mock으로** 채우는 게 관례다 — 빈 mock은 호출되는 순간 패닉하므로,
// 그건 곧 "이 경로는 그 포트를 건드리지 않는다"는 검증이 된다.
//
// 인메모리 상태를 갖는 컴포넌트(예: 카운터, 캐시)가 필요해지면 `Arc<Foo>`로 여기 추가하고,
// 그 안의 `Mutex`는 `.lock()`을 직접 부르지 말고 `sync_util::lock()`을 거치게 한다.
//
// [아키텍처] 이 구조체에는 의도적으로 `Clone`을 달지 않았다. 공유는 항상 `Arc<AppState>`를
// clone하는 방식으로만 하며, 유일한 생성자인 `new()`가 애초에 `Arc<Self>`를 반환해서
// "벗겨진 AppState"를 손에 쥘 일이 없게 만든다 — "구조체를 clone할지 Arc를 clone할지"라는
// 선택지 자체가 사라진다.
pub struct AppState {
    /// 제어(쓰기)와 검증 조회. 트랜잭션이 필요하면 `store.begin()`.
    pub store: Arc<dyn Store>,
    /// 조회 전용. 트랜잭션에 참여하지 않는다.
    pub views: Arc<dyn Views>,
    pub health_store: Arc<dyn HealthStore>,
    /// 스케줄러가 발화를 선점하는 데 쓴다(scheduler/, store/job_run.rs).
    pub job_run_store: Arc<dyn JobRunStore>,
}

impl AppState {
    /// 프로덕션 조립용 생성자.
    // [Rust 특징] 타입 파라미터 `S`에 필요한 포트를 한꺼번에 요구해서 "저장소 구현체는
    // 하나"라는 사실을 시그니처로 표현했다. 넘긴 `Arc<S>`가 각 필드의 `Arc<dyn ...>`로
    // 바뀌는 건 unsized coercion — refcount만 올리고 vtable을 붙이는 것이라 실질 비용이 없다.
    pub fn new<S>(store: Arc<S>) -> Arc<Self>
    where
        S: Store + Views + HealthStore + JobRunStore + 'static,
    {
        Arc::new(Self {
            store: store.clone(),
            views: store.clone(),
            health_store: store.clone(),
            job_run_store: store,
        })
    }
}
