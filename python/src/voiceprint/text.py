"""Reading, cleaning, and tokenizing text.

Two paths through this module:

* Corpus files (your own writing) go through ``read_file`` -> ``clean_*``, which is
  allowed to rewrite the text freely (drop equations, references, markup).
* Drafts go through ``mask_markup``, which blanks markup out with spaces so
  every character offset still points at the same place in the original
  draft. That is what lets ``check`` report line/column and annotate in place.
"""
from __future__ import annotations

import html
import re
import unicodedata
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Iterator

SUPPORTED = {".txt", ".md", ".markdown", ".adoc", ".tex", ".rst", ".pdf", ".docx"}

# Placeholder tokens that stand in for content we never want to model as words.
PLACEHOLDERS = {"<cite>", "<ref>", "<math>", "<url>", "<email>", "<code>", "<num>"}
BOS, EOS = "<s>", "</s>"

ABBREVIATIONS = r"e\.g\.|i\.e\.|et\ al\.|etc\.|vs\.|cf\.|figs?\.|eqs?\.|sec\.|no\.|approx\.|resp\.|dr\.|prof\.|mr\.|mrs\.|ms\."

TOKEN_RE = re.compile(
    rf"""
      <(?:cite|ref|math|url|email|code|num)>
    | (?:{ABBREVIATIONS})
    | [^\W\d_]+(?:['’][^\W\d_]+)*(?:-[^\W\d_]+)*     # words, contractions, hyphenated compounds
    | \d+(?:[.,:]\d+)*(?:%|st|nd|rd|th|s)?           # numbers
    | \.\.\.|…|—|–|-{{2,3}}(?![\w-])                 # "--" as a dash, but not "--option"
    | [.,;:!?()\[\]"“”'‘’/&]
    """,
    re.X | re.I,
)

SENTENCE_END = {".", "?", "!"}
PARAGRAPH_BREAK = re.compile(r"\n[ \t]*\n")


@dataclass(frozen=True)
class Token:
    norm: str      # normalized form used by the model
    surface: str   # exactly as written
    start: int
    end: int


def normalize(tok: str) -> str:
    t = tok.lower().replace("’", "'")
    if t[0].isdigit():
        return "<num>"
    if t in ("--", "---"):
        return "—"
    if t == "…":
        return "..."
    if t in ("“", "”"):
        return '"'
    if t in ("‘", "’"):
        return "'"
    return t


def is_word(norm: str) -> bool:
    return norm[:1].isalpha()


def tokenize(text: str) -> list[Token]:
    return [Token(normalize(m.group()), m.group(), m.start(), m.end()) for m in TOKEN_RE.finditer(text)]


def sentences(text: str) -> list[list[Token]]:
    """Split into sentences at . ? ! and at paragraph breaks."""
    out: list[list[Token]] = []
    cur: list[Token] = []
    prev_end = 0
    for tok in tokenize(text):
        gap = text[prev_end:tok.start]
        if cur and PARAGRAPH_BREAK.search(gap):
            out.append(cur)
            cur = []
        # A closing paren/quote glued to the end of a sentence belongs to it.
        if not cur and out and tok.start == prev_end and tok.norm in {")", '"', "'"}:
            out[-1].append(tok)
        else:
            cur.append(tok)
            nxt = text[tok.end:tok.end + 1]
            # "for .ini files" or "v2.5" must not end a sentence.
            if tok.norm in SENTENCE_END and (not nxt or nxt.isspace() or nxt in "\")'”’"):
                out.append(cur)
                cur = []
        prev_end = tok.end
    if cur:
        out.append(cur)
    return [s for s in out if any(is_word(t.norm) for t in s)]


# --------------------------------------------------------------------------
# Corpus ingestion
# --------------------------------------------------------------------------

def iter_files(paths: Iterable[str | Path]) -> Iterator[Path]:
    for p in map(Path, paths):
        if p.is_dir():
            for f in sorted(p.rglob("*")):
                if f.is_file() and f.suffix.lower() in SUPPORTED:
                    yield f
        elif p.is_file():
            yield p


def read_file(path: Path) -> str:
    ext = path.suffix.lower()
    if ext == ".pdf":
        import logging

        from pypdf import PdfReader

        logging.getLogger("pypdf").setLevel(logging.ERROR)

        reader = PdfReader(str(path))
        return clean_pdf("\n".join(page.extract_text() or "" for page in reader.pages))
    if ext == ".docx":
        return clean_plain(read_docx(path))
    raw = path.read_text(encoding="utf-8", errors="ignore")
    if ext == ".tex":
        return clean_tex(resolve_inputs(raw, path.parent))
    if ext in (".md", ".markdown", ".adoc"):
        return clean_markdown(raw)
    return clean_plain(raw)


