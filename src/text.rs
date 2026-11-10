//! Reading, cleaning, and tokenizing text.
//!
//! Two paths through this module:
//!
//! * Corpus files (your own writing) go through `read_file` -> `clean_*`, which may
//!   rewrite the text freely (drop math, references, markup).
//! * Drafts go through `mask_markup`, which blanks markup out with spaces so every
//!   byte offset still points at the same place in the original draft. That is what
//!   lets `check` report line/column and annotate in place.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::data::zipf;

pub const BOS: &str = "<s>";
pub const EOS: &str = "</s>";
pub const PLACEHOLDERS: [&str; 7] = [
    "<cite>", "<ref>", "<math>", "<url>", "<email>", "<code>", "<num>",
];
pub const SUPPORTED: [&str; 8] = ["txt", "md", "markdown", "adoc", "tex", "rst", "pdf", "docx"];
pub const SENTENCE_END: [&str; 3] = [".", "?", "!"];

macro_rules! re {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| Regex::new($pat).expect($pat))
        }
    };
}

/// Case-insensitive regex for a literal phrase, bounded like `(?<!\w)phrase(?!\w)`.
/// Uses only the linear-time `regex` engine. `flex_apostrophe` matches ' and ’ alike.
pub fn phrase_regex(phrase: &str, flex_apostrophe: bool) -> Regex {
    let mut body = regex::escape(phrase);
    if flex_apostrophe {
        body = body.replace('\'', "['’]");
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let left = if word(phrase.chars().next()) {
        r"\b"
    } else {
        ""
    };
    let right = if word(phrase.chars().last()) {
        r"\b"
    } else {
        ""
    };
    Regex::new(&format!("(?i){left}{body}{right}")).expect("escaped phrase")
}

/// Apply `f` to every maximal run of word characters that is purely ASCII letters and at
/// least `min` long; the same as matching `(?<!\w)[A-Za-z]{min,}(?!\w)`.
fn map_ascii_words(s: &str, min: usize, mut f: impl FnMut(&str) -> String) -> String {
    static WORD_RUN: OnceLock<Regex> = OnceLock::new();
    WORD_RUN
        .get_or_init(|| Regex::new(r"\w+").unwrap())
        .replace_all(s, |c: &regex::Captures| {
            let w = &c[0];
            if w.len() >= min && w.bytes().all(|b| b.is_ascii_alphabetic()) {
                f(w)
            } else {
                w.to_string()
            }
        })
        .into_owned()
}

pub fn is_placeholder(s: &str) -> bool {
    PLACEHOLDERS.contains(&s)
}

pub fn is_word(norm: &str) -> bool {
    norm.chars().next().is_some_and(char::is_alphabetic)
}

#[derive(Debug, Clone)]
pub struct Token {
    pub norm: String,
    pub surface: String,
    /// Byte offsets into the text that was tokenized.
    pub start: usize,
    pub end: usize,
}

re!(
    token_re,
    concat!(
        r"(?i)<(?:cite|ref|math|url|email|code|num)>",
        r"|(?:e\.g\.|i\.e\.|et al\.|etc\.|vs\.|cf\.|figs?\.|eqs?\.|sec\.|no\.|approx\.|resp\.|dr\.|prof\.|mr\.|mrs\.|ms\.)",
        r"|[^\W\d_]+(?:['’][^\W\d_]+)*(?:-[^\W\d_]+)*",
        r"|\d+(?:[.,:]\d+)*(?:%|st|nd|rd|th|s)?",
        r"|\.\.\.|…|—|–|-{2,3}",
        r#"|[.,;:!?()\[\]"“”'‘’/&]"#,
    )
);

pub fn normalize(tok: &str) -> String {
    let t = tok.to_lowercase().replace('’', "'");
    if t.starts_with(|c: char| c.is_ascii_digit()) {
        return "<num>".into();
    }
    match t.as_str() {
        "--" | "---" => "—".into(),
        "…" => "...".into(),
        "“" | "”" => "\"".into(),
        "‘" | "’" => "'".into(),
        _ => t,
    }
}

pub fn tokenize(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    for m in token_re().find_iter(text) {
        let s = m.as_str();
        // "--" is a dash, but "--option" is not.
        if s.starts_with("--") {
            if let Some(c) = text[m.end()..].chars().next() {
                if c.is_alphanumeric() || c == '_' || c == '-' {
                    continue;
                }
            }
        }
        out.push(Token {
            norm: normalize(s),
            surface: s.to_string(),
            start: m.start(),
            end: m.end(),
        });
    }
    out
}

re!(paragraph_break, r"\n[ \t]*\n");

/// Split into sentences at . ? ! and at paragraph breaks.
pub fn sentences(text: &str) -> Vec<Vec<Token>> {
    let mut out: Vec<Vec<Token>> = Vec::new();
    let mut cur: Vec<Token> = Vec::new();
    let mut prev_end = 0;
    for tok in tokenize(text) {
        let gap = &text[prev_end..tok.start];
        if !cur.is_empty() && paragraph_break().is_match(gap) {
            out.push(std::mem::take(&mut cur));
        }
        let (start, end) = (tok.start, tok.end);
        // A closing paren/quote glued to the end of a sentence belongs to it.
        if cur.is_empty()
            && !out.is_empty()
            && tok.start == prev_end
            && matches!(tok.norm.as_str(), ")" | "\"" | "'")
        {
            out.last_mut().unwrap().push(tok);
        } else {
            let ends = SENTENCE_END.contains(&tok.norm.as_str());
            cur.push(tok);
            // "for .ini files" or "v2.5" must not end a sentence.
            let nxt = text[end..].chars().next();
            if ends && nxt.is_none_or(|c| c.is_whitespace() || "\")'”’".contains(c)) {
                out.push(std::mem::take(&mut cur));
            }
        }
        let _ = start;
        prev_end = end;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .filter(|s| s.iter().any(|t| is_word(&t.norm)))
        .collect()
}

// ----------------------------------------------------------------------------
// Corpus ingestion
// ----------------------------------------------------------------------------

pub fn iter_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            for e in walkdir::WalkDir::new(p)
                .sort_by_file_name()
                .into_iter()
                .flatten()
            {
                if e.file_type().is_file() && is_supported(e.path()) {
                    out.push(e.path().to_path_buf());
                }
            }
        } else if p.is_file() {
            out.push(p.clone());
        }
    }
    out
}

