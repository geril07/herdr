//! Subsequence fuzzy matcher for popup search (navigator, agent picker).
//!
//! Hand-rolled fzf-like scoring for small lists: subsequence filter,
//! per-word scores with consecutive, word-boundary, and start-of-string
//! bonuses plus gap penalties, smart case, and match indices for highlight.
//! Sync and dependency-free. Ranking uses an allocation-free score path
//! (after a subsequence prefilter); index tables are built only for visible
//! rows at render time.

const MAX_QUERY_CHARS: usize = 64;
const MAX_HAYSTACK_CHARS: usize = 256;

const SCORE_MATCH: i64 = 10;
const BONUS_SEPARATOR: i64 = 6;
const BONUS_WHITE: i64 = 8;
const FIRST_CHAR_MULTIPLIER: i64 = 2;
const BONUS_CONSECUTIVE: i64 = 8;
const GAP_START: i64 = -3;
const GAP_EXTENSION: i64 = -1;
const LENGTH_PENALTY_DIVISOR: i64 = 8;
const NO_SCORE: i64 = -1_000_000_000;

pub(super) struct FuzzyQuery {
    pub(super) words: Vec<Vec<char>>,
    pub(super) case_sensitive: bool,
}

pub(super) fn parse_query(query: &str) -> FuzzyQuery {
    let truncated: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
    let case_sensitive = truncated.chars().any(|c| c.is_uppercase());
    let words = truncated
        .split_whitespace()
        .map(|word| word.chars().collect::<Vec<_>>())
        .filter(|word| !word.is_empty())
        .collect();
    FuzzyQuery {
        words,
        case_sensitive,
    }
}

#[derive(Default)]
pub(super) struct FuzzyScratch {
    chars: Vec<char>,
    prev_scores: Vec<i64>,
    prev_gaps: Vec<i64>,
    cur_scores: Vec<i64>,
    cur_gaps: Vec<i64>,
}

impl FuzzyScratch {
    pub(super) fn new() -> Self {
        Self::default()
    }
}

fn chars_equal(hay: char, pat: char, case_sensitive: bool) -> bool {
    if hay == pat {
        return true;
    }
    if case_sensitive {
        return false;
    }
    hay.eq_ignore_ascii_case(&pat) || hay.to_lowercase().next() == pat.to_lowercase().next()
}

fn is_separator(c: char) -> bool {
    matches!(c, '-' | '_' | '/' | '.' | ':')
}

fn position_bonus(prev: Option<char>) -> i64 {
    match prev {
        None => BONUS_WHITE,
        Some(c) if c.is_whitespace() => BONUS_WHITE,
        Some(c) if is_separator(c) => BONUS_SEPARATOR,
        _ => 0,
    }
}

