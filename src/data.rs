//! Bundled data: general-English word frequencies, synonyms, and the LLM-marker list.
//! See data/NOTICE.md for sources and licenses.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::OnceLock;

use flate2::read::GzDecoder;
use regex::Regex;

static ZIPF_GZ: &[u8] = include_bytes!("../data/en_zipf.tsv.gz");
static SYNONYMS_GZ: &[u8] = include_bytes!("../data/en_synonyms.tsv.gz");
pub static MARKERS_TXT: &str = include_str!("../data/llm_markers.txt");

fn gunzip(bytes: &[u8]) -> String {
    let mut out = String::new();
    GzDecoder::new(bytes)
        .read_to_string(&mut out)
        .expect("bundled data is valid gzip");
    out
}

struct Zipf {
    map: HashMap<String, f64>,
    /// Words in descending frequency order (the file is sorted that way).
    order: Vec<String>,
}

fn zipf_table() -> &'static Zipf {
    static Z: OnceLock<Zipf> = OnceLock::new();
    Z.get_or_init(|| {
        let text = gunzip(ZIPF_GZ);
        let mut map = HashMap::with_capacity(330_000);
        let mut order = Vec::with_capacity(330_000);
        for line in text.lines() {
            if let Some((w, z)) = line.split_once('\t') {
                if let Ok(z) = z.parse::<f64>() {
                    map.insert(w.to_string(), z);
                    order.push(w.to_string());
                }
            }
        }
        Zipf { map, order }
    })
}

/// Zipf frequency of `word` in general English (log10 of frequency per billion words),
/// compatible with wordfreq's `zipf_frequency`: 0 when unknown, and multi-part words
/// ("mobile-friendly") combine their parts as 1/f = 1/f1 + 1/f2 + ...
pub fn zipf(word: &str) -> f64 {
    let table = zipf_table();
    let lower = word.to_lowercase().replace('’', "'");
    if let Some(z) = table.map.get(lower.as_str()) {
        return *z;
    }
    let parts: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '.'))
        .map(|p| p.trim_matches('.'))
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return 0.0;
    }
    let mut one_over = 0.0;
    for p in &parts {
        match table.map.get(*p) {
            Some(z) => one_over += 1.0 / 10f64.powf(z - 9.0),
            None => return 0.0,
        }
    }
    let z = (1.0 / one_over).log10() + 9.0;
    (z * 100.0).round() / 100.0
}

/// The `n` most frequent English words.
pub fn top_english(n: usize) -> &'static [String] {
    let order = &zipf_table().order;
    &order[..n.min(order.len())]
}

/// Meaning-preserving alternatives for a word (curated map + WordNet, inflected like it).
pub fn synonyms(word: &str) -> &'static [String] {
    static S: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    let map = S.get_or_init(|| {
        let text = gunzip(SYNONYMS_GZ);
        let mut map = HashMap::with_capacity(40_000);
        for line in text.lines() {
            if let Some((w, alts)) = line.split_once('\t') {
                map.insert(w.to_string(), alts.split('|').map(str::to_string).collect());
            }
        }
        map
    });
    map.get(word).map(Vec::as_slice).unwrap_or(&[])
}

/// LLM-marker words and phrase patterns. Lines starting with `re:` are regexes;
/// lines with spaces are phrases; the rest are single words.
pub struct Markers {
    pub words: HashSet<String>,
    pub patterns: Vec<(String, Regex)>,
}

pub fn load_markers(custom: Option<&str>) -> Result<Markers, String> {
    let owned;
    let text = match custom {
        Some(path) => {
            owned =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            owned.as_str()
        }
        None => MARKERS_TXT,
    };
    let mut words = HashSet::new();
    let mut patterns = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(re) = line.strip_prefix("re:") {
            // Linear-time engine only: no lookaround or backreferences in marker regexes.
            let r = Regex::new(&format!("(?i){re}"))
                .map_err(|e| format!("bad marker regex {line:?}: {e}"))?;
            patterns.push((line.to_string(), r));
        } else if line.contains(' ') {
            patterns.push((
                line.to_string(),
                crate::text::phrase_regex(&line.to_lowercase(), true),
            ));
        } else {
            words.insert(line.to_lowercase());
        }
    }
    Ok(Markers { words, patterns })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zipf_matches_wordfreq() {
        assert!((zipf("the") - 7.73).abs() < 0.05);
        assert_eq!(zipf("zzzqqqxx"), 0.0);
        // wordfreq: mobile-friendly 4.43, fine-grained 2.91
        assert!((zipf("mobile-friendly") - 4.43).abs() < 0.02);
        assert!((zipf("fine-grained") - 2.91).abs() < 0.02);
    }

    #[test]
    fn synonyms_and_markers_load() {
        assert!(synonyms("utilizes").iter().any(|s| s == "uses"));
        let m = load_markers(None).unwrap();
        assert!(m.words.contains("delve"));
        assert!(m.patterns.iter().any(|(l, _)| l == "it's worth noting"));
    }
}
