//! Score a draft against your voice model and flag what doesn't sound like you.
//!
//! Four kinds of flags:
//!
//! * `marker`    a word the LLM-marker list knows LLMs over-use, and that you rarely use
//! * `phrase`    the same, for multi-word phrases and constructions ("not only ... but also")
//! * `underused` a common English word you use at under 1/5 of its general-English rate,
//!               where the shortfall is significant for your corpus size (Poisson). Rare and
//!               technical words are never flagged this way, and it is never "high": new
//!               topics bring new words.
//! * `context`   a common word in your most surprising 5% of positions, where a synonym is
//!               much likelier under your model
//!
//! Replacement candidates are synonyms (curated map + WordNet) and words you have used in the
//! same left/right frame, restricted to words in your corpus and ranked by how much more
//! likely the local window becomes under your model (gain in bits; +3 means 8x more likely).

use std::cell::OnceCell;
use std::collections::{BTreeMap, HashMap};

use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};

use crate::data::{self, synonyms, zipf, Markers};
use crate::model::{Id, VoiceModel};
use crate::style::{self, round2, StyleRow};
use crate::text::{is_placeholder, is_word, mask_markup, sentences, tokenize, BOS, EOS};

/// Discourse fillers for which "just delete it" is a legitimate suggestion.
const DELETABLE: [&str; 23] = [
    "notably",
    "furthermore",
    "moreover",
    "additionally",
    "importantly",
    "ultimately",
    "essentially",
    "crucially",
    "remarkably",
    "seamlessly",
    "truly",
    "very",
    "really",
    "indeed",
    "arguably",
    "overall",
    "profoundly",
    "meticulously",
    "basically",
    "actually",
    "clearly",
    "simply",
    "significantly",
];
const FUNCTION_ZIPF: f64 = 6.0; // the/of/and/to/is...: governed by syntax, not style choice
const COMMON_ZIPF: f64 = 3.0; // below this a word is "rare" (often technical) in English
const UNDERUSE_RATIO: f64 = 0.2; // flag only if you use a word at < 1/5 of the English rate

