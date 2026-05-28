//! TokenJuice — LLM 전단 토큰 압축 (spec §5.4, OpenHuman 차용).
//!
//! 큰 prompt/context를 budget 안으로 줄인다. 작은 입력은 그대로 통과.
//! 규칙: 공백/중복 줄 정리 → 여전히 초과면 char-safe 절단.
//!
//! grapheme 정밀 분할(이모지 ZWJ 등)은 unicode-segmentation 의존이 필요 → 현재는
//! **char(코드포인트) 경계**로 안전 절단(CJK 안전). 정밀 grapheme은 후속.

/// 입력을 `budget_chars`(문자 수) 이내로 압축. 이미 작으면 그대로 반환.
pub fn compress_context(input: &str, budget_chars: usize) -> String {
    if count_chars(input) <= budget_chars {
        return input.to_string();
    }
    // 1) 공백·중복 줄 정리
    let tidied = tidy(input);
    if count_chars(&tidied) <= budget_chars {
        return tidied;
    }
    // 2) 여전히 초과 → char-safe 절단 + 말줄임
    truncate_chars(&tidied, budget_chars)
}

fn count_chars(s: &str) -> usize {
    s.chars().count()
}

/// 3줄 이상 연속 빈 줄 → 1줄, 인접 중복 줄 제거, 우측 공백 트림.
fn tidy(input: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut blank_run = 0usize;
    let mut prev: Option<&str> = None;
    for raw in input.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run >= 2 {
                continue; // 연속 빈 줄 압축
            }
        } else {
            blank_run = 0;
            if prev == Some(line) {
                continue; // 인접 중복 줄 제거
            }
        }
        out.push(line);
        prev = Some(line);
    }
    // 선행/후행 빈 줄 정리
    while out.first().is_some_and(|l| l.is_empty()) {
        out.remove(0);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// char 경계로 안전하게 총 `budget` 문자 이내로 자른다.
/// budget이 말줄임 표시보다 크면 표시를 포함(표시 길이만큼 본문을 줄여 총합 유지),
/// 작으면 표시 없이 budget 문자만.
fn truncate_chars(s: &str, budget: usize) -> String {
    if budget == 0 {
        return String::new();
    }
    let marker = "\n…(TokenJuice 압축됨)";
    let marker_len = marker.chars().count();
    if budget > marker_len {
        let keep = budget - marker_len;
        let truncated: String = s.chars().take(keep).collect();
        format!("{truncated}{marker}")
    } else {
        s.chars().take(budget).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_input_unchanged() {
        let s = "짧은 입력";
        assert_eq!(compress_context(s, 100), s);
    }

    #[test]
    fn collapses_blank_lines_and_dupes() {
        let input = "a\n\n\n\na\na\nb";
        let out = tidy(input);
        // 연속 빈 줄 1개로, 인접 중복 a 제거
        assert_eq!(out, "a\n\na\nb");
    }

    #[test]
    fn tidy_alone_can_fit_budget() {
        // 큰 중복/공백이 정리만으로 예산에 들어오면 절단 안 함
        let input = "line\n\n\n\n\nline\nline\nend";
        // tidy 결과("line\n\nline\nend", 14자)는 16 예산에 들어가 절단 안 함
        let out = compress_context(input, 16);
        assert!(!out.contains("압축됨"));
        assert!(out.contains("line"));
    }

    #[test]
    fn truncates_when_over_budget_char_safe() {
        let input = "가나다라마바사아자차카타파하".repeat(10);
        let out = compress_context(&input, 20);
        assert!(count_chars(&out) <= 20);
        // 멀티바이트 경계 안전 — 유효한 UTF-8
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn truncate_tiny_budget_no_marker() {
        let out = truncate_chars("가나다라마", 2);
        assert_eq!(count_chars(&out), 2);
        assert!(!out.contains("압축됨"));
    }

    #[test]
    fn budget_zero_empty() {
        assert_eq!(truncate_chars("x", 0), "");
    }

    #[test]
    fn over_budget_includes_marker_when_room() {
        let input = "a".repeat(500);
        let out = compress_context(&input, 100);
        assert!(out.contains("압축됨"));
        assert!(count_chars(&out) <= 100);
    }
}
