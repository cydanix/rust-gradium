//! Text similarity metrics for comparing TTS/STT output.

use std::collections::HashMap;

/// Similarity scores between two texts.
#[derive(Debug, Clone)]
pub struct Scores {
    /// Word Error Rate ∈ [0,1] (lower is better)
    pub wer: f64,
    /// Character Error Rate ∈ [0,1] (lower is better)
    pub cer: f64,
    /// Bag-of-words F1 score ∈ [0,1] (higher is better)
    pub token_f1: f64,
    /// Combined similarity score ∈ [0,1] (higher is better)
    pub similarity: f64,
}

/// Compare two texts and return detailed similarity metrics.
///
/// Normalization: lowercases, strips punctuation/symbols, collapses spaces.
pub fn compare(reference: &str, hypothesis: &str) -> Scores {
    let wer = word_error_rate(reference, hypothesis);
    let cer = char_error_rate(reference, hypothesis);
    let token_f1 = token_f1_score(reference, hypothesis);

    // Simple blend: average of (1-WER), (1-CER), and F1
    let mut similarity = ((1.0 - wer) + (1.0 - cer) + token_f1) / 3.0;
    similarity = similarity.clamp(0.0, 1.0);

    Scores {
        wer,
        cer,
        token_f1,
        similarity,
    }
}

/// Compute Word Error Rate between normalized texts.
pub fn word_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let ref_norm = normalize(reference);
    let hyp_norm = normalize(hypothesis);
    let r: Vec<&str> = ref_norm.split_whitespace().collect();
    let h: Vec<&str> = hyp_norm.split_whitespace().collect();

    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }

    let dist = edit_distance(&r, &h);
    dist as f64 / r.len() as f64
}

/// Compute Character Error Rate on normalized texts (spaces removed).
pub fn char_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let r: Vec<char> = normalize(reference)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let h: Vec<char> = normalize(hypothesis)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }

    let dist = edit_distance(&r, &h);
    dist as f64 / r.len() as f64
}

/// Compute bag-of-words F1 score (order-agnostic).
pub fn token_f1_score(reference: &str, hypothesis: &str) -> f64 {
    let ref_norm = normalize(reference);
    let hyp_norm = normalize(hypothesis);
    let r: Vec<&str> = ref_norm.split_whitespace().collect();
    let h: Vec<&str> = hyp_norm.split_whitespace().collect();

    if r.is_empty() && h.is_empty() {
        return 1.0;
    }

    let mut rc: HashMap<&str, usize> = HashMap::new();
    let mut hc: HashMap<&str, usize> = HashMap::new();

    for t in &r {
        *rc.entry(*t).or_default() += 1;
    }
    for t in &h {
        *hc.entry(*t).or_default() += 1;
    }

    let mut correct = 0;
    for (t, &rv) in &rc {
        if let Some(&hv) = hc.get(t) {
            correct += rv.min(hv);
        }
    }

    if correct == 0 {
        return 0.0;
    }

    let precision = correct as f64 / h.len() as f64;
    let recall = correct as f64 / r.len() as f64;
    2.0 * precision * recall / (precision + recall)
}

/// Normalize text: lowercase, remove punctuation/symbols, collapse whitespace.
fn normalize(s: &str) -> String {
    let normalized: String = s
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphabetic() || c.is_numeric() {
                c
            } else if c.is_whitespace() {
                ' '
            } else {
                ' ' // replace punctuation with space
            }
        })
        .collect();

    // Collapse multiple spaces
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Compute Levenshtein edit distance.
fn edit_distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let la = a.len();
    let lb = b.len();

    if la == 0 {
        return lb;
    }
    if lb == 0 {
        return la;
    }

    let mut prev: Vec<usize> = (0..=lb).collect();
    let mut curr = vec![0; lb + 1];

    for i in 1..=la {
        curr[0] = i;
        for j in 1..=lb {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let del = prev[j] + 1;
            let ins = curr[j - 1] + 1;
            let sub = prev[j - 1] + cost;
            curr[j] = del.min(ins).min(sub);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[lb]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_texts() {
        let scores = compare("hello world", "hello world");
        assert_eq!(scores.wer, 0.0);
        assert_eq!(scores.cer, 0.0);
        assert_eq!(scores.token_f1, 1.0);
        assert_eq!(scores.similarity, 1.0);
    }

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("Hello, World!"), "hello world");
        assert_eq!(normalize("  multiple   spaces  "), "multiple spaces");
        assert_eq!(normalize("punctuation...removed"), "punctuation removed");
    }

    #[test]
    fn test_edit_distance() {
        let a: Vec<char> = "kitten".chars().collect();
        let b: Vec<char> = "sitting".chars().collect();
        assert_eq!(edit_distance(&a, &b), 3);
    }
}

