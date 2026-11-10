//! voiceprint: let agents write like you.

mod check;
mod data;
mod model;
mod profile;
mod style;
mod text;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use crate::check::{annotate, load_markers, render_text, severity_rank, thousands, Checker};
use crate::model::{build, Doc, VoiceModel};
use crate::text::{draft_kind, is_word, iter_files, read_file, sentences, tokenize, BOS, EOS};

#[derive(Parser)]
#[command(
    name = "voiceprint",
    version,
    about = "Let agents write like you: a statistical model of your own writing."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args)]
struct IndexArg {
    /// Index file [default: $VOICEPRINT_INDEX or ~/.voiceprint/voice.vpi]
    #[arg(short, long)]
    index: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum FailOn {
    High,
    Medium,
    Low,
    Never,
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Md,
    Tex,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build an index from text you wrote (files or folders; @list.txt reads paths from a file)
    Build {
        /// Files or folders (.md .txt .docx .pdf .tex .rst .adoc)
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Where to write the index [default: $VOICEPRINT_INDEX or ~/.voiceprint/voice.vpi]
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// n-gram order
        #[arg(long, default_value_t = 4)]
        order: usize,
        /// Skip documents shorter than this many words
        #[arg(long, default_value_t = 100)]
        min_words: usize,
        /// Skip files whose name matches this glob (repeatable), e.g. --exclude 'draft*'
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,
        /// Write the cleaned text here, to inspect what the cleaner kept
        #[arg(long, value_name = "DIR")]
        dump_clean: Option<PathBuf>,
    },
    /// Flag what in a draft doesn't sound like you
    Check {
        /// Draft file, or - for stdin
        file: String,
        #[command(flatten)]
        index: IndexArg,
        /// Machine-readable output (for agents)
        #[arg(long)]
        json: bool,
        /// Print the draft with [[word -> alternatives]] inline
        #[arg(long)]
        annotate: bool,
        /// Markup to mask [default: from the file extension]
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        /// Custom LLM-marker list
        #[arg(long)]
        markers: Option<String>,
        /// Bits a replacement must gain to raise a context flag (3 = 8x likelier)
        #[arg(long, default_value_t = 3.0)]
        margin: f64,
        #[arg(long, default_value_t = 5)]
        suggestions: usize,
        #[arg(long, default_value_t = 60)]
        max_flags: usize,
        /// Exit 1 if any flag is at this severity or above (for hooks and CI)
        #[arg(long, value_enum, default_value_t = FailOn::Never)]
        fail_on: FailOn,
    },
    /// What would you put in this slot?
    Suggest {
        #[command(flatten)]
        index: IndexArg,
        /// Text before the slot
        #[arg(long)]
        left: String,
        /// Text after the slot
        #[arg(long, default_value = "")]
        right: String,
        /// The current word in the slot, to compare against
        #[arg(long)]
        word: Option<String>,
        #[arg(short, default_value_t = 10)]
        k: usize,
    },
    /// Autocomplete the next token from your writing
    Complete {
        prefix: String,
        #[command(flatten)]
        index: IndexArg,
        #[arg(short, default_value_t = 10)]
        k: usize,
    },
    /// Show how you use a word or phrase
    Kwic {
        phrase: String,
        #[command(flatten)]
        index: IndexArg,
        #[arg(short, default_value_t = 15)]
        n: usize,
        #[arg(long, default_value_t = 60)]
        width: usize,
    },
    /// Write a voice card (markdown) summarizing your style
    Profile {
        #[command(flatten)]
        index: IndexArg,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        markers: Option<String>,
    },
    /// Index metadata and calibration
    Info {
        #[command(flatten)]
        index: IndexArg,
    },
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        Some(rest) => home().join(rest),
        None if p == "~" => home(),
        None => PathBuf::from(p),
    }
}

fn default_index() -> PathBuf {
    std::env::var_os("VOICEPRINT_INDEX")
        .map(|p| expand_tilde(&p.to_string_lossy()))
        .unwrap_or_else(|| home().join(".voiceprint").join("voice.vpi"))
}