def resolve_inputs(tex: str, base: Path, depth: int = 0) -> str:
    """Inline \\input{x} / \\include{x} so a multi-file paper reads as one document.
    Comments are stripped first, so commented-out inputs stay out."""
    tex = re.sub(r"(?<!\\)%.*", "", tex)
    if depth > 5:
        return tex

    def repl(m: re.Match) -> str:
        name = m.group(1).strip()
        f = base / name
        if not f.suffix:
            f = f.with_suffix(".tex")
        if not f.is_file():
            return " "
        # LaTeX resolves nested inputs relative to the main file, not the includer.
        return resolve_inputs(f.read_text(encoding="utf-8", errors="ignore"), base, depth + 1)

    return re.sub(r"\\(?:input|include)\{([^}]+)\}", repl, tex)


def read_docx(path: Path) -> str:
    with zipfile.ZipFile(path) as z:
        xml = z.read("word/document.xml").decode("utf-8", errors="ignore")
    xml = re.sub(r"</w:p>", "\n\n", xml)
    xml = re.sub(r"<w:tab/>", " ", xml)
    return html.unescape(re.sub(r"<[^>]+>", "", xml))


CITATION_PATTERNS = [
    # [12], [3, 4], [1–5]
    (re.compile(r"\[\d+(?:\s*[,–\-]\s*\d+)*\]"), " <cite> "),
    # (Smith, 2020), (Smith et al., 2020; Lee and Kim 2019a)
    (re.compile(r"\((?:[A-Z][\w'’\-]+(?: et al\.)?(?:,? (?:and|&) [A-Z][\w'’\-]+)?,? \d{4}[a-z]?(?:; ?)?)+\)"), " <cite> "),
]


def clean_plain(s: str) -> str:
    s = unicodedata.normalize("NFKC", s)  # also unpacks PDF ligatures (ﬁ -> fi)
    s = s.replace("\u00ad", "")  # soft hyphen
    s = re.sub("[\ud800-\udfff]", "", s)  # lone surrogates from PDF math fonts
    s = re.sub(r"https?://\S+|www\.\S+", " <url> ", s)
    s = re.sub(r"\b[\w.+-]+@[\w-]+\.[\w.]+\b", " <email> ", s)
    for pat, repl in CITATION_PATTERNS:
        s = pat.sub(repl, s)
    s = re.sub(r"[ \t]+", " ", s)
    return s.strip()


def clean_markdown(s: str) -> str:
    s = re.sub(r"\A---\n.*?\n---\n", "", s, flags=re.S)           # front matter
    s = re.sub(r"```.*?```", "\n\n", s, flags=re.S)                 # code fences
    s = re.sub(r"`[^`\n]+`", " <code> ", s)
    s = re.sub(r"!\[[^\]]*\]\([^)]*\)", " ", s)                      # images
    s = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", s)                   # links -> text
    s = re.sub(r"<[^>\n]+>", " ", s)                                 # html tags
    s = re.sub(r"^\s*\|.*$", "", s, flags=re.M)                      # tables
    s = re.sub(r"^\s*([=\-~*_#^+])\1{2,}\s*$", "\n\n", s, flags=re.M)  # underlines / rules
    s = re.sub(r"^\s{0,3}#{1,6}\s*(.*)$", r"\n\n\1\n\n", s, flags=re.M)  # headings as their own paragraph
    s = re.sub(r"^\s*(?:[-*+]|\d+\.)\s+", "\n\n", s, flags=re.M)     # list items as their own paragraph
    s = re.sub(r"(\*\*|__|\*|_)(?=\S)(.+?)(?<=\S)\1", r"\2", s)       # emphasis
    return clean_plain(s)


NON_PROSE_ENVS = (
    "equation", "align", "gather", "multline", "eqnarray", "displaymath", "math",
    "figure", "table", "tabular", "tabularx", "algorithm", "algorithmic", "lstlisting",
    "verbatim", "minted", "tikzpicture", "thebibliography",
)