fn ext(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn is_supported(path: &Path) -> bool {
    SUPPORTED.contains(&ext(path).as_str())
}

pub fn read_file(path: &Path) -> Result<String, String> {
    match ext(path).as_str() {
        "pdf" => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            // pdf-extract prints font diagnostics to stdout/stderr and panics on some
            // malformed files: silence it while it runs, and treat a panic as unreadable.
            let res = {
                let _quiet = (gag::Gag::stdout().ok(), gag::Gag::stderr().ok());
                std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(&bytes))
            };
            match res {
                Ok(Ok(text)) => Ok(clean_pdf(&text)),
                Ok(Err(e)) => Err(format!("PDF: {e}")),
                Err(_) => Err("PDF could not be parsed".into()),
            }
        }
        "docx" => Ok(clean_plain(&read_docx(path)?)),
        e => {
            let raw = String::from_utf8_lossy(&std::fs::read(path).map_err(|e| e.to_string())?)
                .into_owned();
            Ok(match e {
                "tex" => clean_tex(&resolve_inputs(
                    &raw,
                    path.parent().unwrap_or(Path::new(".")),
                    0,
                )),
                "md" | "markdown" | "adoc" => clean_markdown(&raw),
                _ => clean_plain(&raw),
            })
        }
    }
}

re!(tex_input, r"\\(?:input|include)\{([^}]+)\}");
// `(?<!\\)%.*` without lookbehind: group 1 keeps the preceding character.
re!(tex_comment, r"(?m)(^|[^\\])%.*");

