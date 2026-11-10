//! A word-level interpolated Kneser-Ney n-gram model of one author's writing.
//!
//! Besides the n-gram tables the index keeps:
//!
//! * `slots`     (left word, right word) -> middle words, for fill-in-the-blank
//! * `prev`      right word -> words that precede it
//! * `unigram`   raw counts, for comparing your usage with general English
//! * `sentences` the cleaned corpus sentences, for keyword-in-context lookups
//! * `calib`     surprisal percentiles measured on held-out documents, so "unusual"
//!               means unusual relative to your own unseen writing
//! * `style`     per-chunk style feature distribution
//!
//! The index is stored as gzipped JSON with words interned to integer ids.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

use crate::style::{self, StyleProfile};
use crate::text::{is_word, BOS, EOS};

pub type Id = u32;
pub const UNK: Id = Id::MAX;
pub const FORMAT: &str = "voiceprint-index";
pub const FORMAT_VERSION: u32 = 2;

#[derive(Default)]
pub struct Entry {
    pub counts: HashMap<Id, u32>,
    pub total: u32,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Calib {
    pub word_surprisal_mean: f64,
    pub word_surprisal_pct: BTreeMap<String, f64>,
    pub sentence_mean: f64,
    pub sentence_sd: f64,
    pub sentence_pct: BTreeMap<String, f64>,
    pub held_out_words: usize,
}

impl Calib {
    pub fn pct(&self, p: &str) -> f64 {
        self.word_surprisal_pct.get(p).copied().unwrap_or(0.0)
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Meta {
    pub built: String,
    pub seconds: f64,
    pub documents: usize,
    pub sentences: usize,
    pub words: u64,
    pub vocabulary: usize,
    pub order: usize,
    pub calibration: String,
    pub voiceprint_version: String,
}

pub struct VoiceModel {
    pub order: usize,
    pub vocab: Vec<String>,
    pub ids: HashMap<String, Id>,
    /// tables[k] for k in 1..=order (index 0 unused). Order `order` holds raw counts;
    /// lower orders hold Kneser-Ney continuation counts.
    pub tables: Vec<HashMap<Vec<Id>, Entry>>,
    pub discounts: Vec<f64>,
    pub v: usize,
    pub unigram: HashMap<Id, u32>,
    pub total_words: u64,
    pub slots: HashMap<(Id, Id), HashMap<Id, u32>>,
    pub prev: HashMap<Id, HashMap<Id, u32>>,
    pub sentences: Vec<String>,
    pub sentence_doc: Vec<u32>,
    pub docs: Vec<String>,
    pub calib: Calib,
    pub style: StyleProfile,
    pub meta: Meta,
}

impl VoiceModel {
    pub fn new(order: usize) -> Self {
        assert!(order >= 2, "order must be >= 2");
        let mut m = VoiceModel {
            order,
            vocab: Vec::new(),
            ids: HashMap::new(),
            tables: (0..=order).map(|_| HashMap::new()).collect(),
            discounts: vec![0.75; order + 1],
            v: 1,
            unigram: HashMap::new(),
            total_words: 0,
            slots: HashMap::new(),
            prev: HashMap::new(),
            sentences: Vec::new(),
            sentence_doc: Vec::new(),
            docs: Vec::new(),
            calib: Calib::default(),
            style: StyleProfile::default(),
            meta: Meta::default(),
        };
        m.intern(BOS);
        m.intern(EOS);
        m
    }

    fn intern(&mut self, w: &str) -> Id {
        if let Some(&id) = self.ids.get(w) {
            return id;
        }
        let id = self.vocab.len() as Id;
        self.vocab.push(w.to_string());
        self.ids.insert(w.to_string(), id);
        id
    }

    pub fn id(&self, w: &str) -> Id {
        self.ids.get(w).copied().unwrap_or(UNK)
    }

    pub fn word(&self, id: Id) -> &str {
        self.vocab
            .get(id as usize)
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn bos(&self) -> Id {
        0
    }

    pub fn eos(&self) -> Id {
        1
    }

    // ------------------------------------------------------------------
    // Training
    // ------------------------------------------------------------------
    pub fn fit(&mut self, sents: &[Vec<String>]) {
        let n = self.order;
        let mut top: HashMap<Vec<Id>, HashMap<Id, u32>> = HashMap::new();
        for s in sents {
            let mut seq: Vec<Id> = vec![self.bos(); n - 1];
            for t in s {
                let id = self.intern(t);
                seq.push(id);
            }
            seq.push(self.eos());
            for i in n - 1..seq.len() {
                *top.entry(seq[i + 1 - n..i].to_vec())
                    .or_default()
                    .entry(seq[i])
                    .or_default() += 1;
            }
            for i in n - 1..seq.len() - 1 {
                let t = seq[i];
                *self.unigram.entry(t).or_default() += 1;
                *self
                    .slots
                    .entry((seq[i - 1], seq[i + 1]))
                    .or_default()
                    .entry(t)
                    .or_default() += 1;
                *self
                    .prev
                    .entry(seq[i + 1])
                    .or_default()
                    .entry(t)
                    .or_default() += 1;
            }
        }
        self.tables[n] = to_entries(top);
        // Continuation counts: how many distinct words precede (ctx, w).
        for k in (1..n).rev() {
            let mut cont: HashMap<Vec<Id>, HashMap<Id, u32>> = HashMap::new();
            for (ctx, e) in &self.tables[k + 1] {
                let sub = ctx[1..].to_vec();
                let slot = cont.entry(sub).or_default();
                for w in e.counts.keys() {
                    *slot.entry(*w).or_default() += 1;
                }
            }
            self.tables[k] = to_entries(cont);
        }
        self.finish();
    }

    /// Discounts, vocabulary size and word total, from the tables.
    fn finish(&mut self) {
        for k in 1..=self.order {
            let (mut n1, mut n2) = (0u64, 0u64);
            for e in self.tables[k].values() {
                for &c in e.counts.values() {
                    if c == 1 {
                        n1 += 1;
                    } else if c == 2 {
                        n2 += 1;
                    }
                }
            }
            let d = if n1 + n2 > 0 {
                n1 as f64 / (n1 + 2 * n2) as f64
            } else {
                0.75
            };
            self.discounts[k] = d.clamp(0.1, 0.95);
        }
        self.v = self.tables[1]
            .get(&Vec::new())
            .map(|e| e.counts.len())
            .unwrap_or(0)
            + 1;
        self.total_words = self
            .unigram
            .iter()
            .filter(|(id, _)| is_word(self.word(**id)))
            .map(|(_, c)| *c as u64)
            .sum();
    }

    // ------------------------------------------------------------------
    // Probabilities
    // ------------------------------------------------------------------
    pub fn pad(&self, hist: &[Id]) -> Vec<Id> {
        let n = self.order - 1;
        let tail = &hist[hist.len().saturating_sub(n)..];
        let mut h = vec![self.bos(); n - tail.len()];
        h.extend_from_slice(tail);
        h
    }

    fn p(&self, w: Id, hist: &[Id]) -> f64 {
        let k = hist.len() + 1;
        if k == 1 {
            let Some(e) = self.tables[1].get(hist) else {
                return 1.0 / self.v as f64;
            };
            let d = self.discounts[1];
            let c = e.counts.get(&w).copied().unwrap_or(0) as f64;
            let total = e.total as f64;
            return (c - d).max(0.0) / total + d * e.counts.len() as f64 / total / self.v as f64;
        }
        let lower = self.p(w, &hist[1..]);
        let Some(e) = self.tables[k].get(hist) else {
            return lower;
        };
        let d = self.discounts[k];
        let c = e.counts.get(&w).copied().unwrap_or(0) as f64;
        let total = e.total as f64;
        (c - d).max(0.0) / total + d * e.counts.len() as f64 / total * lower
    }

    pub fn prob(&self, w: Id, hist: &[Id]) -> f64 {
        self.p(w, &self.pad(hist))
    }

    /// Bits of surprise for `w` after `hist` (higher = less like you).
    pub fn surprisal(&self, w: Id, hist: &[Id]) -> f64 {
        -self.prob(w, hist).log2()
    }

    pub fn logprob_seq(&self, hist: &[Id], seq: &[Id]) -> f64 {
        let mut h = self.pad(hist);
        let n = self.order - 1;
        let mut lp = 0.0;
        for &t in seq {
            lp += self.p(t, &h[h.len() - n..]).log2();
            h.push(t);
        }
        lp
    }

    /// Most likely next tokens after `hist` (autocomplete).
    pub fn continuations(&self, hist: &[Id], k: usize, words_only: bool) -> Vec<(Id, f64)> {
        let h = self.pad(hist);
        let mut cands: HashSet<Id> = HashSet::new();
        for j in 0..=h.len() {
            if let Some(e) = self.tables[h.len() - j + 1].get(&h[j..]) {
                cands.extend(e.counts.keys().copied());
            }
            if cands.len() >= 200 {
                break;
            }
        }
        let mut scored: Vec<(Id, f64)> = cands
            .into_iter()
            .filter(|&w| !words_only || is_word(self.word(w)))
            .map(|w| (w, self.p(w, &h)))
            .collect();
        scored.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| self.word(a.0).cmp(self.word(b.0)))
        });
        scored.truncate(k);
        scored
    }

