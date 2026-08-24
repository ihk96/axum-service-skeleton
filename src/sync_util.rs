use std::sync::{Mutex, MutexGuard};

/// Mutex를 잠근다. 이전에 락을 쥐고 있던 쪽이 그 안에서 panic해 "poisoned" 상태가
/// 되어도 guard를 그대로 복구해서 돌려준다.
// [Rust 특징] std::sync::Mutex는 락을 쥔 스레드가 panic하면 락을 poisoned로 표시하고,
// 이후 `.lock()`이 Err를 반환해 "데이터가 반쯤 갱신됐을 수 있다"고 알린다. 그런데 axum은
// 핸들러 하나의 패닉을 그 태스크만 격리해서 처리하므로 프로세스는 계속 살아있다 — 그
// 다음 요청부터 공유 상태를 영구히 못 쓰게 되는 건 곤란하다. 그래서 poison 여부와 무관하게
// 내부 값을 꺼내 쓴다: "패닉이 못 끝낸 갱신"보다 구조적으로 유효한 이전 상태를 택한 것.
// [아키텍처] Mutex를 직접 잠그는 모든 곳이 `.lock()`을 바로 쓰지 않고 항상 이 함수를
// 거치게 통일해서, poison 복구 정책이 한 곳에만 존재하게 한다.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // [테스트 시나리오] 다른 스레드가 락을 쥔 채 panic해 poisoned를 만든 뒤에도 lock()이
    // 값을 꺼내오는지, panic 직전까지의 쓰기가 살아있는지, 이후 갱신도 되는지 확인한다.
    #[test]
    fn lock_recovers_after_a_panic_while_holding_the_lock() {
        let mutex = Arc::new(Mutex::new(0));

        let panicking = mutex.clone();
        let result = std::panic::catch_unwind(move || {
            let mut guard = panicking.lock().unwrap();
            *guard = 1;
            panic!("simulated panic while holding the lock");
        });
        assert!(result.is_err());

        // 표준 `.lock().unwrap()`을 썼다면 여기서 panic했을 것이다.
        let mut guard = lock(&mutex);
        assert_eq!(*guard, 1);
        *guard += 1;
        drop(guard);

        assert_eq!(*lock(&mutex), 2);
    }
}
