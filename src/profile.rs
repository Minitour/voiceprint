//! A human- and agent-readable "voice card" summarizing the index.

use std::collections::HashMap;

use crate::check::thousands;
use crate::data::{top_english, zipf, Markers};
use crate::model::VoiceModel;
use crate::style::FEATURES;
use crate::text::{is_placeholder, is_word, tokenize};

const TRANSITIONS: [&str; 35] = [
    "however",
    "thus",
    "therefore",
    "hence",
    "so",
    "also",
    "moreover",
    "furthermore",
    "additionally",
    "consequently",
    "indeed",
    "notably",
    "importantly",
    "specifically",
    "namely",
    "instead",
    "rather",
    "yet",
    "still",
    "meanwhile",
    "finally",
    "first",
    "second",
    "then",
    "overall",
    "in particular",
    "for example",
    "for instance",
    "in addition",
    "in contrast",
    "on the other hand",
    "as a result",
    "that is",
    "e.g.",
    "i.e.",
];

pub fn voice_card(m: &VoiceModel, markers: &Markers) -> String {
    let total = m.total_words.max(1) as f64;
    let words: Vec<(&str, u32)> = m
        .unigram
        .iter()
        .map(|(id, c)| (m.word(*id), *c))
        .filter(|(w, _)| is_word(w) && !is_placeholder(w) && w.chars().count() > 1)
        .collect();
    let ratio = |w: &str, c: u32| {
        let z = zipf(w);
        let eng = if z > 0.0 { 10f64.powf(z - 9.0) } else { 1e-9 };
        (c as f64 / total) / eng
    };
    let mut distinctive: Vec<(&str, u32)> = words
        .iter()
        .copied()
        .filter(|(w, c)| *c >= 5 && zipf(w) >= 4.0)
        .collect();
    distinctive.sort_by(|a, b| {
        ratio(b.0, b.1)
            .total_cmp(&ratio(a.0, a.1))
            .then(a.0.cmp(b.0))
    });
    distinctive.truncate(30);
    let mut topical: Vec<(&str, u32)> = words
        .iter()
        .copied()
        .filter(|(w, c)| *c >= 5 && zipf(w) < 4.0)
        .collect();
    topical.sort_by(|a, b| {
        ratio(b.0, b.1)
            .total_cmp(&ratio(a.0, a.1))
            .then(a.0.cmp(b.0))
    });
    topical.truncate(30);

    let seen: std::collections::HashSet<&str> = words.iter().map(|(w, _)| *w).collect();
    let avoided: Vec<&str> = top_english(4000)
        .iter()
        .map(String::as_str)
        .filter(|w| {
            !seen.contains(w) && w.chars().all(char::is_alphabetic) && w.chars().count() >= 2
        })
        .filter(|w| 10f64.powf(zipf(w) - 9.0) * total >= 5.0)
        .take(60)
        .collect();

    let corpus = format!(
        " {}",
        m.sentences
            .iter()
            .map(|s| s.to_lowercase())
            .collect::<Vec<_>>()
            .join(" \n")
    );
    let mut trans: Vec<(&str, usize)> = TRANSITIONS
        .iter()
        .map(|t| {
            let n = if t.contains(' ') {
                corpus.matches(&format!(" {t} ")).count()
                    + corpus.matches(&format!("\n{t} ")).count()
            } else {
                m.count(t) as usize
            };
            (*t, n)
        })
        .filter(|(_, n)| *n > 0)
        .collect();
    trans.sort_by(|a, b| b.1.cmp(&a.1));

    let mut openers: HashMap<String, usize> = HashMap::new();
    for s in &m.sentences {
        let ws: Vec<String> = tokenize(s)
            .into_iter()
            .map(|t| t.norm)
            .filter(|n| is_word(n))
            .take(2)
            .collect();
        if ws.len() == 2 {
            *openers.entry(ws.join(" ")).or_default() += 1;
        }
    }
    let mut openers: Vec<(String, usize)> = openers.into_iter().collect();
    openers.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    openers.truncate(20);

    let mut used_markers: Vec<&String> = markers.words.iter().filter(|w| m.count(w) >= 3).collect();
    used_markers.sort_by(|a, b| m.count(b).cmp(&m.count(a)).then(a.cmp(b)));
    let mut never: Vec<&String> = markers.words.iter().filter(|w| m.count(w) == 0).collect();
    never.sort();

    let join = |v: Vec<String>| {
        if v.is_empty() {
            "(none)".to_string()
        } else {
            v.join(", ")
        }
    };
    let mut l = vec![
        "# Voice card".to_string(),
        String::new(),
        format!(
            "Built {} from {} documents, {} words, {} sentences (n-gram order {}).",
            m.meta.built,
            m.meta.documents,
            thousands(m.meta.words),
            thousands(m.meta.sentences as u64),
            m.order
        ),
        String::new(),
        "## Style norms (typical p10-p90 range across ~300-word chunks)".into(),
        String::new(),
        "| Feature | Your mean | Typical range |".into(),
        "|---|---|---|".into(),
    ];
    for (name, label, unit) in FEATURES {
        if let Some(p) = m.style.features.get(name) {
            let label = if unit.is_empty() {
                label.to_string()
            } else {
                format!("{label} ({unit})")
            };
            l.push(format!(
                "| {label} | {:.2} | {:.2} - {:.2} |",
                p.mean, p.p10, p.p90
            ));
        }
    }
    let sections: Vec<(&str, Option<&str>, String)> = vec![
        (
            "Common words you use far more than general English",
            None,
            join(
                distinctive
                    .iter()
                    .map(|(w, c)| format!("{w} ({c})"))
                    .collect(),
            ),
        ),
        (
            "Your topic vocabulary",
            None,
            join(topical.iter().map(|(w, c)| format!("{w} ({c})")).collect()),
        ),
        (
            "Transitions you use (count, per 10k words)",
            None,
            join(
                trans
                    .iter()
                    .map(|(t, c)| format!("{t} ({c}, {:.1})", 1e4 * *c as f64 / total))
                    .collect(),
            ),
        ),
        (
            "Common sentence openers",
            None,
            join(
                openers
                    .iter()
                    .map(|(o, c)| format!("\"{o}\" ({c})"))
                    .collect(),
            ),
        ),
        (
            "Common English words you never use",
            Some("Each would be expected at least 5 times in a corpus your size."),
            join(avoided.iter().map(|w| w.to_string()).collect()),
        ),
        (
            "LLM-marker words you genuinely use (fine to keep)",
            None,
            join(
                used_markers
                    .iter()
                    .map(|w| format!("{w} ({})", m.count(w)))
                    .collect(),
            ),
        ),
        (
            "LLM-marker words you never use (avoid)",
            None,
            join(never.iter().map(|w| w.to_string()).collect()),
        ),
    ];
    for (title, note, body) in sections {
        l.push(String::new());
        l.push(format!("## {title}"));
        l.push(String::new());
        if let Some(n) = note {
            l.push(n.to_string());
            l.push(String::new());
        }
        l.push(body);
    }
    l.push(String::new());
    l.join("\n")
}