    pub fn count(&self, w: &str) -> u32 {
        self.ids
            .get(w)
            .and_then(|id| self.unigram.get(id))
            .copied()
            .unwrap_or(0)
    }

    // ------------------------------------------------------------------
    // Persistence: gzipped JSON with interned ids
    // ------------------------------------------------------------------
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let file = IndexFile {
            format: FORMAT.into(),
            version: FORMAT_VERSION,
            order: self.order,
            vocab: self.vocab.clone(),
            tables: (1..=self.order)
                .map(|k| {
                    let mut rows: Vec<(Vec<Id>, Vec<(Id, u32)>)> = self.tables[k]
                        .iter()
                        .map(|(ctx, e)| (ctx.clone(), sorted_pairs(&e.counts)))
                        .collect();
                    rows.sort();
                    (k.to_string(), rows)
                })
                .collect(),
            unigram: sorted_pairs(&self.unigram),
            slots: {
                let mut v: Vec<(Id, Id, Vec<(Id, u32)>)> = self
                    .slots
                    .iter()
                    .map(|((a, b), m)| (*a, *b, sorted_pairs(m)))
                    .collect();
                v.sort();
                v
            },
            prev: {
                let mut v: Vec<(Id, Vec<(Id, u32)>)> = self
                    .prev
                    .iter()
                    .map(|(r, m)| (*r, sorted_pairs(m)))
                    .collect();
                v.sort();
                v
            },
            sentences: self.sentences.clone(),
            sentence_doc: self.sentence_doc.clone(),
            docs: self.docs.clone(),
            calib: self.calib.clone(),
            style: self.style.clone(),
            meta: self.meta.clone(),
        };
        let f = std::fs::File::create(path)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        let mut gz = GzEncoder::new(f, Compression::new(6));
        serde_json::to_writer(&mut gz, &file).map_err(|e| e.to_string())?;
        gz.finish()
            .map_err(|e| e.to_string())?
            .flush()
            .map_err(|e| e.to_string())
    }