def clean_tex(s: str) -> str:
    s = re.sub(r"(?<!\\)%.*", "", s)
    m = re.search(r"\\begin\{document\}(.*?)(?:\\end\{document\}|\Z)", s, re.S)
    if m:
        s = m.group(1)
    s = re.split(r"\\bibliography\{|\\printbibliography|\\begin\{thebibliography\}", s)[0]
    for env in NON_PROSE_ENVS:
        s = re.sub(rf"\\begin\{{{env}\*?\}}.*?\\end\{{{env}\*?\}}", "\n\n", s, flags=re.S)
    s = re.sub(r"\$\$.*?\$\$|\\\[.*?\\\]", " <math> ", s, flags=re.S)
    s = re.sub(r"(?<!\\)\$.+?(?<!\\)\$|\\\(.*?\\\)", " <math> ", s, flags=re.S)
    s = re.sub(r"~?\\[a-zA-Z]*cite[a-zA-Z]*\*?(?:\[[^\]]*\])*\{[^}]*\}", " <cite> ", s)
    s = re.sub(r"~?\\(?:ref|eqref|autoref|cref|Cref|pageref|nameref)\{[^}]*\}", " <ref> ", s)
    s = re.sub(r"\\href\{[^}]*\}\{([^}]*)\}", r"\1", s)
    s = re.sub(r"\\(?:url|label|input|include|includegraphics)(?:\[[^\]]*\])?\{[^}]*\}", " ", s)
    s = re.sub(r"\\(?:part|chapter|section|subsection|subsubsection|paragraph)\*?(?:\[[^\]]*\])?\{([^}]*)\}",
               r"\n\n\1\n\n", s)
    s = re.sub(r"\\(?:begin|end)\{[^}]*\}(?:\[[^\]]*\])?", "\n\n", s)
    s = re.sub(r"\\item\b(?:\[[^\]]*\])?", "\n\n", s)
    s = s.replace("---", "—").replace("--", "–")
    s = re.sub(r"``|''", '"', s)
    for _ in range(3):  # unwrap \emph{x}, \textbf{x}, nested a couple of levels
        s = re.sub(r"\\[a-zA-Z]+\*?(?:\[[^\]]*\])?\{([^{}]*)\}", r"\1", s)
    s = re.sub(r"\\[a-zA-Z]+\*?", " ", s)
    s = re.sub(r"\\(.)", r"\1", s)
    s = s.replace("{", "").replace("}", "").replace("~", " ")
    return clean_plain(s)


def _word_cost(w: str) -> float:
    from wordfreq import zipf_frequency

    z = zipf_frequency(w, "en")
    # Single letters other than a/I are almost never right inside a glued run.
    if z <= 0 or (len(w) == 1 and w.lower() not in ("a", "i")):
        return 25.0 + 2.0 * len(w)
    return (9.0 - z) * 2.3  # -ln(p) up to a constant


def split_glued(token: str, max_word: int = 20) -> str:
    """'Iwouldalsoliketothank' -> 'I would also like to thank' (Viterbi over
    word frequencies). Returns the token unchanged unless the split is clearly better."""
    lower = token.lower()
    n = len(lower)
    best = [0.0] + [float("inf")] * n
    back = [0] * (n + 1)
    for i in range(1, n + 1):
        for j in range(max(0, i - max_word), i):
            c = best[j] + _word_cost(lower[j:i])
            if c < best[i]:
                best[i], back[i] = c, j
    if best[n] >= _word_cost(lower) - 5:
        return token
    pieces, i = [], n
    while i > 0:
        pieces.append(token[back[i]:i])
        i = back[i]
    return " ".join(reversed(pieces))


def _fix_ligatures(s: str) -> str:
    """Some PDFs drop the fi/fl/ff ligature glyph: 'defned' -> 'defined'."""
    from wordfreq import zipf_frequency

    cache: dict[str, str] = {}

    def repl(m: re.Match) -> str:
        w = m.group()
        if w in cache:
            return cache[w]
        out = w
        z0 = zipf_frequency(w, "en")
        if z0 < 3.0:  # common words are never ligature-damaged versions of others
            best = z0 + 1.5  # repair only if the fixed word is far more common
            # The ligature glyph usually survives as a bare "f": defined -> defned,
            # first -> frst, flow -> fow, efficient -> effcient.
            for i, ch in enumerate(w):
                if ch not in "fF":
                    continue
                for ins in ("i", "l"):
                    cand = w[:i + 1] + ins + w[i + 1:]
                    z = zipf_frequency(cand, "en")
                    if z >= 3.0 and z > best:
                        best, out = z, cand
        cache[w] = out
        return out

    return re.sub(r"(?<!\w)[A-Za-z]{3,}(?!\w)", repl, s)  # also inside "domain-specifc"


def _fix_glued(s: str) -> str:
    from wordfreq import zipf_frequency

    def repl(m: re.Match) -> str:
        w = m.group()
        if zipf_frequency(w, "en") > 0 or re.fullmatch(r"(?:[A-Z][a-z]+){2,}", w):
            return w  # a real word, or a CamelCase identifier like ObjectIntersectionOf
        return split_glued(w)

    return re.sub(r"(?<!\w)[A-Za-z]{10,}(?!\w)", repl, s)


