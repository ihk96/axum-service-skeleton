// [아키텍처] 이 서비스가 **무엇을 언제 돌리는가**, 그것만 적는 파일이다. 어떻게 도는지
// (크론식 해석, 루프, 중복 실행 방지)는 scheduler/의 일이고, 여기서는 그 이름조차 나오지
// 않는다 — 등록 창구인 `JobRegistry`와 시간대를 고르는 `Tz` 둘만 보인다.
//
// job을 추가할 때 손대는 파일은 둘뿐이다: job 함수를 쓸 domain/ 모듈과, 아래 한 줄.
//
// http/mod.rs가 라우트를 모으는 것과 같은 트레이드오프를 택했다. 링크 타임 등록
// (inventory/linkme)으로 "파일만 추가하면 자동 등록"을 흉내낼 수는 있지만, 그러면 **이
// 서비스가 무엇을 언제 돌리는지 한눈에 볼 수 있는 자리**가 사라진다. 배치는 조용히 도는
// 코드라 그 목록이 더더욱 눈에 보여야 한다.
//
// 크론식 오타나 이름 중복은 기동 시점에 걸러져서 프로세스가 아예 뜨지 않는다.
use crate::{
    scheduler::{JobRegistry, Tz},
    usecase::example_job,
};

/// 이 서비스가 돌리는 스케줄 job 목록.
pub fn register(jobs: &mut JobRegistry) {
    // 인자 순서는 (이름, 크론식, 시간대, job 함수).
    //
    //   ┌───────── 분   (0-59)
    //   │ ┌─────── 시   (0-23)
    //   │ │ ┌───── 일   (1-31)
    //   │ │ │ ┌─── 월   (1-12, JAN-DEC)
    //   │ │ │ │ ┌─ 요일 (0-6, SUN-SAT)
    //   0 3 * * *      → 매일 03:00 (뒤에 적은 시간대 기준)
    //
    // 초 단위가 필요하면 필드를 하나 앞에 붙여 6필드로 쓴다("*/30 * * * * *" = 30초마다).
    // `@daily`, `@hourly` 같은 별칭도 그대로 받는다.
    jobs.add(
        "example_daily_report",
        "0 3 * * *",
        Tz::Asia__Seoul,
        example_job::run,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::Scheduler;

    // [테스트 시나리오] 등록부 전체가 실제로 조립되는지. 크론식은 문자열이라 오타가
    // 컴파일 타임에 걸리지 않으므로, job을 추가할 때마다 이 테스트가 그 자리를 대신한다.
    // 기동 시점에도 같은 검사를 하지만, 그건 배포한 뒤에야 안다.
    #[test]
    fn every_registered_job_is_valid() {
        Scheduler::validate(register).unwrap();
    }
}