/// Inline \input{x} / \include{x} so a multi-file document reads as one.
/// Comments are stripped first, so commented-out inputs stay out.
pub fn resolve_inputs(tex: &str, base: &Path, depth: usize) -> String {
    let tex = tex_comment().replace_all(tex, "$1").into_owned();
    if depth > 5 {
        return tex;
    }
    tex_input()
        .replace_all(&tex, |c: &regex::Captures| {
            let mut f = base.join(c[1].trim());
            if f.extension().is_none() {
                f.set_extension("tex");
            }
            match std::fs::read(&f) {
                // LaTeX resolves nested inputs relative to the main file, not the includer.
                Ok(b) => resolve_inputs(&String::from_utf8_lossy(&b), base, depth + 1),
                Err(_) => " ".into(),
            }
        })
        .into_owned()
}

pub fn read_docx(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|e| e.to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    let xml = xml.replace("</w:p>", "\n\n").replace("<w:tab/>", " ");
    static TAG: OnceLock<Regex> = OnceLock::new();
    let text = TAG
        .get_or_init(|| Regex::new(r"<[^>]+>").unwrap())
        .replace_all(&xml, "");
    Ok(html_unescape(&text))
}

fn html_unescape(s: &str) -> String {
    static ENT: OnceLock<Regex> = OnceLock::new();
    ENT.get_or_init(|| Regex::new(r"&(#x[0-9a-fA-F]+|#\d+|amp|lt|gt|quot|apos|nbsp);").unwrap())
        .replace_all(s, |c: &regex::Captures| {
            let e = &c[1];
            let ch = match e {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                    .ok()
                    .and_then(char::from_u32),
                _ => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            };
            ch.map(String::from).unwrap_or_else(|| c[0].to_string())
        })
        .into_owned()
}

re!(url_re, r"https?://\S+|www\.\S+");
re!(email_re, r"\b[\w.+-]+@[\w-]+\.[\w.]+\b");
re!(cite_num, r"\[\d+(?:\s*[,–\-]\s*\d+)*\]");
re!(
    cite_author,
    r"\((?:[A-Z][\w'’\-]+(?: et al\.)?(?:,? (?:and|&) [A-Z][\w'’\-]+)?,? \d{4}[a-z]?(?:; ?)?)+\)"
);
re!(spaces, r"[ \t]+");

pub fn clean_plain(s: &str) -> String {
    let s: String = s.nfkc().collect(); // also unpacks PDF ligatures (ﬁ -> fi)
    let s = s.replace('\u{ad}', ""); // soft hyphen
    let s = url_re().replace_all(&s, " <url> ");
    let s = email_re().replace_all(&s, " <email> ");
    let s = cite_num().replace_all(&s, " <cite> ");
    let s = cite_author().replace_all(&s, " <cite> ");
    spaces().replace_all(&s, " ").trim().to_string()
}

re!(md_front, r"\A---\n(?s:.*?)\n---\n");
re!(md_fence, r"(?s)```.*?```");
re!(md_inline_code, r"`[^`\n]+`");
re!(md_image, r"!\[[^\]]*\]\([^)]*\)");
re!(md_link, r"\[([^\]]+)\]\([^)]*\)");
re!(md_html, r"<[^>\n]+>");
re!(md_table, r"(?m)^\s*\|.*$");
re!(
    md_rule,
    r"(?m)^\s*(?:={3,}|-{3,}|~{3,}|\*{3,}|_{3,}|#{3,}|\^{3,}|\+{3,})\s*$"
);
re!(md_heading, r"(?m)^\s{0,3}#{1,6}\s*(.*)$");
re!(md_list, r"(?m)^\s*(?:[-*+]|\d+\.)\s+");
fn md_emph() -> &'static [Regex; 4] {
    // `(\*\*|__|\*|_)(?=\S)(.+?)(?<=\S)\1`, one delimiter at a time (no backreferences).
    static R: OnceLock<[Regex; 4]> = OnceLock::new();
    R.get_or_init(|| {
        [
            r"\*\*(\S|\S.*?\S)\*\*",
            r"__(\S|\S.*?\S)__",
            r"\*(\S|\S.*?\S)\*",
            r"_(\S|\S.*?\S)_",
        ]
        .map(|p| Regex::new(p).unwrap())
    })
}

