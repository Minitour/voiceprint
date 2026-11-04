"""Surface style features: the habits that word choice alone doesn't capture."""
from __future__ import annotations

import statistics
from collections import Counter

from .text import is_word

LLM_TRANSITIONS = {
    "moreover", "furthermore", "additionally", "notably", "importantly",
    "crucially", "ultimately", "overall", "essentially", "consequently",
}
FIRST_SINGULAR = {"i", "me", "my", "mine", "myself"}
FIRST_PLURAL = {"we", "us", "our", "ours", "ourselves"}
CONTRACTION_ENDINGS = {"t", "re", "ve", "ll", "m", "d"}

# name -> (label, unit) ; used for reporting
FEATURES = {
    "sentence_length_mean": ("Average sentence length", "words"),
    "sentence_length_sd": ("Sentence length variation (sd)", "words"),
    "comma_per_sentence": ("Commas per sentence", ""),
    "em_dash_per_1k": ("Em dashes", "per 1k words"),
    "semicolon_per_1k": ("Semicolons", "per 1k words"),
    "colon_per_1k": ("Colons", "per 1k words"),
    "parenthesis_per_1k": ("Parentheses", "per 1k words"),
    "question_per_1k": ("Questions", "per 1k words"),
    "first_singular_per_1k": ("I / me / my", "per 1k words"),
    "first_plural_per_1k": ("we / us / our", "per 1k words"),
    "contractions_per_1k": ("Contractions", "per 1k words"),
    "llm_transitions_per_1k": ("Moreover/furthermore/notably/...", "per 1k words"),
    "mean_word_length": ("Average word length", "chars"),
}


def features(sents: list[list[str]]) -> dict[str, float]:
    """Compute features over sentences given as lists of normalized tokens."""
    words = [t for s in sents for t in s if is_word(t)]
    nw = max(len(words), 1)
    lens = [sum(1 for t in s if is_word(t)) for s in sents]
    lens = [n for n in lens if n] or [0]
    c = Counter(t for s in sents for t in s)

    def per_k(n: float) -> float:
        return 1000.0 * n / nw

    return {
        "sentence_length_mean": statistics.fmean(lens),
        "sentence_length_sd": statistics.pstdev(lens) if len(lens) > 1 else 0.0,
        "comma_per_sentence": c[","] / len(lens),
        "em_dash_per_1k": per_k(c["—"]),
        "semicolon_per_1k": per_k(c[";"]),
        "colon_per_1k": per_k(c[":"]),
        "parenthesis_per_1k": per_k(c["("]),
        "question_per_1k": per_k(c["?"]),
        "first_singular_per_1k": per_k(sum(c[w] for w in FIRST_SINGULAR)),
        "first_plural_per_1k": per_k(sum(c[w] for w in FIRST_PLURAL)),
        "contractions_per_1k": per_k(sum(
            n for t, n in c.items() if "'" in t and t.rsplit("'", 1)[-1] in CONTRACTION_ENDINGS)),
        "llm_transitions_per_1k": per_k(sum(c[w] for w in LLM_TRANSITIONS)),
        "mean_word_length": statistics.fmean(len(w) for w in words) if words else 0.0,
    }


def chunked_profile(docs: list[list[list[str]]], chunk_words: int = 300) -> dict[str, dict[str, float]]:
    """Feature distribution over ~chunk_words-sized chunks of the corpus."""
    chunks: list[list[list[str]]] = []
    for doc in docs:
        cur, n = [], 0
        for s in doc:
            cur.append(s)
            n += sum(1 for t in s if is_word(t))
            if n >= chunk_words:
                chunks.append(cur)
                cur, n = [], 0
        if n >= chunk_words // 2:
            chunks.append(cur)
    if not chunks:
        chunks = [[s for doc in docs for s in doc]]
    rows = [features(ch) for ch in chunks]
    prof = {}
    for name in FEATURES:
        vals = sorted(r[name] for r in rows)
        prof[name] = {
            "mean": statistics.fmean(vals),
            "sd": statistics.pstdev(vals) if len(vals) > 1 else 0.0,
            "p10": vals[int(0.1 * (len(vals) - 1))],
            "p90": vals[int(0.9 * (len(vals) - 1))],
        }
    prof["_chunks"] = {"n": len(chunks), "words": chunk_words}
    return prof


def compare(draft: dict[str, float], profile: dict) -> list[dict]:
    out = []
    for name, (label, unit) in FEATURES.items():
        p = profile[name]
        x = draft[name]
        # Floor the sd so features you never use (sd = 0) still yield a finite z.
        sd = max(p["sd"], 0.15 * abs(p["mean"]), 0.5 if unit.startswith("per") else 0.05)
        z = (x - p["mean"]) / sd
        out.append({
            "feature": name, "label": label, "unit": unit,
            "draft": round(x, 2), "yours_mean": round(p["mean"], 2),
            "yours_range": [round(p["p10"], 2), round(p["p90"], 2)],
            "z": round(z, 2),
            "off": abs(z) >= 2 and not (p["p10"] <= x <= p["p90"]),
        })
    return out