    pub fn load(path: &Path) -> Result<VoiceModel, String> {
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut raw = Vec::new();
        GzDecoder::new(bytes.as_slice())
            .read_to_end(&mut raw)
            .map_err(|_| format!("{} is not a voiceprint index", path.display()))?;
        if raw.first() == Some(&0x80) {
            return Err(format!(
                "{} was built by the old Python voiceprint (pickle format). Rebuild it with `voiceprint build`.",
                path.display()
            ));
        }
        let file: IndexFile = serde_json::from_slice(&raw)
            .map_err(|e| format!("{} is not a voiceprint index: {e}", path.display()))?;
        if file.format != FORMAT || file.version != FORMAT_VERSION {
            return Err(format!(
                "{} uses index format {} v{}; this voiceprint reads v{FORMAT_VERSION}. Rebuild it.",
                path.display(),
                file.format,
                file.version
            ));
        }
        let mut m = VoiceModel::new(file.order);
        m.vocab = file.vocab;
        m.ids = m
            .vocab
            .iter()
            .enumerate()
            .map(|(i, w)| (w.clone(), i as Id))
            .collect();
        for (k, rows) in file.tables {
            let k: usize = k.parse().map_err(|_| "bad table order".to_string())?;
            if k == 0 || k > m.order {
                return Err("bad table order".into());
            }
            m.tables[k] = rows
                .into_iter()
                .map(|(ctx, pairs)| {
                    let total = pairs.iter().map(|p| p.1).sum();
                    (
                        ctx,
                        Entry {
                            counts: pairs.into_iter().collect(),
                            total,
                        },
                    )
                })
                .collect();
        }
        m.unigram = file.unigram.into_iter().collect();
        m.slots = file
            .slots
            .into_iter()
            .map(|(a, b, p)| ((a, b), p.into_iter().collect()))
            .collect();
        m.prev = file
            .prev
            .into_iter()
            .map(|(r, p)| (r, p.into_iter().collect()))
            .collect();
        m.sentences = file.sentences;
        m.sentence_doc = file.sentence_doc;
        m.docs = file.docs;
        m.calib = file.calib;
        m.style = file.style;
        m.meta = file.meta;
        m.finish();
        Ok(m)
    }
}

