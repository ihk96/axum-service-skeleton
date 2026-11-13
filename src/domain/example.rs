// [예제] 도메인 규칙의 모양. 저장소도 HTTP도 모르는 **순수 함수**만 둔다 — 입력을 받아
// 판단하고 결과를 돌려줄 뿐이라, 테스트에 mock조차 필요 없다.
//
// [설명] 이런 규칙이 usecase나 HTTP 핸들러가 아니라 여기 있는 이유 — 같은 규칙을 생성과
// 수정이 함께 쓰고, CLI나 컨슈머 같은 다른 입구가 생겨도 규칙은 한 곳에 남아야 하기 때문이다.
use crate::error::AppError;

const NAME_MAX_CHARS: usize = 64;

/// example 이름을 정리하고 검증한다. 통과하면 저장할 형태(앞뒤 공백 제거)를 돌려준다.
pub fn normalize_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();

    if name.is_empty() {
        return Err(AppError::BadRequest("name must not be empty".into()));
    }
    if name.chars().count() > NAME_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "name must be {NAME_MAX_CHARS} characters or fewer"
        )));
    }

    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_name_is_rejected() {
        for name in ["", "   "] {
            assert!(matches!(normalize_name(name), Err(AppError::BadRequest(_))));
        }
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(normalize_name("  hello  ").unwrap(), "hello");
    }

    // [테스트 시나리오] 길이는 바이트가 아니라 글자 수로 센다 — 한글 64자는 192바이트지만 통과해야 한다.
    #[test]
    fn length_is_counted_in_characters() {
        assert!(normalize_name(&"가".repeat(NAME_MAX_CHARS)).is_ok());
        assert!(normalize_name(&"가".repeat(NAME_MAX_CHARS + 1)).is_err());
    }
}