pub fn severity_rank(s: &str) -> u8 {
    match s {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// P(X <= k) for X ~ Poisson(lam).
fn poisson_cdf(k: u32, lam: f64) -> f64 {
    let mut term = (-lam).exp();
    let mut total = term;
    for i in 1..=k {
        term *= lam / i as f64;
        total += term;
    }
    total.min(1.0)
}

#[derive(Serialize, Clone, Debug)]
pub struct Suggestion {
    pub word: String,
    pub source: &'static str,
    pub gain_bits: f64,
    pub seen_in_this_frame: u32,
}

#[derive(Serialize, Clone, Debug)]
pub struct Flag {
    pub kind: &'static str,
    pub severity: &'static str,
    pub text: String,
    /// Character offsets into the draft.
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub col: usize,
    pub reason: String,
    pub suggestions: Vec<Suggestion>,
    pub stats: Value,
    #[serde(skip)]
    pub byte_start: usize,
    #[serde(skip)]
    pub byte_end: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct Summary {
    pub words: usize,
    pub sentences: usize,
    pub mean_surprisal_bits: f64,
    pub your_mean_surprisal_bits: f64,
    pub share_above_your_p95: f64,
    pub your_share_above_p95: f64,
    pub flags: BTreeMap<&'static str, usize>,
    pub flags_per_100_words: f64,
    pub verdict: &'static str,
    pub short_draft: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Report {
    pub summary: Summary,
    pub style: Vec<StyleRow>,
    pub flags: Vec<Flag>,
}

impl Report {
    pub fn worst_severity(&self) -> u8 {
        self.flags
            .iter()
            .map(|f| severity_rank(f.severity))
            .max()
            .unwrap_or(0)
    }
}

struct Usage {
    your_count: u32,
    your_per_million: f64,
    english_per_million: f64,
    expected_count: f64,
}

pub struct Checker<'a> {
    m: &'a VoiceModel,
    markers: Markers,
    margin: f64,
    k: usize,
    corpus_lower: OnceCell<String>,
}

impl<'a> Checker<'a> {
    pub fn new(m: &'a VoiceModel, markers: Markers, margin: f64, k: usize) -> Self {
        Checker {
            m,
            markers,
            margin,
            k,
            corpus_lower: OnceCell::new(),
        }
    }

    fn corpus_count(&self, pattern: &Regex) -> usize {
        let corpus = self
            .corpus_lower
            .get_or_init(|| self.m.sentences.join("\n").to_lowercase().replace('’', "'"));
        pattern.find_iter(corpus).count()
    }

    fn usage(&self, w: &str) -> Usage {
        let c = self.m.count(w);
        let z = zipf(w);
        let total = self.m.total_words.max(1) as f64;
        Usage {
            your_count: c,
            your_per_million: round2(c as f64 / total * 1e6),
            english_per_million: if z > 0.0 {
                round2(10f64.powf(z - 3.0))
            } else {
                0.0
            },
            expected_count: if z > 0.0 {
                round2(10f64.powf(z - 9.0) * self.m.total_words as f64)
            } else {
                0.0
            },
        }
    }

    /// Replacement candidates for `words[pos]`, ranked by your model in context.
    ///
    /// Sources, in order of trust: `synonym` (curated map / WordNet, so meaning is roughly
    /// preserved), `frame` (words you have written between the same left and right word),
    /// and `delete` for discourse fillers. Every candidate must occur in your corpus twice.
    pub fn suggest(&self, words: &[String], ids: &[Id], pos: usize) -> Vec<Suggestion> {
        let m = self.m;
        let n = m.order;
        let original = words[pos].as_str();
        let hist = &ids[pos + 1 - n..pos];
        let right = &ids[pos + 1..(pos + n).min(ids.len())];
        let left1 = ids[pos - 1];
        let right1 = ids.get(pos + 1).copied().unwrap_or(m.eos());
        let empty = HashMap::new();
        let frame = m.slots.get(&(left1, right1)).unwrap_or(&empty);
        let mut seq = vec![ids[pos]];
        seq.extend_from_slice(right);
        let base = m.logprob_seq(hist, &seq);

        let score = |alt: &str, source: &'static str| -> Option<Suggestion> {
            let toks: Vec<String> = tokenize(alt).into_iter().map(|t| t.norm).collect();
            if toks.is_empty() || toks.iter().any(|t| !is_word(t) || m.count(t) < 2) {
                return None;
            }
            let mut s: Vec<Id> = toks.iter().map(|t| m.id(t)).collect();
            s.extend_from_slice(right);
            Some(Suggestion {
                word: alt.to_string(),
                source,
                gain_bits: round1(m.logprob_seq(hist, &s) - base),
                seen_in_this_frame: frame.get(&m.id(alt)).copied().unwrap_or(0),
            })
        };

        let mut syn: Vec<Suggestion> = synonyms(original)
            .iter()
            .filter_map(|a| score(a, "synonym"))
            .collect();
        let content = zipf(original) < FUNCTION_ZIPF;
        let mut top_frame: Vec<(Id, u32)> = frame.iter().map(|(a, b)| (*a, *b)).collect();
        top_frame.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| m.word(a.0).cmp(m.word(b.0))));
        let mut framed = Vec::new();
        for (wid, _) in top_frame.into_iter().take(30) {
            let w = m.word(wid);
            if w == original || is_placeholder(w) || (content && zipf(w) >= FUNCTION_ZIPF) {
                continue; // don't propose "the" for "crucial"
            }
            if let Some(s) = score(w, "frame") {
                if !syn.iter().any(|x| x.word == w) {
                    framed.push(s);
                }
            }
        }
        syn.sort_by(|a, b| b.gain_bits.total_cmp(&a.gain_bits));
        framed.sort_by(|a, b| {
            b.seen_in_this_frame
                .cmp(&a.seen_in_this_frame)
                .then_with(|| b.gain_bits.total_cmp(&a.gain_bits))
        });
        let mut out: Vec<Suggestion> = syn
            .into_iter()
            .take(self.k.saturating_sub(2).max(3))
            .collect();
        out.extend(framed.into_iter().take(2));

        if DELETABLE.contains(&original) {
            let comma = m.id(",");
            let skip_comma = right.first() == Some(&comma);
            let rest = if skip_comma { &right[1..] } else { right };
            let mut with_hist = hist.to_vec();
            with_hist.push(ids[pos]);
            if skip_comma {
                with_hist.push(comma);
            }
            let with_w = m.logprob_seq(&with_hist, rest);
            let without = m.logprob_seq(hist, rest);
            out.insert(
                0,
                Suggestion {
                    word: "(delete)".into(),
                    source: "delete",
                    gain_bits: round1(without - with_w),
                    seen_in_this_frame: 0,
                },
            );
        }
        out.truncate(self.k);
        out
    }

    pub fn check(&self, text: &str, kind: &str) -> Report {
        let m = self.m;
        let n = m.order;
        let masked = mask_markup(text, kind);
        let sents = sentences(&masked);
        let p95 = m.calib.pct("95");
        let mut flags: BTreeMap<(usize, usize), Flag> = BTreeMap::new();
        let mut surprisals: Vec<f64> = Vec::new();
        let mut token_sents: Vec<Vec<String>> = Vec::new();

        for sent in &sents {
            let norms: Vec<String> = sent.iter().map(|t| t.norm.clone()).collect();
            let mut words: Vec<String> = vec![BOS.to_string(); n - 1];
            words.extend(norms.iter().cloned());
            words.push(EOS.to_string());
            let ids: Vec<Id> = words.iter().map(|w| m.id(w)).collect();
            token_sents.push(norms);
            let mut seen_word = false;
            for (i, tok) in sent.iter().enumerate() {
                let w = tok.norm.as_str();
                if !is_word(w) {
                    continue;
                }
                let pos = i + n - 1;
                let s = m.surprisal(ids[pos], &ids[pos + 1 - n..pos]);
                surprisals.push(s);
                let initial = !seen_word;
                seen_word = true;
                let surf = tok.surface.as_str();
                let all_caps = surf.chars().count() > 1
                    && surf.chars().any(char::is_alphabetic)
                    && surf
                        .chars()
                        .filter(|c| c.is_alphabetic())
                        .all(char::is_uppercase);
                let proper =
                    (!initial && surf.chars().next().is_some_and(char::is_uppercase)) || all_caps;
                if is_placeholder(w)
                    || proper
                    || (w.chars().count() < 3 && !self.markers.words.contains(w))
                {
                    continue;
                }
                let u = self.usage(w);
                let z = zipf(w);
                let mut flag: Option<Flag> = None;
                let new_flag = |kind, severity, reason: String| Flag {
                    kind,
                    severity,
                    text: surf.to_string(),
                    start: 0,
                    end: 0,
                    line: 0,
                    col: 0,
                    reason,
                    suggestions: Vec::new(),
                    stats: Value::Null,
                    byte_start: tok.start,
                    byte_end: tok.end,
                };
                if self.markers.words.contains(w)
                    && (u.your_count < 3 || u.your_per_million < 2.0 * u.english_per_million)
                {
                    flag = Some(new_flag(
                        "marker",
                        if u.your_count == 0 { "high" } else { "medium" },
                        format!(
                            "LLM-favoured word; you used it {}x in {} words",
                            u.your_count,
                            thousands(m.total_words)
                        ),
                    ));
                } else if z >= COMMON_ZIPF
                    && u.expected_count >= 3.0
                    && (u.your_count as f64) < UNDERUSE_RATIO * u.expected_count
                {
                    let p = poisson_cdf(u.your_count, u.expected_count);
                    if p < 0.01 {
                        flag = Some(new_flag(
                            "underused",
                            if p < 0.001 { "medium" } else { "low" },
                            format!(
                                "you used it {}x; general English usage predicts ~{:.0}x in a corpus your size",
                                u.your_count, u.expected_count
                            ),
                        ));
                    }
                }
                if flag.is_none() && (COMMON_ZIPF..FUNCTION_ZIPF).contains(&z) && s > p95 {
                    let sug = self.suggest(&words, &ids, pos);
                    // Only a meaning-preserving alternative justifies a context flag;
                    // frame-only candidates would flag every new technical term.
                    if sug
                        .iter()
                        .any(|x| x.gain_bits >= self.margin && x.source != "frame")
                    {
                        let mut f = new_flag(
                            "context",
                            "low",
                            format!(
                                "unusual for you here ({s:.1} bits; your 95th pct is {p95:.1})"
                            ),
                        );
                        f.suggestions = sug;
                        flag = Some(f);
                    }
                }
                if let Some(mut f) = flag {
                    if f.suggestions.is_empty() {
                        f.suggestions = self.suggest(&words, &ids, pos);
                    }
                    f.stats = json!({
                        "your_count": u.your_count,
                        "your_per_million": u.your_per_million,
                        "english_per_million": u.english_per_million,
                        "expected_count": u.expected_count,
                        "surprisal_bits": round2(s),
                    });
                    flags.insert((tok.start, tok.end), f);
                }
            }
        }

        // Multi-word markers, matched on the masked draft.
        for (label, pat) in &self.markers.patterns {
            let mut yours: Option<usize> = None;
            for mt in pat.find_iter(&masked) {
                let y = *yours.get_or_insert_with(|| self.corpus_count(pat));
                if y >= 2 {
                    break;
                }
                flags.insert(
                    (mt.start(), mt.end()),
                    Flag {
                        kind: "phrase",
                        severity: if y == 0 { "high" } else { "medium" },
                        text: text[mt.start()..mt.end()].to_string(),
                        start: 0,
                        end: 0,
                        line: 0,
                        col: 0,
                        reason: format!("LLM-favoured construction ({label}); you used it {y}x"),
                        suggestions: Vec::new(),
                        stats: json!({ "your_count": y }),
                        byte_start: mt.start(),
                        byte_end: mt.end(),
                    },
                );
            }
        }

        // Word flags inside a flagged phrase are redundant; overlapping phrases keep the longest.
        let mut phrase_spans: Vec<(usize, usize)> = flags
            .values()
            .filter(|f| f.kind == "phrase")
            .map(|f| (f.byte_start, f.byte_end))
            .collect();
        phrase_spans.sort_by_key(|(s, e)| std::cmp::Reverse(e - s));
        let mut kept: Vec<(usize, usize)> = Vec::new();
        for (st, en) in phrase_spans {
            if kept.iter().any(|&(ks, ke)| st < ke && en > ks) {
                flags.remove(&(st, en));
            } else {
                kept.push((st, en));
            }
        }
        flags.retain(|_, f| {
            f.kind == "phrase"
                || !kept
                    .iter()
                    .any(|&(ks, ke)| ks <= f.byte_start && f.byte_end <= ke)
        });

        // Character-based positions for output.
        let char_at = |b: usize| text[..b].chars().count();
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(
                text.char_indices()
                    .filter(|(_, c)| *c == '\n')
                    .map(|(i, _)| i + 1),
            )
            .collect();
        let mut ordered: Vec<Flag> = flags.into_values().collect();
        ordered.sort_by_key(|f| f.byte_start);
        for f in &mut ordered {
            let li = line_starts.partition_point(|&s| s <= f.byte_start);
            f.line = li;
            f.col = text[line_starts[li - 1]..f.byte_start].chars().count() + 1;
            f.start = char_at(f.byte_start);
            f.end = char_at(f.byte_end);
        }

        let n_words = surprisals.len();
        let above = surprisals.iter().filter(|&&s| s > p95).count() as f64 / n_words.max(1) as f64;
        let mut sev: BTreeMap<&'static str, usize> = [("high", 0), ("medium", 0), ("low", 0)]
            .into_iter()
            .collect();
        for f in &ordered {
            *sev.entry(f.severity).or_default() += 1;
        }
        let style_rows = if token_sents.is_empty() {
            Vec::new()
        } else {
            style::compare(&style::features(&token_sents), &m.style)
        };
        let mean_s = surprisals.iter().sum::<f64>() / n_words.max(1) as f64;

        // Thresholds from held-out tests: your own unseen writing typically sits at 0-1 high
        // flags, 1-5 flags per 100 words, and 5-10% of words above p95.
        let per100 = 100.0 * ordered.len() as f64 / n_words.max(1) as f64;
        let style_off = style_rows.iter().filter(|r| r.off).count();
        let verdict = if sev["high"] >= 4 || per100 >= 8.0 || above >= 0.15 {
            "does not sound like you"
        } else if sev["high"] <= 1 && per100 <= 5.0 && above <= 0.12 && style_off <= 1 {
            "close to your voice"
        } else {
            "drifting"
        };

        Report {
            summary: Summary {
                words: n_words,
                sentences: sents.len(),
                mean_surprisal_bits: round2(mean_s),
                your_mean_surprisal_bits: round2(m.calib.word_surprisal_mean),
                share_above_your_p95: (above * 1000.0).round() / 1000.0,
                your_share_above_p95: 0.05,
                flags: sev,
                flags_per_100_words: round1(per100),
                verdict,
                short_draft: n_words < 150,
            },
            style: style_rows,
            flags: ordered,
        }
    }
}

pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ----------------------------------------------------------------------
// Rendering
// ----------------------------------------------------------------------

fn fmt_sugs(sugs: &[Suggestion]) -> String {
    let good: Vec<&Suggestion> = sugs.iter().filter(|s| s.gain_bits > 0.0).take(4).collect();
    if good.is_empty() {
        return "(no better fit in your corpus; rephrase)".into();
    }
    let f = |s: &&Suggestion| format!("{} ({:+.1})", s.word, s.gain_bits);
    let syn: Vec<String> = good.iter().filter(|s| s.source != "frame").map(f).collect();
    let frame: Vec<String> = good.iter().filter(|s| s.source == "frame").map(f).collect();
    let mut out = syn.join(", ");
    if !frame.is_empty() {
        out.push_str(if syn.is_empty() {
            "same slot: "
        } else {
            "   | same slot: "
        });
        out.push_str(&frame.join(", "));
    }
    out
}

pub fn render_text(r: &Report, name: &str, m: &VoiceModel, max_flags: usize) -> String {
    let s = &r.summary;
    let mut out = vec![
        format!("voiceprint check: {name}"),
        format!(
            "  index: {} words from {} documents",
            thousands(m.meta.words),
            m.meta.documents
        ),
        String::new(),
        "Voice".into(),
        format!("  verdict                   {}", s.verdict),
        format!(
            "  surprisal (bits/word)     {:.2}   yours on unseen text: {:.2}",
            s.mean_surprisal_bits, s.your_mean_surprisal_bits
        ),
        format!(
            "  words above your p95      {:.1}%   yours: 5%",
            100.0 * s.share_above_your_p95
        ),
        format!(
            "  flags                     {} high, {} medium, {} low  ({} per 100 words)",
            s.flags["high"], s.flags["medium"], s.flags["low"], s.flags_per_100_words
        ),
    ];
    if s.short_draft {
        out.push("  note: under 150 words, so style numbers are noisy".into());
    }
    out.push(String::new());
    out.push("Style (draft vs your typical p10-p90 range)".into());
    for row in &r.style {
        out.push(format!(
            "  {} {:<36} {:>7.2}   yours {:.2} (range {:.2}-{:.2}) {}",
            if row.off { "!" } else { " " },
            row.label,
            row.draft,
            row.yours_mean,
            row.yours_range[0],
            row.yours_range[1],
            row.unit
        ));
    }
    let mut shown: Vec<&Flag> = r.flags.iter().collect();
    shown.sort_by_key(|f| (std::cmp::Reverse(severity_rank(f.severity)), f.start));
    shown.truncate(max_flags);
    shown.sort_by_key(|f| f.start);
    out.push(String::new());
    out.push(format!("Flags ({})", r.flags.len()));
    for f in shown {
        let pos = format!("{}:{}", f.line, f.col);
        out.push(format!(
            "  {:<8} {:<6} {:<9} \"{}\"  {}",
            pos,
            f.severity.to_uppercase(),
            f.kind,
            f.text,
            f.reason
        ));
        if !f.suggestions.is_empty() {
            out.push(format!("  {:<22}-> {}", "", fmt_sugs(&f.suggestions)));
        }
    }
    if r.flags.len() > max_flags {
        out.push(format!(
            "  ... {} more (use --json or --max-flags)",
            r.flags.len() - max_flags
        ));
    }
    out.join("\n")
}