/// Score one word against one field. Returns `None` when the word is not a
/// subsequence of the field. Never allocates on the non-match path: the
/// subsequence prefilter runs before the dynamic program, and the program
/// reuses caller-provided scratch rows.
pub(super) fn fuzzy_word_score(
    haystack: &str,
    word: &[char],
    case_sensitive: bool,
    scratch: &mut FuzzyScratch,
) -> Option<i64> {
    if word.is_empty() {
        return Some(0);
    }
    scratch.chars.clear();
    scratch
        .chars
        .extend(haystack.chars().take(MAX_HAYSTACK_CHARS));
    let n = scratch.chars.len();
    let m = word.len();
    if m > n {
        return None;
    }
    if !is_subsequence(&scratch.chars, word, case_sensitive) {
        return None;
    }
    scratch.prev_scores.resize(n, NO_SCORE);
    scratch.prev_gaps.resize(n, NO_SCORE);
    scratch.cur_scores.resize(n, NO_SCORE);
    scratch.cur_gaps.resize(n, NO_SCORE);

    for (j, cell) in scratch.prev_scores.iter_mut().enumerate().take(n) {
        *cell = if chars_equal(scratch.chars[j], word[0], case_sensitive) {
            SCORE_MATCH + FIRST_CHAR_MULTIPLIER * position_bonus(prev_char(&scratch.chars, j))
        } else {
            NO_SCORE
        };
    }
    running_gap_row(&scratch.prev_scores, &mut scratch.prev_gaps, n);

    for pat in word.iter().skip(1) {
        for j in 0..n {
            let mut best = NO_SCORE;
            if chars_equal(scratch.chars[j], *pat, case_sensitive) {
                if j > 0 && scratch.prev_scores[j - 1] != NO_SCORE {
                    best = scratch.prev_scores[j - 1] + SCORE_MATCH + BONUS_CONSECUTIVE;
                }
                if j > 0 && scratch.prev_gaps[j - 1] != NO_SCORE {
                    let gap = scratch.prev_gaps[j - 1] + GAP_START - GAP_EXTENSION
                        + SCORE_MATCH
                        + position_bonus(prev_char(&scratch.chars, j));
                    if gap > best {
                        best = gap;
                    }
                }
            }
            scratch.cur_scores[j] = best;
        }
        running_gap_row(&scratch.cur_scores, &mut scratch.cur_gaps, n);
        std::mem::swap(&mut scratch.prev_scores, &mut scratch.cur_scores);
        std::mem::swap(&mut scratch.prev_gaps, &mut scratch.cur_gaps);
    }

    let answer = scratch.prev_scores.iter().take(n).max().copied();
    match answer {
        Some(score) if score != NO_SCORE => Some(score - n as i64 / LENGTH_PENALTY_DIVISOR),
        _ => None,
    }
}

fn prev_char(chars: &[char], j: usize) -> Option<char> {
    if j == 0 {
        None
    } else {
        Some(chars[j - 1])
    }
}

fn running_gap_row(scores: &[i64], gaps: &mut [i64], n: usize) {
    for j in 0..n {
        let extend = if j > 0 && gaps[j - 1] != NO_SCORE {
            gaps[j - 1] + GAP_EXTENSION
        } else {
            NO_SCORE
        };
        gaps[j] = scores[j].max(extend);
    }
}

fn is_subsequence(haystack: &[char], word: &[char], case_sensitive: bool) -> bool {
    let mut rest = word.iter();
    let mut current = rest.next();
    for hay in haystack {
        if let Some(pat) = current {
            if chars_equal(*hay, *pat, case_sensitive) {
                current = rest.next();
            }
        } else {
            break;
        }
    }
    current.is_none()
}

/// Score every word against a set of fields. A row matches when each word
/// matches at least one field; the row score is the sum of per-word best
/// field scores.
pub(super) fn match_any_field(
    fields: &[&str],
    query: &FuzzyQuery,
    scratch: &mut FuzzyScratch,
) -> Option<i64> {
    let mut total = 0_i64;
    for word in &query.words {
        let mut best: Option<i64> = None;
        for field in fields {
            if let Some(score) = fuzzy_word_score(field, word, query.case_sensitive, scratch) {
                best = Some(best.map_or(score, |known: i64| known.max(score)));
            }
        }
        total += best?;
    }
    Some(total)
}