pub fn clean_markdown(s: &str) -> String {
    let s = md_front().replace(s, "");
    let s = md_fence().replace_all(&s, "\n\n");
    let s = md_inline_code().replace_all(&s, " <code> ");
    let s = md_image().replace_all(&s, " ");
    let s = md_link().replace_all(&s, "$1");
    let s = md_html().replace_all(&s, " ");
    let s = md_table().replace_all(&s, "");
    let s = md_rule().replace_all(&s, "\n\n");
    let s = md_heading().replace_all(&s, "\n\n$1\n\n");
    let s = md_list().replace_all(&s, "\n\n");
    let mut s = s.into_owned();
    for r in md_emph() {
        s = r.replace_all(&s, "$1").into_owned();
    }
    clean_plain(&s)
}

const NON_PROSE_ENVS: [&str; 18] = [
    "equation",
    "align",
    "gather",
    "multline",
    "eqnarray",
    "displaymath",
    "math",
    "figure",
    "table",
    "tabular",
    "tabularx",
    "algorithm",
    "algorithmic",
    "lstlisting",
    "verbatim",
    "minted",
    "tikzpicture",
    "thebibliography",
];

fn env_res() -> &'static Vec<Regex> {
    static R: OnceLock<Vec<Regex>> = OnceLock::new();
    R.get_or_init(|| {
        NON_PROSE_ENVS
            .iter()
            .map(|e| Regex::new(&format!(r"(?s)\\begin\{{{e}\*?\}}.*?\\end\{{{e}\*?\}}")).unwrap())
            .collect()
    })
}

re!(
    tex_body,
    r"(?s)\\begin\{document\}(.*?)(?:\\end\{document\}|\z)"
);
re!(
    tex_bib,
    r"\\bibliography\{|\\printbibliography|\\begin\{thebibliography\}"
);
re!(tex_display_math, r"(?s)\$\$.*?\$\$|\\\[.*?\\\]");
// `(?<!\\)\$.+?(?<!\\)\$` without lookbehind: group 1 keeps the preceding character.
re!(tex_inline_math, r"(?s)(^|[^\\])\$(?:\\.|[^\\$])+\$");
re!(tex_paren_math, r"(?s)\\\(.*?\\\)");
re!(
    tex_cite,
    r"~?\\[a-zA-Z]*cite[a-zA-Z]*\*?(?:\[[^\]]*\])*\{[^}]*\}"
);
re!(
    tex_ref,
    r"~?\\(?:ref|eqref|autoref|cref|Cref|pageref|nameref)\{[^}]*\}"
);
re!(tex_href, r"\\href\{[^}]*\}\{([^}]*)\}");
re!(
    tex_drop,
    r"\\(?:url|label|input|include|includegraphics)(?:\[[^\]]*\])?\{[^}]*\}"
);
re!(
    tex_section,
    r"\\(?:part|chapter|section|subsection|subsubsection|paragraph)\*?(?:\[[^\]]*\])?\{([^}]*)\}"
);
re!(tex_beginend, r"\\(?:begin|end)\{[^}]*\}(?:\[[^\]]*\])?");
re!(tex_item, r"\\item\b(?:\[[^\]]*\])?");
re!(tex_quotes, r"``|''");
re!(tex_unwrap, r"\\[a-zA-Z]+\*?(?:\[[^\]]*\])?\{([^{}]*)\}");
re!(tex_cmd, r"\\[a-zA-Z]+\*?");
re!(tex_escaped, r"\\(.)");

