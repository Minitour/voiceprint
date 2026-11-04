"""Meaning-preserving candidate words: a curated map plus WordNet (optional).

The n-gram model knows nothing about meaning; it can only say which word *you*
would put in a slot. So candidates come from a semantic source first, and the
model ranks them. WordNet needs ``pip install nltk`` and
``python -m nltk.downloader wordnet``; without it only the curated map is used.
"""
from __future__ import annotations

from functools import lru_cache
from pathlib import Path

DATA = Path(__file__).parent / "data"
SUFFIXES = ("ally", "ing", "ies", "ied", "ed", "es", "ly", "s")


@lru_cache(maxsize=1)
def curated() -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    for line in (DATA / "plain_alternatives.txt").read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or ":" not in line:
            continue
        head, alts = line.split(":", 1)
        out[head.strip().lower()] = [a.strip().lower() for a in alts.split(",") if a.strip()]
    return out


@lru_cache(maxsize=1)
def _wordnet():
    try:
        from nltk.corpus import wordnet as wn

        wn.ensure_loaded()
        return wn
    except Exception:
        return None


def wordnet_available() -> bool:
    return _wordnet() is not None


def _strip_suffix(word: str) -> tuple[str, str]:
    for suf in SUFFIXES:
        if word.endswith(suf) and len(word) - len(suf) >= 3:
            return word[: -len(suf)], suf
    return word, ""


def inflect(lemma: str, suffix: str) -> set[str]:
    """Crude inflection; wrong forms are harmless because candidates are
    filtered to words that actually occur in your corpus."""
    if not suffix or " " in lemma:
        return {lemma}
    forms = {lemma, lemma + suffix}
    if lemma.endswith("e"):
        forms.add(lemma[:-1] + suffix)          # use -> using
    if lemma.endswith("y"):
        forms.add(lemma[:-1] + "i" + suffix)    # apply -> applies / easily
    if suffix in ("ies", "ied"):
        forms.add(lemma[:-1] + suffix if lemma.endswith("y") else lemma + suffix[1:])
    if suffix in ("ing", "ed") and len(lemma) >= 3:
        forms.add(lemma + lemma[-1] + suffix)   # stop -> stopped
    if suffix == "ly" and lemma.endswith("le"):
        forms.add(lemma[:-1] + "y")             # simple -> simply
    if suffix == "ally":
        forms.add(lemma + "ly")
    return forms


@lru_cache(maxsize=20000)
def synonyms(word: str) -> tuple[str, ...]:
    """Candidate replacements for ``word``, inflected like it."""
    word = word.lower()
    out: set[str] = set()
    stem, suffix = _strip_suffix(word)

    for key in (word, stem, stem + "e", stem + "y"):
        for alt in curated().get(key, []):
            out |= inflect(alt, suffix if key != word else "")

    wn = _wordnet()
    if wn is not None:
        for pos in "nvar":
            base = wn.morphy(word, pos)
            bases = [base] if base else []
            if not bases and word.endswith("ly"):      # seamlessly -> seamless
                bases = [wn.morphy(word[:-2], "a")] if wn.morphy(word[:-2], "a") else []
            for b in bases:
                suf = word[len(b):] if word.startswith(b) else suffix
                for syn in wn.synsets(b, pos):
                    related = [syn] + (syn.similar_tos() if pos in "as" else [])
                    for s in related:
                        for lem in s.lemmas():
                            name = lem.name().lower()
                            if "_" not in name and name != b:
                                out |= inflect(name, suf)
    out.discard(word)
    return tuple(sorted(out))