fn to_entries(m: HashMap<Vec<Id>, HashMap<Id, u32>>) -> HashMap<Vec<Id>, Entry> {
    m.into_iter()
        .map(|(ctx, counts)| {
            let total = counts.values().sum();
            (ctx, Entry { counts, total })
        })
        .collect()
}

fn sorted_pairs(m: &HashMap<Id, u32>) -> Vec<(Id, u32)> {
    let mut v: Vec<(Id, u32)> = m.iter().map(|(a, b)| (*a, *b)).collect();
    v.sort_unstable();
    v
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    format: String,
    version: u32,
    order: usize,
    vocab: Vec<String>,
    tables: BTreeMap<String, Vec<(Vec<Id>, Vec<(Id, u32)>)>>,
    unigram: Vec<(Id, u32)>,
    slots: Vec<(Id, Id, Vec<(Id, u32)>)>,
    prev: Vec<(Id, Vec<(Id, u32)>)>,
    sentences: Vec<String>,
    sentence_doc: Vec<u32>,
    docs: Vec<String>,
    calib: Calib,
    style: StyleProfile,
    meta: Meta,
}

// ----------------------------------------------------------------------
// Building an index from documents
// ----------------------------------------------------------------------

/// One document: its name, token sentences (normalized), and surface sentences.
pub struct Doc {
    pub name: String,
    pub tokens: Vec<Vec<String>>,
    pub surface: Vec<String>,
}

fn percentiles(vals: &mut [f64]) -> BTreeMap<String, f64> {
    vals.sort_by(f64::total_cmp);
    [50, 75, 90, 95, 99]
        .iter()
        .map(|&p| {
            let v = if vals.is_empty() {
                0.0
            } else {
                vals[((p as f64 / 100.0 * vals.len() as f64) as usize).min(vals.len() - 1)]
            };
            (p.to_string(), v)
        })
        .collect()
}

/// Score each held-out fold with a model trained on the other folds.
pub fn calibrate(
    folds: &[(Vec<&Vec<Vec<String>>>, Vec<&Vec<Vec<String>>>)],
    order: usize,
) -> Calib {
    let mut word_s = Vec::new();
    let mut sent_means = Vec::new();
    for (train, held) in folds {
        let mut m = VoiceModel::new(order);
        let sents: Vec<Vec<String>> = train.iter().flat_map(|d| d.iter().cloned()).collect();
        m.fit(&sents);
        for d in held {
            for s in d.iter() {
                let mut seq: Vec<Id> = vec![m.bos(); order - 1];
                seq.extend(s.iter().map(|t| m.id(t)));
                let vals: Vec<f64> = s
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| is_word(t))
                    .map(|(i, _)| m.surprisal(seq[i + order - 1], &seq[i..i + order - 1]))
                    .collect();
                if !vals.is_empty() {
                    sent_means.push(vals.iter().sum::<f64>() / vals.len() as f64);
                    word_s.extend(vals);
                }
            }
        }
    }
    let mean = word_s.iter().sum::<f64>() / word_s.len().max(1) as f64;
    let sm = sent_means.iter().sum::<f64>() / sent_means.len().max(1) as f64;
    let var = sent_means.iter().map(|x| (x - sm).powi(2)).sum::<f64>()
        / (sent_means.len().max(2) - 1) as f64;
    let held_out_words = word_s.len();
    Calib {
        word_surprisal_mean: mean,
        word_surprisal_pct: percentiles(&mut word_s),
        sentence_mean: sm,
        sentence_sd: var.sqrt(),
        sentence_pct: percentiles(&mut sent_means),
        held_out_words,
    }
}

