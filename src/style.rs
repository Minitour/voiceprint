//! Surface style features: the habits that word choice alone doesn't capture.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::text::is_word;

const LLM_TRANSITIONS: [&str; 10] = [
    "moreover",
    "furthermore",
    "additionally",
    "notably",
    "importantly",
    "crucially",
    "ultimately",
    "overall",
    "essentially",
    "consequently",
];
const FIRST_SINGULAR: [&str; 5] = ["i", "me", "my", "mine", "myself"];
const FIRST_PLURAL: [&str; 5] = ["we", "us", "our", "ours", "ourselves"];
const CONTRACTION_ENDINGS: [&str; 6] = ["t", "re", "ve", "ll", "m", "d"];

/// (name, label, unit) in reporting order.
pub const FEATURES: [(&str, &str, &str); 13] = [
    ("sentence_length_mean", "Average sentence length", "words"),
    (
        "sentence_length_sd",
        "Sentence length variation (sd)",
        "words",
    ),
    ("comma_per_sentence", "Commas per sentence", ""),
    ("em_dash_per_1k", "Em dashes", "per 1k words"),
    ("semicolon_per_1k", "Semicolons", "per 1k words"),
    ("colon_per_1k", "Colons", "per 1k words"),
    ("parenthesis_per_1k", "Parentheses", "per 1k words"),
    ("question_per_1k", "Questions", "per 1k words"),
    ("first_singular_per_1k", "I / me / my", "per 1k words"),
    ("first_plural_per_1k", "we / us / our", "per 1k words"),
    ("contractions_per_1k", "Contractions", "per 1k words"),
    (
        "llm_transitions_per_1k",
        "Moreover/furthermore/notably/...",
        "per 1k words",
    ),
    ("mean_word_length", "Average word length", "chars"),
];

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct FeatureStats {
    pub mean: f64,
    pub sd: f64,
    pub p10: f64,
    pub p90: f64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct StyleProfile {
    pub features: BTreeMap<String, FeatureStats>,
    pub chunks: usize,
    pub chunk_words: usize,
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

fn pstdev(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
}

/// Features over sentences given as lists of normalized tokens.
pub fn features(sents: &[Vec<String>]) -> HashMap<&'static str, f64> {
    let words: Vec<&String> = sents.iter().flatten().filter(|t| is_word(t)).collect();
    let nw = words.len().max(1) as f64;
    let mut lens: Vec<f64> = sents
        .iter()
        .map(|s| s.iter().filter(|t| is_word(t)).count() as f64)
        .filter(|&n| n > 0.0)
        .collect();
    if lens.is_empty() {
        lens.push(0.0);
    }
    let mut c: HashMap<&str, f64> = HashMap::new();
    for t in sents.iter().flatten() {
        *c.entry(t.as_str()).or_default() += 1.0;
    }
    let get = |k: &str| c.get(k).copied().unwrap_or(0.0);
    let per_k = |n: f64| 1000.0 * n / nw;
    let contractions: f64 = c
        .iter()
        .filter(|(t, _)| {
            t.contains('\'') && CONTRACTION_ENDINGS.contains(&t.rsplit('\'').next().unwrap_or(""))
        })
        .map(|(_, n)| n)
        .sum();
    let word_lens: Vec<f64> = words.iter().map(|w| w.chars().count() as f64).collect();
    HashMap::from([
        ("sentence_length_mean", mean(&lens)),
        ("sentence_length_sd", pstdev(&lens)),
        ("comma_per_sentence", get(",") / lens.len() as f64),
        ("em_dash_per_1k", per_k(get("—"))),
        ("semicolon_per_1k", per_k(get(";"))),
        ("colon_per_1k", per_k(get(":"))),
        ("parenthesis_per_1k", per_k(get("("))),
        ("question_per_1k", per_k(get("?"))),
        (
            "first_singular_per_1k",
            per_k(FIRST_SINGULAR.iter().map(|w| get(w)).sum()),
        ),
        (
            "first_plural_per_1k",
            per_k(FIRST_PLURAL.iter().map(|w| get(w)).sum()),
        ),
        ("contractions_per_1k", per_k(contractions)),
        (
            "llm_transitions_per_1k",
            per_k(LLM_TRANSITIONS.iter().map(|w| get(w)).sum()),
        ),
        ("mean_word_length", mean(&word_lens)),
    ])
}

/// Feature distribution over ~chunk_words-sized chunks of the corpus.
pub fn chunked_profile(docs: &[Vec<Vec<String>>], chunk_words: usize) -> StyleProfile {
    let mut chunks: Vec<Vec<Vec<String>>> = Vec::new();
    for doc in docs {
        let (mut cur, mut n) = (Vec::new(), 0);
        for s in doc {
            cur.push(s.clone());
            n += s.iter().filter(|t| is_word(t)).count();
            if n >= chunk_words {
                chunks.push(std::mem::take(&mut cur));
                n = 0;
            }
        }
        if n >= chunk_words / 2 {
            chunks.push(cur);
        }
    }
    if chunks.is_empty() {
        chunks.push(docs.iter().flatten().cloned().collect());
    }
    let rows: Vec<HashMap<&str, f64>> = chunks.iter().map(|c| features(c)).collect();
    let mut out = BTreeMap::new();
    for (name, _, _) in FEATURES {
        let mut vals: Vec<f64> = rows.iter().map(|r| r[name]).collect();
        vals.sort_by(f64::total_cmp);
        let at = |q: f64| vals[(q * (vals.len() - 1) as f64) as usize];
        out.insert(
            name.to_string(),
            FeatureStats {
                mean: mean(&vals),
                sd: pstdev(&vals),
                p10: at(0.1),
                p90: at(0.9),
            },
        );
    }
    StyleProfile {
        features: out,
        chunks: chunks.len(),
        chunk_words,
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct StyleRow {
    pub feature: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub draft: f64,
    pub yours_mean: f64,
    pub yours_range: [f64; 2],
    pub z: f64,
    pub off: bool,
}

pub fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

pub fn compare(draft: &HashMap<&'static str, f64>, profile: &StyleProfile) -> Vec<StyleRow> {
    FEATURES
        .iter()
        .filter_map(|&(name, label, unit)| {
            let p = profile.features.get(name)?;
            let x = draft[name];
            // Floor the sd so features you never use (sd = 0) still yield a finite z.
            let floor = if unit.starts_with("per") { 0.5 } else { 0.05 };
            let sd = p.sd.max(0.15 * p.mean.abs()).max(floor);
            let z = (x - p.mean) / sd;
            Some(StyleRow {
                feature: name,
                label,
                unit,
                draft: round2(x),
                yours_mean: round2(p.mean),
                yours_range: [round2(p.p10), round2(p.p90)],
                z: round2(z),
                off: z.abs() >= 2.0 && !(p.p10 <= x && x <= p.p90),
            })
        })
        .collect()
}