TOC_LINE = re.compile(r"^\s*(?:\d+\s+)?(?:\d+(?:\.\d+)+|Chapter\s+[IVXLC\d]+:?|[IVXLC]+\.)\s.*?(?:\s\.|\.{2,}|\s\d+)\s*$")


def clean_pdf(s: str) -> str:
    s = s.replace("\r", "")
    s = "\n".join(ln for ln in s.split("\n") if not TOC_LINE.match(ln))
    s = re.sub(r"(\w)-\n([a-z])", r"\1\2", s)  # undo end-of-line hyphenation
    # Cut the reference list (last "References" heading in the back half).
    heads = list(re.finditer(r"^\s*(?:\d+\.?\s*)?(references|bibliography|works cited)\s*$", s, re.I | re.M))
    if heads and heads[-1].start() > len(s) * 0.5:
        s = s[: heads[-1].start()]
    lines = [ln.strip() for ln in s.split("\n")]
    out: list[str] = []
    for i, ln in enumerate(lines):
        if not ln or re.fullmatch(r"[\d\s\-–—|.]+", ln):
            out.append("\n\n")
            continue
        nxt = lines[i + 1] if i + 1 < len(lines) else ""
        looks_like_heading = (
            len(ln.split()) <= 8 and not re.search(r"[.,;:?!]$", ln) and nxt[:1].isupper()
        )
        out.append(ln + ("\n\n" if looks_like_heading else " "))
    return clean_plain(_fix_glued(_fix_ligatures("".join(out))))


# --------------------------------------------------------------------------
# Draft masking (offset-preserving)
# --------------------------------------------------------------------------

def _blank(m: re.Match) -> str:
    # Keep newlines so line numbers and paragraph breaks survive.
    return re.sub(r"[^\n]", " ", m.group())


def _blank_keep_group(group: int):
    def repl(m: re.Match) -> str:
        whole, keep = m.group(), m.group(group)
        i = m.start(group) - m.start()
        return _blank_str(whole[:i]) + keep + _blank_str(whole[i + len(keep):])
    return repl


def _blank_str(s: str) -> str:
    return re.sub(r"[^\n]", " ", s)


def mask_markup(text: str, kind: str = "md") -> str:
    """Blank out markup with spaces so offsets into the result match ``text``."""
    s = text
    if kind == "tex":
        s = re.sub(r"(?<!\\)%.*", _blank, s)
        for env in NON_PROSE_ENVS:
            s = re.sub(rf"\\begin\{{{env}\*?\}}.*?\\end\{{{env}\*?\}}", _blank, s, flags=re.S)
        s = re.sub(r"\$\$.*?\$\$|\\\[.*?\\\]|(?<!\\)\$.+?(?<!\\)\$", _blank, s, flags=re.S)
        s = re.sub(r"~?\\[a-zA-Z]*(?:cite|ref)[a-zA-Z]*\*?(?:\[[^\]]*\])*\{[^}]*\}", _blank, s)
        s = re.sub(r"\\(?:begin|end|label|url|includegraphics)\*?(?:\[[^\]]*\])?\{[^}]*\}", _blank, s)
        s = re.sub(r"\\[a-zA-Z]+\*?(?:\[[^\]]*\])?\{([^{}]*)\}", _blank_keep_group(1), s)
        s = re.sub(r"\\[a-zA-Z]+\*?|[{}~]", _blank, s)
    else:
        s = re.sub(r"```.*?```", _blank, s, flags=re.S)
        s = re.sub(r"`[^`\n]+`", _blank, s)
        s = re.sub(r"!\[[^\]]*\]\([^)]*\)", _blank, s)
        s = re.sub(r"\[([^\]]+)\]\([^)]*\)", _blank_keep_group(1), s)
        s = re.sub(r"https?://\S+", _blank, s)
        s = re.sub(r"<[^>\n]+>", _blank, s)
        s = re.sub(r"^\s*\|.*$", _blank, s, flags=re.M)
        s = re.sub(r"^\s*([=\-~*_#^+])\1{2,}\s*$", _blank, s, flags=re.M)
        s = re.sub(r"^\s{0,3}#{1,6}\s", _blank, s, flags=re.M)
        s = re.sub(r"^\s*(?:[-*+]|\d+\.)\s+", _blank, s, flags=re.M)
        s = re.sub(r"\*\*|__|(?<!\w)[*_]|[*_](?!\w)", _blank, s)
    s = re.sub(r"\[\d+(?:\s*[,–\-]\s*\d+)*\]", _blank, s)
    assert len(s) == len(text)
    return s


def draft_kind(path: str | None) -> str:
    if path and path.lower().endswith(".tex"):
        return "tex"
    return "md"
