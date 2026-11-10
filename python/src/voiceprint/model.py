"""A word-level interpolated Kneser-Ney n-gram model of one author's writing.

Besides the n-gram tables the index keeps:

* ``slots``    (left word, right word) -> middle words, for fill-in-the-blank
* ``prev``     right word -> words that precede it
* ``unigram``  raw counts, for comparing your usage against general English
* ``sentences`` the cleaned corpus sentences, for keyword-in-context lookups
* ``calib``    surprisal percentiles measured on held-out documents, so
               "unusual" means unusual relative to your own unseen writing
* ``style``    per-chunk style feature distribution
"""
from __future__ import annotations

import gzip
import math
import pickle
import time
from collections import defaultdict
from pathlib import Path

from . import style
from .text import BOS, EOS, is_word

INDEX_VERSION = 1


class VoiceModel:
    def __init__(self, order: int = 4):
        if order < 2:
            raise ValueError("order must be >= 2")
        self.order = order
        # tables[k][context] = (counts: dict[word, int], total: int)
        # order == N holds raw counts; lower orders hold continuation counts.
        self.tables: dict[int, dict[tuple, tuple[dict, int]]] = {}
        self.D: dict[int, float] = {}
        self.V = 1
        self.unigram: dict[str, int] = {}
        self.total_words = 0
        self.slots: dict[tuple[str, str], dict[str, int]] = {}
        self.prev: dict[str, dict[str, int]] = {}
        self.sentences: list[str] = []
        self.sentence_doc: list[int] = []
        self.docs: list[str] = []
        self.calib: dict = {}
        self.style: dict = {}
        self.meta: dict = {}

    # ------------------------------------------------------------------
    # Training
    # ------------------------------------------------------------------
    def fit(self, sents: list[list[str]]) -> "VoiceModel":
        N = self.order
        top: dict[tuple, dict[str, int]] = defaultdict(lambda: defaultdict(int))
        unigram: dict[str, int] = defaultdict(int)
        slots: dict[tuple, dict[str, int]] = defaultdict(lambda: defaultdict(int))
        prev: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))

        for s in sents:
            seq = [BOS] * (N - 1) + s + [EOS]
            for i in range(N - 1, len(seq)):
                top[tuple(seq[i - N + 1:i])][seq[i]] += 1
            for i, t in enumerate(seq[N - 1:-1], start=N - 1):
                unigram[t] += 1
                left, right = seq[i - 1], seq[i + 1]
                slots[(left, right)][t] += 1
                prev[right][t] += 1

        tables: dict[int, dict] = {N: {ctx: (dict(c), sum(c.values())) for ctx, c in top.items()}}
        # Continuation counts: how many distinct words precede (ctx, w).
        for k in range(N - 1, 0, -1):
            cont: dict[tuple, dict[str, int]] = defaultdict(lambda: defaultdict(int))
            for ctx, (counts, _) in tables[k + 1].items():
                sub = ctx[1:]
                for w in counts:
                    cont[sub][w] += 1
            tables[k] = {ctx: (dict(c), sum(c.values())) for ctx, c in cont.items()}
        self.tables = tables

        for k, tab in tables.items():
            n1 = n2 = 0
            for counts, _ in tab.values():
                for c in counts.values():
                    if c == 1:
                        n1 += 1
                    elif c == 2:
                        n2 += 1
            d = n1 / (n1 + 2 * n2) if n1 + n2 else 0.75
            self.D[k] = min(max(d, 0.1), 0.95)

        self.unigram = dict(unigram)
        self.total_words = sum(c for w, c in unigram.items() if is_word(w))
        self.V = len(tables[1][()][0]) + 1
        self.slots = {k: dict(v) for k, v in slots.items()}
        self.prev = {k: dict(v) for k, v in prev.items()}
        return self

    # ------------------------------------------------------------------
    # Probabilities
    # ------------------------------------------------------------------
    def _hist(self, hist) -> tuple:
        n = self.order - 1
        h = tuple(hist[-n:]) if hist else ()
        if len(h) < n:
            h = (BOS,) * (n - len(h)) + h
        return h

    def _p(self, w: str, hist: tuple) -> float:
        k = len(hist) + 1
        if k == 1:
            counts, total = self.tables[1][()]
            D = self.D[1]
            return max(counts.get(w, 0) - D, 0) / total + D * len(counts) / total / self.V
        lower = self._p(w, hist[1:])
        entry = self.tables[k].get(hist)
        if entry is None:
            return lower
        counts, total = entry
        D = self.D[k]
        return max(counts.get(w, 0) - D, 0) / total + D * len(counts) / total * lower

    def prob(self, w: str, hist) -> float:
        return self._p(w, self._hist(hist))

    def surprisal(self, w: str, hist) -> float:
        """Bits of surprise for ``w`` after ``hist`` (higher = less like you)."""
        return -math.log2(self.prob(w, hist))

    def logprob_seq(self, hist, seq) -> float:
        h = list(self._hist(hist))
        lp = 0.0
        for t in seq:
            lp += math.log2(self._p(t, tuple(h[-(self.order - 1):])))
            h.append(t)
        return lp

    def continuations(self, hist, k: int = 10, words_only: bool = False) -> list[tuple[str, float]]:
        """Most likely next tokens after ``hist`` (autocomplete)."""
        h = self._hist(hist)
        cands: set[str] = set()
        for j in range(len(h) + 1):
            entry = self.tables[len(h) - j + 1].get(h[j:])
            if entry:
                cands.update(entry[0])
            if len(cands) >= 200:
                break
        scored = [(w, self._p(w, h)) for w in cands if not words_only or is_word(w)]
        scored.sort(key=lambda x: -x[1])
        return scored[:k]

    def count(self, w: str) -> int:
        return self.unigram.get(w, 0)

    # ------------------------------------------------------------------
    # Persistence
    # ------------------------------------------------------------------
    def save(self, path: str | Path) -> None:
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        with gzip.open(path, "wb", compresslevel=5) as f:
            pickle.dump({"version": INDEX_VERSION, "model": self.__dict__}, f, protocol=pickle.HIGHEST_PROTOCOL)

    @classmethod
    def load(cls, path: str | Path) -> "VoiceModel":
        # pickle: only load index files you built yourself.
        with gzip.open(path, "rb") as f:
            blob = pickle.load(f)
        if blob.get("version") != INDEX_VERSION:
            raise ValueError(f"{path} was built by an incompatible voiceprint version; rebuild it.")
        m = cls.__new__(cls)
        m.__dict__.update(blob["model"])
        return m