pub fn clean_tex(s: &str) -> String {
    let mut s = tex_comment().replace_all(s, "$1").into_owned();
    if let Some(c) = tex_body().captures(&s) {
        s = c[1].to_string();
    }
    if let Some(m) = tex_bib().find(&s) {
        s.truncate(m.start());
    }
    for r in env_res() {
        s = r.replace_all(&s, "\n\n").into_owned();
    }
    let s = tex_display_math().replace_all(&s, " <math> ");
    let s = tex_inline_math().replace_all(&s, "$1 <math> ");
    let s = tex_paren_math().replace_all(&s, " <math> ");
    let s = tex_cite().replace_all(&s, " <cite> ");
    let s = tex_ref().replace_all(&s, " <ref> ");
    let s = tex_href().replace_all(&s, "$1");
    let s = tex_drop().replace_all(&s, " ");
    let s = tex_section().replace_all(&s, "\n\n$1\n\n");
    let s = tex_beginend().replace_all(&s, "\n\n");
    let s = tex_item().replace_all(&s, "\n\n");
    let s = s.replace("---", "—").replace("--", "–");
    let mut s = tex_quotes().replace_all(&s, "\"").into_owned();
    for _ in 0..3 {
        s = tex_unwrap().replace_all(&s, "$1").into_owned();
    }
    let s = tex_cmd().replace_all(&s, " ");
    let s = tex_escaped().replace_all(&s, "$1");
    let s = s.replace(['{', '}'], "").replace('~', " ");
    clean_plain(&s)
}

re!(
    toc_line,
    r"^\s*(?:\d+\s+)?(?:\d+(?:\.\d+)+|Chapter\s+[IVXLC\d]+:?|[IVXLC]+\.)\s.*?(?:\s\.|\.{2,}|\s\d+)\s*$"
);
re!(pdf_hyphen, r"(\w)-\n([a-z])");
re!(
    pdf_refs,
    r"(?mi)^\s*(?:\d+\.?\s*)?(?:references|bibliography|works cited)\s*$"
);
re!(pdf_junk_line, r"^[\d\s\-–—|.]+$");
re!(pdf_terminal_punct, r"[.,;:?!]$");

pub fn clean_pdf(s: &str) -> String {
    let s = s.replace('\r', "");
    let s: Vec<&str> = s
        .split('\n')
        .filter(|ln| !toc_line().is_match(ln))
        .collect();
    let s = s.join("\n");
    let mut s = pdf_hyphen().replace_all(&s, "$1$2").into_owned(); // undo end-of-line hyphenation
                                                                   // Cut the reference list (last references heading in the back half).
    if let Some(m) = pdf_refs().find_iter(&s).last() {
        if m.start() as f64 > s.len() as f64 * 0.5 {
            s.truncate(m.start());
        }
    }
    let lines: Vec<&str> = s.split('\n').map(str::trim).collect();
    let mut out = String::with_capacity(s.len());
    for (i, ln) in lines.iter().enumerate() {
        if ln.is_empty() || pdf_junk_line().is_match(ln) {
            out.push_str("\n\n");
            continue;
        }
        let nxt = lines.get(i + 1).copied().unwrap_or("");
        let heading = ln.split_whitespace().count() <= 8
            && !pdf_terminal_punct().is_match(ln)
            && nxt.chars().next().is_some_and(char::is_uppercase);
        out.push_str(ln);
        out.push_str(if heading { "\n\n" } else { " " });
    }
    clean_plain(&fix_glued(&fix_ligatures(&out)))
}

/// Some PDFs drop the fi/fl ligature glyph, leaving a bare "f":
/// defined -> defned, first -> frst, flow -> fow.
pub fn fix_ligatures(s: &str) -> String {
    let mut cache: HashMap<String, String> = HashMap::new();
    map_ascii_words(s, 3, |w| {
        if let Some(v) = cache.get(w) {
            return v.clone();
        }
        let mut out = w.to_string();
        let z0 = zipf(w);
        if z0 < 3.0 {
            // common words are never ligature-damaged versions of others
            let mut best = z0 + 1.5;
            for (i, ch) in w.char_indices() {
                if ch != 'f' && ch != 'F' {
                    continue;
                }
                for ins in ["i", "l"] {
                    let cand = format!("{}{}{}", &w[..i + 1], ins, &w[i + 1..]);
                    let z = zipf(&cand);
                    if z >= 3.0 && z > best {
                        best = z;
                        out = cand;
                    }
                }
            }
        }
        cache.insert(w.to_string(), out.clone());
        out
    })
}