pub fn annotate(text: &str, r: &Report) -> String {
    let mut out = text.to_string();
    let mut flags: Vec<&Flag> = r.flags.iter().collect();
    flags.sort_by_key(|f| std::cmp::Reverse(f.byte_start));
    for f in flags {
        let alts: Vec<&str> = f
            .suggestions
            .iter()
            .filter(|s| s.gain_bits > 0.0)
            .take(3)
            .map(|s| s.word.as_str())
            .collect();
        let tag = format!(
            "[[{} -> {}]]",
            &out[f.byte_start..f.byte_end],
            if alts.is_empty() {
                "?".to_string()
            } else {
                alts.join(" | ")
            }
        );
        out.replace_range(f.byte_start..f.byte_end, &tag);
    }
    out
}

pub fn load_markers(path: Option<&str>) -> Result<Markers, String> {
    data::load_markers(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{build, Doc};
    use crate::text::sentences as split;

    fn model() -> VoiceModel {
        let subjects = [
            "the parser",
            "our method",
            "the agent",
            "this approach",
            "the runtime",
        ];
        let verbs = ["handles", "reduces", "checks", "tracks", "rewrites"];
        let objects = [
            "the input",
            "each request",
            "the error rate",
            "the queue",
            "every edge case",
        ];
        let tails = [
            "in practice.",
            "without extra state.",
            "at low cost.",
            "as expected.",
            "in most runs.",
        ];
        let docs = (0..6)
            .map(|d| {
                let mut text = String::new();
                for i in 0..120usize {
                    let k = i * 7 + d * 3;
                    let mut s = format!(
                        "{} {} {} {} ",
                        subjects[k % 5],
                        verbs[(k / 5) % 5],
                        objects[(k / 3) % 5],
                        tails[(k / 2) % 5]
                    );
                    if i % 3 == 0 {
                        s = format!("However, {s}");
                    }
                    let mut c = s.chars();
                    let first = c.next().unwrap().to_uppercase().collect::<String>();
                    text.push_str(&(first + c.as_str()));
                }
                let sents = split(&text);
                Doc {
                    name: format!("doc{d}"),
                    tokens: sents
                        .iter()
                        .map(|s| s.iter().map(|t| t.norm.clone()).collect())
                        .collect(),
                    surface: sents
                        .iter()
                        .map(|s| text[s[0].start..s.last().unwrap().end].to_string())
                        .collect(),
                }
            })
            .collect();
        build(docs, 3)
    }

    #[test]
    fn flags_llm_words_but_not_own_style() {
        let m = model();
        let ch = Checker::new(&m, load_markers(None).unwrap(), 3.0, 5);
        let own = ch.check("However, the parser checks the input in practice. Our method tracks the queue as expected.", "md");
        assert_eq!(own.summary.flags["high"], 0);
        let llm = ch.check(
            "Furthermore, our method seamlessly leverages a comprehensive approach that plays a crucial role.",
            "md",
        );
        let high: Vec<String> = llm
            .flags
            .iter()
            .filter(|f| f.severity == "high")
            .map(|f| f.text.to_lowercase())
            .collect();
        for w in ["furthermore", "seamlessly", "comprehensive"] {
            assert!(high.iter().any(|h| h == w), "{w} not flagged: {high:?}");
        }
    }

    #[test]
    fn offsets_are_characters() {
        let m = model();
        let ch = Checker::new(&m, load_markers(None).unwrap(), 3.0, 5);
        let text = "Ünïcödé first. Then it seamlessly works.";
        let r = ch.check(text, "md");
        let f = r
            .flags
            .iter()
            .find(|f| f.text == "seamlessly")
            .expect("flagged");
        let chars: Vec<char> = text.chars().collect();
        assert_eq!(
            chars[f.start..f.end].iter().collect::<String>(),
            "seamlessly"
        );
        assert!(annotate(text, &r).contains("[[seamlessly ->"));
    }
}