/// Match indices (char positions) for highlight. One entry per word match;
/// callers union them over the displayed label. Built only for visible rows.
pub(super) fn fuzzy_word_match(
    haystack: &str,
    word: &[char],
    case_sensitive: bool,
) -> Option<(i64, Vec<usize>)> {
    if word.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<char> = haystack.chars().take(MAX_HAYSTACK_CHARS).collect();
    let n = chars.len();
    let m = word.len();
    if m > n || !is_subsequence(&chars, word, case_sensitive) {
        return None;
    }
    // scores[i * n + j]: best score matching word[..=i] with word[i] at chars[j].
    // back[i * n + j]: previous char index, u16::MAX for a match start.
    const NO_PREV: u16 = u16::MAX;
    let mut scores = vec![NO_SCORE; m * n];
    let mut back = vec![NO_PREV; m * n];
    let mut gap_values = vec![NO_SCORE; n];
    let mut gap_args: Vec<u16> = vec![0; n];
    for j in 0..n {
        if chars_equal(chars[j], word[0], case_sensitive) {
            scores[j] = SCORE_MATCH + FIRST_CHAR_MULTIPLIER * position_bonus(prev_char(&chars, j));
            back[j] = NO_PREV;
        }
        let extend = if j > 0 && gap_values[j - 1] != NO_SCORE {
            gap_values[j - 1] + GAP_EXTENSION
        } else {
            NO_SCORE
        };
        if scores[j] >= extend {
            gap_values[j] = scores[j];
            gap_args[j] = j as u16;
        } else {
            gap_values[j] = extend;
            gap_args[j] = gap_args[j - 1];
        }
    }
    for i in 1..m {
        let mut row_gaps = vec![NO_SCORE; n];
        let mut row_args: Vec<u16> = vec![0; n];
        for j in 0..n {
            let mut best = NO_SCORE;
            let mut arg = NO_PREV;
            if chars_equal(chars[j], word[i], case_sensitive) {
                if j > 0 && scores[(i - 1) * n + j - 1] != NO_SCORE {
                    best = scores[(i - 1) * n + j - 1] + SCORE_MATCH + BONUS_CONSECUTIVE;
                    arg = (j - 1) as u16;
                }
                if j > 0 && gap_values[j - 1] != NO_SCORE {
                    let gap = gap_values[j - 1] + GAP_START - GAP_EXTENSION
                        + SCORE_MATCH
                        + position_bonus(prev_char(&chars, j));
                    if gap > best {
                        best = gap;
                        arg = gap_args[j - 1];
                    }
                }
                scores[i * n + j] = best;
                back[i * n + j] = arg;
            }
            let extend = if j > 0 && row_gaps[j - 1] != NO_SCORE {
                row_gaps[j - 1] + GAP_EXTENSION
            } else {
                NO_SCORE
            };
            if scores[i * n + j] >= extend {
                row_gaps[j] = scores[i * n + j];
                row_args[j] = j as u16;
            } else {
                row_gaps[j] = extend;
                row_args[j] = row_args[j - 1];
            }
        }
        gap_values = row_gaps;
        gap_args = row_args;
    }
    let last = m - 1;
    let mut end = 0;
    let mut best = NO_SCORE;
    for j in 0..n {
        if scores[last * n + j] > best {
            best = scores[last * n + j];
            end = j;
        }
    }
    if best == NO_SCORE {
        return None;
    }
    let mut indices = Vec::with_capacity(m);
    let mut i = last;
    let mut j = end;
    loop {
        indices.push(j);
        if i == 0 {
            break;
        }
        let prev = back[i * n + j];
        if prev == NO_PREV {
            break;
        }
        j = prev as usize;
        i -= 1;
    }
    indices.reverse();
    Some((best - n as i64 / LENGTH_PENALTY_DIVISOR, indices))
}