/// Expand `@file` arguments: one argument per line; blank lines and `#` comments skipped.
fn expand_args() -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (i, a) in std::env::args().enumerate() {
        match a.strip_prefix('@') {
            Some(path) if i > 0 => {
                let text = std::fs::read_to_string(expand_tilde(path))
                    .map_err(|e| format!("cannot read {path}: {e}"))?;
                out.extend(
                    text.lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty() && !l.starts_with('#'))
                        .map(String::from),
                );
            }
            _ => out.push(a),
        }
    }
    Ok(out)
}

fn load(index: &IndexArg) -> Result<VoiceModel, String> {
    let p = index.index.clone().unwrap_or_else(default_index);
    if !p.exists() {
        return Err(format!(
            "No index at {}. Build one first: voiceprint build <files or folders you wrote> -o {}",
            p.display(),
            p.display()
        ));
    }
    VoiceModel::load(&p)
}

fn run() -> Result<ExitCode, String> {
    let cli = Cli::parse_from(expand_args()?);
    match cli.cmd {
        Cmd::Build {
            paths,
            output,
            order,
            min_words,
            exclude,
            dump_clean,
        } => {
            if order < 2 {
                return Err("--order must be at least 2".into());
            }
            let paths: Vec<PathBuf> = paths
                .iter()
                .map(|p| expand_tilde(&p.to_string_lossy()))
                .collect();
            let patterns: Vec<glob::Pattern> = exclude
                .iter()
                .map(|g| glob::Pattern::new(g).map_err(|e| format!("bad --exclude {g}: {e}")))
                .collect::<Result<_, _>>()?;
            let mut docs = Vec::new();
            for f in iter_files(&paths) {
                let name = f
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if patterns.iter().any(|p| p.matches(&name)) {
                    continue;
                }
                let text = match read_file(&f) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("  skip {}: {e}", f.display());
                        continue;
                    }
                };
                let sents = sentences(&text);
                let n_words: usize = sents
                    .iter()
                    .map(|s| s.iter().filter(|t| is_word(&t.norm)).count())
                    .sum();
                if n_words < min_words {
                    eprintln!("  skip {}: only {n_words} words", f.display());
                    continue;
                }
                eprintln!(
                    "  read {}  ({} words)",
                    f.display(),
                    thousands(n_words as u64)
                );
                docs.push(Doc {
                    name: f.display().to_string(),
                    tokens: sents
                        .iter()
                        .map(|s| s.iter().map(|t| t.norm.clone()).collect())
                        .collect(),
                    surface: sents
                        .iter()
                        .map(|s| {
                            text[s[0].start..s.last().unwrap().end]
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .collect(),
                });
            }
            if docs.is_empty() {
                return Err("No usable documents found.".into());
            }
            if let Some(dir) = dump_clean {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                for (i, d) in docs.iter().enumerate() {
                    let p = Path::new(&d.name);
                    let parent = p
                        .parent()
                        .and_then(|x| x.file_name())
                        .map(|x| x.to_string_lossy())
                        .unwrap_or_default();
                    let stem = p
                        .file_stem()
                        .map(|x| x.to_string_lossy())
                        .unwrap_or_default();
                    let mut name = format!("{i:02}-{parent}-{stem}");
                    name.truncate(80);
                    std::fs::write(dir.join(format!("{name}.clean.txt")), d.surface.join("\n"))
                        .map_err(|e| e.to_string())?;
                }
            }
            let out = output
                .map(|p| expand_tilde(&p.to_string_lossy()))
                .unwrap_or_else(default_index);
            let m = build(docs, order);
            m.save(&out)?;
            println!(
                "Built {}: {} docs, {} words, vocab {}, order {}, {}s",
                out.display(),
                m.meta.documents,
                thousands(m.meta.words),
                thousands(m.meta.vocabulary as u64),
                order,
                m.meta.seconds
            );
            println!(
                "Calibration ({}): mean {:.2} bits/word, p95 {:.2}",
                m.meta.calibration,
                m.calib.word_surprisal_mean,
                m.calib.pct("95")
            );
            if m.meta.words < 30_000 {
                println!("Note: under ~30k words the model is thin; add more of your writing if you can.");
            }
        }
        Cmd::Check {
            file,
            index,
            json,
            annotate: ann,
            kind,
            markers,
            margin,
            suggestions,
            max_flags,
            fail_on,
        } => {
            let m = load(&index)?;
            let (text, name) = if file == "-" {
                let mut s = String::new();
                std::io::stdin()
                    .read_to_string(&mut s)
                    .map_err(|e| e.to_string())?;
                (s, "<stdin>".to_string())
            } else {
                let b = std::fs::read(&file).map_err(|e| format!("cannot read {file}: {e}"))?;
                (String::from_utf8_lossy(&b).into_owned(), file.clone())
            };
            let kind = match kind {
                Some(Kind::Tex) => "tex",
                Some(Kind::Md) => "md",
                None => draft_kind(if file == "-" { None } else { Some(&file) }),
            };
            let checker = Checker::new(&m, load_markers(markers.as_deref())?, margin, suggestions);
            let report = checker.check(&text, kind);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
                );
            } else if ann {
                println!("{}", annotate(&text, &report));
            } else {
                println!("{}", render_text(&report, &name, &m, max_flags));
            }
            let threshold = match fail_on {
                FailOn::High => severity_rank("high"),
                FailOn::Medium => severity_rank("medium"),
                FailOn::Low => severity_rank("low"),
                FailOn::Never => u8::MAX,
            };
            if report.worst_severity() >= threshold {
                return Ok(ExitCode::from(1));
            }
        }
        Cmd::Suggest {
            index,
            left,
            right,
            word,
            k,
        } => {
            let m = load(&index)?;
            let left: Vec<String> = tokenize(&left).into_iter().map(|t| t.norm).collect();
            let right: Vec<String> = tokenize(&right).into_iter().map(|t| t.norm).collect();
            let original = word
                .filter(|w| !w.is_empty())
                .map(|w| w.to_lowercase())
                .unwrap_or_else(|| "<blank>".into());
            let mut words: Vec<String> = vec![BOS.to_string(); m.order - 1];
            words.extend(left.iter().cloned());
            let pos = words.len();
            words.push(original);
            words.extend(right);
            words.push(EOS.to_string());
            let ids: Vec<_> = words.iter().map(|w| m.id(w)).collect();
            let checker = Checker::new(&m, load_markers(None)?, 3.0, k);
            for s in checker.suggest(&words, &ids, pos) {
                println!(
                    "{:<20} {:<8} {:+6.1} bits   seen in this frame {}x",
                    s.word, s.source, s.gain_bits, s.seen_in_this_frame
                );
            }
        }
        Cmd::Complete { prefix, index, k } => {
            let m = load(&index)?;
            let hist: Vec<_> = tokenize(&prefix)
                .into_iter()
                .map(|t| m.id(&t.norm))
                .collect();
            for (w, p) in m.continuations(&hist, k, false) {
                println!("{:<20} {p:.4}", m.word(w));
            }
        }
        Cmd::Kwic {
            phrase,
            index,
            n,
            width,
        } => {
            let m = load(&index)?;
            let pat = text::phrase_regex(&phrase, false);
            let mut hits = 0;
            let mut total = 0;
            for s in &m.sentences {
                let matches: Vec<_> = pat.find_iter(s).collect();
                total += matches.len();
                if hits < n {
                    if let Some(mt) = matches.first() {
                        let before: String = s[..mt.start()]
                            .chars()
                            .rev()
                            .take(width)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect();
                        let after: String = s[mt.end()..].chars().take(width).collect();
                        println!("{before:>width$} [{}] {after}", mt.as_str());
                        hits += 1;
                    }
                }
            }
            println!(
                "-- {total} occurrences in your corpus ({} as a single token)",
                m.count(&phrase.to_lowercase())
            );
        }
        Cmd::Profile {
            index,
            output,
            markers,
        } => {
            let m = load(&index)?;
            let card = profile::voice_card(&m, &load_markers(markers.as_deref())?);
            match output {
                Some(p) => {
                    let p = expand_tilde(&p.to_string_lossy());
                    if let Some(dir) = p.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&p, card).map_err(|e| e.to_string())?;
                    println!("Wrote {}", p.display());
                }
                None => println!("{card}"),
            }
        }
        Cmd::Info { index } => {
            let m = load(&index)?;
            let v = serde_json::json!({ "meta": m.meta, "calibration": m.calib });
            println!(
                "{}",
                serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("voiceprint: {e}");
            ExitCode::from(2)
        }
    }
}