fn word_cost(w: &str) -> f64 {
    let z = zipf(w);
    // Single letters other than a/i are almost never right inside a glued run.
    if z <= 0.0 || (w.len() == 1 && w != "a" && w != "i") {
        25.0 + 2.0 * w.len() as f64
    } else {
        (9.0 - z) * 2.3
    }
}

/// "Iwouldalsoliketothank" -> "I would also like to thank" (Viterbi over word
/// frequencies). Returns the token unchanged unless the split is clearly better.
pub fn split_glued(token: &str) -> String {
    const MAX_WORD: usize = 20;
    let lower = token.to_ascii_lowercase();
    let n = lower.len();
    let mut best = vec![f64::INFINITY; n + 1];
    let mut back = vec![0usize; n + 1];
    best[0] = 0.0;
    for i in 1..=n {
        for j in i.saturating_sub(MAX_WORD)..i {
            let c = best[j] + word_cost(&lower[j..i]);
            if c < best[i] {
                best[i] = c;
                back[i] = j;
            }
        }
    }
    if best[n] >= word_cost(&lower) - 5.0 {
        return token.to_string();
    }
    let mut pieces = Vec::new();
    let mut i = n;
    while i > 0 {
        pieces.push(&token[back[i]..i]);
        i = back[i];
    }
    pieces.reverse();
    pieces.join(" ")
}

re!(camel_case, r"^(?:[A-Z][a-z]+){2,}$");

pub fn fix_glued(s: &str) -> String {
    map_ascii_words(s, 10, |w| {
        if zipf(w) > 0.0 || camel_case().is_match(w) {
            w.to_string() // a real word, or a CamelCase identifier like ObjectIntersectionOf
        } else {
            split_glued(w)
        }
    })
}

// ----------------------------------------------------------------------------
// Draft masking (offset-preserving)
// ----------------------------------------------------------------------------

fn blank(s: &str) -> String {
    // Keep newlines so line numbers and paragraph breaks survive; keep byte length.
    s.chars()
        .map(|c| {
            if c == '\n' {
                "\n".to_string()
            } else {
                " ".repeat(c.len_utf8())
            }
        })
        .collect()
}

fn blank_all(re: &Regex, s: &str) -> String {
    re.replace_all(s, |c: &regex::Captures| blank(&c[0]))
        .into_owned()
}

/// Blank the match except for capture group 1 at its start (the stand-in for a lookbehind).
fn blank_after_group1(re: &Regex, s: &str) -> String {
    re.replace_all(s, |c: &regex::Captures| {
        let keep = c.get(1).map_or("", |g| g.as_str());
        format!("{keep}{}", blank(&c[0][keep.len()..]))
    })
    .into_owned()
}

/// Blank emphasis markers: `**` and `__` always, single `*`/`_` unless between word chars.
fn blank_emphasis(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if (c == '*' || c == '_') && chars.get(i + 1) == Some(&c) {
            out.push_str("  ");
            i += 2;
            continue;
        }
        let between = i > 0 && word(chars.get(i - 1)) && word(chars.get(i + 1));
        out.push(if (c == '*' || c == '_') && !between {
            ' '
        } else {
            c
        });
        i += 1;
    }
    out
}

fn blank_keep_group1(re: &Regex, s: &str) -> String {
    re.replace_all(s, |c: &regex::Captures| {
        let whole = c.get(0).unwrap();
        let keep = c.get(1).unwrap();
        let (a, b) = (keep.start() - whole.start(), keep.end() - whole.start());
        let w = whole.as_str();
        format!("{}{}{}", blank(&w[..a]), &w[a..b], blank(&w[b..]))
    })
    .into_owned()
}

