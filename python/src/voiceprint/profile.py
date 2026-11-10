"""A human/agent-readable "voice card" summarizing the index."""
from __future__ import annotations

from collections import Counter

from . import style
from .check import load_markers, zipf
from .model import VoiceModel
from .text import PLACEHOLDERS, is_word, tokenize

TRANSITIONS = [
    "however", "thus", "therefore", "hence", "so", "also", "moreover", "furthermore",
    "additionally", "consequently", "indeed", "notably", "importantly", "specifically",
    "namely", "instead", "rather", "yet", "still", "meanwhile", "finally", "first",
    "second", "then", "overall", "in particular", "for example", "for instance",
    "in addition", "in contrast", "on the other hand", "as a result", "that is",
    "e.g.", "i.e.",
]


def _rate(m: VoiceModel, c: int, per: float = 1e4) -> float:
    return per * c / max(m.total_words, 1)


def _phrase_count(m: VoiceModel, phrase: str, corpus: str) -> int:
    if " " not in phrase:
        return m.count(phrase)
    return corpus.count(f" {phrase} ") + corpus.count(f"\n{phrase} ")


def voice_card(m: VoiceModel, markers_path=None) -> str:
    meta = m.meta
    words = [(w, c) for w, c in m.unigram.items() if is_word(w) and w not in PLACEHOLDERS and len(w) > 1]
    total = max(m.total_words, 1)

    def ratio(w, c):
        z = zipf(w)
        eng = 10 ** (z - 9) if z > 0 else 1e-9
        return (c / total) / eng

    distinctive = sorted((x for x in words if x[1] >= 5 and zipf(x[0]) >= 4.0), key=lambda x: -ratio(*x))[:30]
    topical = sorted((x for x in words if x[1] >= 5 and zipf(x[0]) < 4.0), key=lambda x: -ratio(*x))[:30]
    seen = {w for w, _ in words}
    avoided = []
    for w in _common_english():
        if w in seen or not w.isalpha() or len(w) < 2:
            continue
        exp = 10 ** (zipf(w) - 9) * total
        if exp >= 5:
            avoided.append((w, exp))
    avoided.sort(key=lambda x: -x[1])

    corpus = "\n".join(s.lower() for s in m.sentences)
    corpus = " " + corpus.replace("\n", " \n")
    trans = [(t, _phrase_count(m, t, corpus)) for t in TRANSITIONS]
    trans = sorted((x for x in trans if x[1] > 0), key=lambda x: -x[1])

    openers = Counter()
    for s in m.sentences:
        ws = [t.norm for t in tokenize(s) if is_word(t.norm)][:2]
        if len(ws) == 2:
            openers[" ".join(ws)] += 1

    mwords, mpatterns = load_markers(markers_path)
    used_markers = sorted((w for w in mwords if m.count(w) >= 3), key=lambda w: -m.count(w))
    never_markers = sorted(w for w in mwords if m.count(w) == 0)

    L = [
        "# Voice card",
        "",
        f"Built {meta.get('built')} from {meta.get('documents')} documents, "
        f"{meta.get('words', 0):,} words, {meta.get('sentences', 0):,} sentences (n-gram order {m.order}).",
        "",
        "## Style norms (typical p10-p90 range across ~300-word chunks)",
        "",
        "| Feature | Your mean | Typical range |",
        "|---|---|---|",
    ]
    for name, (label, unit) in style.FEATURES.items():
        p = m.style[name]
        L.append(f"| {label} ({unit}) | {p['mean']:.2f} | {p['p10']:.2f} - {p['p90']:.2f} |" if unit
                 else f"| {label} | {p['mean']:.2f} | {p['p10']:.2f} - {p['p90']:.2f} |")
    L += [
        "",
        "## Common words you use far more than general English",
        "",
        ", ".join(f"{w} ({c})" for w, c in distinctive) or "(none)",
        "",
        "## Your topic vocabulary",
        "",
        ", ".join(f"{w} ({c})" for w, c in topical) or "(none)",
        "",
        "## Transitions you use (count, per 10k words)",
        "",
        ", ".join(f"{t} ({c}, {_rate(m, c):.1f})" for t, c in trans) or "(none)",
        "",
        "## Common sentence openers",
        "",
        ", ".join(f'"{o}" ({c})' for o, c in openers.most_common(20)),
        "",
        "## Common English words you never use",
        "",
        "Each would be expected at least 5 times in a corpus your size.",
        "",
        ", ".join(w for w, _ in avoided[:60]) or "(none)",
        "",
        "## LLM-marker words you genuinely use (fine to keep)",
        "",
        ", ".join(f"{w} ({m.count(w)})" for w in used_markers) or "(none)",
        "",
        "## LLM-marker words you never use (avoid)",
        "",
        ", ".join(never_markers) or "(none)",
        "",
    ]
    return "\n".join(L)


def _common_english(n: int = 4000) -> list[str]:
    from wordfreq import top_n_list

    return top_n_list("en", n)
