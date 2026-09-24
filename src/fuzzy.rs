pub fn score(candidate: &str, query: &str) -> Option<i64> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let candidate = candidate.to_lowercase();
    let chars: Vec<char> = candidate.chars().collect();
    let mut score = 0i64;
    let mut cursor = 0usize;
    let mut previous = None;
    for needle in query.chars() {
        let found = chars
            .iter()
            .enumerate()
            .skip(cursor)
            .find(|(_, ch)| **ch == needle)
            .map(|(index, _)| index)?;
        score += 10;
        if previous.is_some_and(|prev| found == prev + 1) {
            score += 9;
        }
        if found == 0
            || chars
                .get(found.wrapping_sub(1))
                .is_some_and(|ch| !ch.is_alphanumeric())
        {
            score += 7;
        }
        score -= found as i64 / 6;
        previous = Some(found);
        cursor = found + 1;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_subsequences_and_rejects_missing_characters() {
        assert!(score("Visual Studio Code", "vsc").is_some());
        assert!(score("Firefox", "xyz").is_none());
        assert!(score("Firefox", "").is_some());
    }

    #[test]
    fn prefers_contiguous_and_word_boundary_matches() {
        let contiguous = score("Code", "code").expect("contiguous match");
        let scattered = score("Creative Outline Document Editor", "code").expect("scattered match");
        assert!(contiguous > scattered);
    }

    #[test]
    fn is_case_insensitive_and_trims_query() {
        assert_eq!(score("Firefox", "FIREFOX"), score("firefox", "firefox"));
        assert_eq!(score("Firefox", "  fi  "), score("Firefox", "fi"));
    }
}