/// Per-char match flags for painting the displayed label. Shorter than the
/// label when scoring was capped; the tail stays unhighlighted.
pub(super) fn highlight_for_label(label: &str, query: &FuzzyQuery) -> Vec<bool> {
    let len = label.chars().count();
    let mut out = vec![false; len];
    if query.words.is_empty() {
        return out;
    }
    for word in &query.words {
        if let Some((_, indices)) = fuzzy_word_match(label, word, query.case_sensitive) {
            for index in indices {
                if index < out.len() {
                    out[index] = true;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(haystack: &str, query: &str) -> Option<i64> {
        let parsed = parse_query(query);
        let mut scratch = FuzzyScratch::new();
        match_any_field(&[haystack], &parsed, &mut scratch)
    }

    fn word_score(haystack: &str, word: &str, case_sensitive: bool) -> Option<i64> {
        let mut scratch = FuzzyScratch::new();
        fuzzy_word_score(
            haystack,
            &word.chars().collect::<Vec<_>>(),
            case_sensitive,
            &mut scratch,
        )
    }

    #[test]
    fn empty_query_matches_everything_with_zero_score() {
        assert_eq!(score("anything", ""), Some(0));
        assert_eq!(score("anything", "   "), Some(0));
    }

    #[test]
    fn subsequence_matches_while_wrong_order_does_not() {
        assert!(score("my-cool-dev", "mcld").is_some());
        assert!(score("my-cool-dev", "dclm").is_none());
        assert!(score("ab", "abc").is_none());
    }

    #[test]
    fn consecutive_match_beats_gappy_match() {
        let tight = score("ab-test", "ab").unwrap_or(NO_SCORE);
        let gappy = score("a--b-test", "ab").unwrap_or(NO_SCORE);
        assert!(tight > gappy);
    }

    #[test]
    fn word_boundary_match_beats_mid_word_match() {
        let boundary = score("my-dev", "dev").unwrap_or(NO_SCORE);
        let middle = score("adev", "dev").unwrap_or(NO_SCORE);
        assert!(boundary > middle);
    }

    #[test]
    fn prefix_match_beats_middle_match() {
        let prefix = score("agent-x", "ag").unwrap_or(NO_SCORE);
        let middle = score("my-agent", "ag").unwrap_or(NO_SCORE);
        assert!(prefix > middle);
    }

    #[test]
    fn shorter_haystack_wins_ties() {
        let short = score("api", "api").unwrap_or(NO_SCORE);
        let long = score("api-integration-extra-long-name", "api").unwrap_or(NO_SCORE);
        assert!(short > long);
    }

    #[test]
    fn smart_case_is_insensitive_until_uppercase() {
        assert!(score("Agent-X", "agent").is_some());
        assert!(score("agent-x", "Agent").is_none());
        assert!(score("Agent-X", "Agent").is_some());
    }

    #[test]
    fn and_words_match_across_fields() {
        let parsed = parse_query("auth api");
        let mut scratch = FuzzyScratch::new();
        assert!(match_any_field(&["auth-service", "api-gateway"], &parsed, &mut scratch).is_some());
        assert!(match_any_field(&["auth-service", "web"], &parsed, &mut scratch).is_none());
    }

    #[test]
    fn long_inputs_are_capped_without_panicking() {
        let long_query = "a".repeat(200);
        let long_haystack = "b".repeat(500);
        assert!(score(&long_haystack, &long_query).is_none());
        assert!(score("abc", &long_query).is_none());
    }

    #[test]
    fn highlight_marks_the_matched_word() {
        let parsed = parse_query("mcld");
        let flags = highlight_for_label("my-cool-dev", &parsed);
        assert_eq!(flags.len(), "my-cool-dev".chars().count());
        let matched: String = "my-cool-dev"
            .chars()
            .zip(flags.iter())
            .filter_map(|(c, hit)| hit.then_some(c))
            .collect();
        assert_eq!(matched, "mcld");
    }

    #[test]
    fn index_path_scores_match_ranking_scores() {
        let corpus = [
            ("my-cool-dev", "mcld"),
            ("agent-picker-worktree", "apw"),
            ("a--b-test", "ab"),
            ("My Agent (1)", "ma1"),
            ("reagent-agent", "agent"),
            ("x", "xyz"),
            ("", "a"),
            ("Café au lait", "cfl"),
        ];
        for (haystack, query) in corpus {
            let parsed = parse_query(query);
            for word in &parsed.words {
                let word_string: String = word.iter().collect();
                let rank = word_score(haystack, &word_string, parsed.case_sensitive);
                let full = fuzzy_word_match(haystack, word, parsed.case_sensitive).map(|(s, _)| s);
                assert_eq!(rank, full, "score mismatch for {haystack:?} / {query:?}");
            }
        }
    }
}