# ----------------------------------------------------------------------
# Building an index from documents
# ----------------------------------------------------------------------

def _percentiles(vals: list[float], ps=(50, 75, 90, 95, 99)) -> dict[str, float]:
    vals = sorted(vals)
    if not vals:
        return {str(p): 0.0 for p in ps}
    return {str(p): vals[min(len(vals) - 1, int(p / 100 * len(vals)))] for p in ps}


def calibrate(folds: list[tuple[list, list]], order: int) -> dict:
    """Score each held-out fold with a model trained on the other folds."""
    word_s: list[float] = []
    sent_means: list[float] = []
    for train_docs, held_docs in folds:
        m = VoiceModel(order).fit([s for d in train_docs for s in d])
        for d in held_docs:
            for s in d:
                seq = [BOS] * (order - 1) + s
                vals = [m.surprisal(t, seq[:i + order - 1]) for i, t in enumerate(s) if is_word(t)]
                if vals:
                    word_s.extend(vals)
                    sent_means.append(sum(vals) / len(vals))
    mean = sum(word_s) / max(len(word_s), 1)
    sm = sum(sent_means) / max(len(sent_means), 1)
    ssd = (sum((x - sm) ** 2 for x in sent_means) / max(len(sent_means) - 1, 1)) ** 0.5
    return {
        "word_surprisal_mean": mean,
        "word_surprisal_pct": _percentiles(word_s),
        "sentence_mean": sm,
        "sentence_sd": ssd,
        "sentence_pct": _percentiles(sent_means),
        "held_out_words": len(word_s),
    }


def build(docs: list[tuple[str, list[list[str]], list[str]]], order: int = 4) -> VoiceModel:
    """docs: (name, token sentences, surface sentences) per document."""
    t0 = time.time()
    token_docs = [d[1] for d in docs]

    # Cross-validate by document (up to 5 folds) so every document is scored by a
    # model that never saw it. A single document falls back to a 80/20 split.
    k = min(len(token_docs), 5)
    if k >= 2:
        folds = [([d for i, d in enumerate(token_docs) if i % k != f],
                  [d for i, d in enumerate(token_docs) if i % k == f]) for f in range(k)]
        how = f"{k}-fold cross-validation by document"
    else:
        d = token_docs[0]
        cut = int(len(d) * 0.8)
        folds = [([d[:cut]], [d[cut:]])]
        how = "held-out tail of the only document"
    calib = calibrate(folds, order)

    m = VoiceModel(order).fit([s for d in token_docs for s in d])
    m.calib = calib
    m.style = style.chunked_profile(token_docs)
    for i, (name, _, surf) in enumerate(docs):
        m.docs.append(name)
        m.sentences.extend(surf)
        m.sentence_doc.extend([i] * len(surf))
    m.meta = {
        "built": time.strftime("%Y-%m-%d %H:%M"),
        "seconds": round(time.time() - t0, 1),
        "documents": len(docs),
        "sentences": len(m.sentences),
        "words": m.total_words,
        "vocabulary": m.V - 1,
        "order": order,
        "calibration": how,
    }
    return m