pub fn build(docs: Vec<Doc>, order: usize) -> VoiceModel {
    let t0 = Instant::now();
    // Cross-validate by document (up to 5 folds) so every document is scored by a
    // model that never saw it. A single document falls back to an 80/20 split.
    let k = docs.len().min(5);
    let (calib, how) = if k >= 2 {
        let folds: Vec<_> = (0..k)
            .map(|f| {
                let train = docs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| i % k != f)
                    .map(|(_, d)| &d.tokens)
                    .collect();
                let held = docs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| i % k == f)
                    .map(|(_, d)| &d.tokens)
                    .collect();
                (train, held)
            })
            .collect();
        (
            calibrate(&folds, order),
            format!("{k}-fold cross-validation by document"),
        )
    } else {
        let d = &docs[0].tokens;
        let cut = d.len() * 8 / 10;
        let (a, b) = (d[..cut].to_vec(), d[cut..].to_vec());
        (
            calibrate(&[(vec![&a], vec![&b])], order),
            "held-out tail of the only document".to_string(),
        )
    };

    let mut m = VoiceModel::new(order);
    let all: Vec<Vec<String>> = docs.iter().flat_map(|d| d.tokens.iter().cloned()).collect();
    m.fit(&all);
    m.calib = calib;
    m.style = style::chunked_profile(
        &docs.iter().map(|d| d.tokens.clone()).collect::<Vec<_>>(),
        300,
    );
    for (i, d) in docs.into_iter().enumerate() {
        m.docs.push(d.name);
        m.sentence_doc
            .extend(std::iter::repeat_n(i as u32, d.surface.len()));
        m.sentences.extend(d.surface);
    }
    m.meta = Meta {
        built: now_string(),
        seconds: (t0.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        documents: m.docs.len(),
        sentences: m.sentences.len(),
        words: m.total_words,
        vocabulary: m.v - 1,
        order,
        calibration: how,
        voiceprint_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    m
}

fn now_string() -> String {
    // UTC, without pulling in a date crate.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let (h, mi) = ((secs % 86_400) / 3600, (secs % 3600) / 60);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Vec<Vec<String>> {
        let s = [
            "the parser checks the input",
            "our method tracks the queue",
            "the agent rewrites each request",
        ];
        (0..30)
            .map(|i| s[i % 3].split(' ').map(String::from).collect())
            .collect()
    }

    #[test]
    fn kn_is_a_distribution() {
        let mut m = VoiceModel::new(3);
        m.fit(&corpus());
        let vocab: Vec<Id> = m.tables[1][&Vec::new()].counts.keys().copied().collect();
        for hist in [
            vec![m.bos(), m.bos()],
            vec![m.id("the"), m.id("parser")],
            vec![UNK, UNK],
        ] {
            let total: f64 =
                vocab.iter().map(|&w| m.prob(w, &hist)).sum::<f64>() + m.prob(UNK, &hist);
            assert!((total - 1.0).abs() < 1e-9, "{total}");
        }
    }

    #[test]
    fn save_load_roundtrip() {
        let docs: Vec<Doc> = (0..3)
            .map(|i| Doc {
                name: format!("d{i}"),
                tokens: corpus(),
                surface: vec!["x".into(); 30],
            })
            .collect();
        let m = build(docs, 3);
        let dir = std::env::temp_dir().join(format!("vp-test-{}", std::process::id()));
        let p = dir.join("v.vpi");
        m.save(&p).unwrap();
        let m2 = VoiceModel::load(&p).unwrap();
        let h = [m.bos(), m.id("the")];
        let h2 = [m2.bos(), m2.id("the")];
        assert!((m.prob(m.id("parser"), &h) - m2.prob(m2.id("parser"), &h2)).abs() < 1e-12);
        let _ = std::fs::remove_dir_all(dir);
    }
}