re!(mask_tex_math, r"(?s)\$\$.*?\$\$|\\\[.*?\\\]");
re!(
    mask_tex_citeref,
    r"~?\\[a-zA-Z]*(?:cite|ref)[a-zA-Z]*\*?(?:\[[^\]]*\])*\{[^}]*\}"
);
re!(
    mask_tex_drop,
    r"\\(?:begin|end|label|url|includegraphics)\*?(?:\[[^\]]*\])?\{[^}]*\}"
);
re!(mask_tex_rest, r"\\[a-zA-Z]+\*?|[{}~]");
re!(mask_md_url, r"https?://\S+");
re!(mask_md_heading, r"(?m)^\s{0,3}#{1,6}\s");

/// Blank out markup with spaces so byte offsets into the result match `text`.
pub fn mask_markup(text: &str, kind: &str) -> String {
    let mut s = text.to_string();
    if kind == "tex" {
        s = blank_after_group1(tex_comment(), &s);
        for r in env_res() {
            s = blank_all(r, &s);
        }
        s = blank_all(mask_tex_math(), &s);
        s = blank_after_group1(tex_inline_math(), &s);
        s = blank_all(mask_tex_citeref(), &s);
        s = blank_all(mask_tex_drop(), &s);
        s = blank_keep_group1(tex_unwrap(), &s);
        s = blank_all(mask_tex_rest(), &s);
    } else {
        s = blank_all(md_fence(), &s);
        s = blank_all(md_inline_code(), &s);
        s = blank_all(md_image(), &s);
        s = blank_keep_group1(md_link(), &s);
        s = blank_all(mask_md_url(), &s);
        s = blank_all(md_html(), &s);
        s = blank_all(md_table(), &s);
        s = blank_all(md_rule(), &s);
        s = blank_all(mask_md_heading(), &s);
        s = blank_all(md_list(), &s);
        s = blank_emphasis(&s);
    }
    s = blank_all(cite_num(), &s);
    debug_assert_eq!(s.len(), text.len());
    s
}

pub fn draft_kind(path: Option<&str>) -> &'static str {
    match path {
        Some(p) if p.to_lowercase().ends_with(".tex") => "tex",
        _ => "md",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_normalizes() {
        let toks: Vec<String> = tokenize("We don’t use 42 items, e.g. GPUs -- see --flag.")
            .into_iter()
            .map(|t| t.norm)
            .collect();
        assert!(toks.contains(&"don't".to_string()));
        assert!(toks.contains(&"<num>".to_string()));
        assert!(toks.contains(&"e.g.".to_string()));
        assert_eq!(toks.iter().filter(|t| *t == "—").count(), 1);
    }

    #[test]
    fn sentence_split_ignores_inner_dots() {
        assert_eq!(
            sentences(r#"A driver for ".ini" files was added. It works."#).len(),
            2
        );
    }

    #[test]
    fn mask_preserves_offsets() {
        let md =
            "# Title\n\nSome **bold** text with `code` and a [link](http://x.y). Ünïcödé — ok\n";
        let masked = mask_markup(md, "md");
        assert_eq!(masked.len(), md.len());
        assert!(masked.contains("bold") && !masked.contains("code") && !masked.contains("http"));
    }

    #[test]
    fn clean_tex_strips_markup() {
        let out = clean_tex(
            r"\begin{document}\section{Intro} We show \emph{this} \cite{a}. $x^2$ holds.\end{document}",
        );
        assert!(
            out.contains("Intro")
                && out.contains("this")
                && out.contains("<cite>")
                && out.contains("<math>")
        );
        assert!(!out.contains('\\'));
    }

    #[test]
    fn pdf_repairs() {
        assert_eq!(
            split_glued("Iwouldalsoliketothanktheexperts"),
            "I would also like to thank the experts"
        );
        assert_eq!(split_glued("phytoplankton"), "phytoplankton");
        assert_eq!(
            fix_ligatures("We defne a domain-specifc fle; keep form, life, of."),
            "We define a domain-specific file; keep form, life, of."
        );
    }
}
