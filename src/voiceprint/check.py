"""Score a draft against your voice model and flag what doesn't sound like you.

Four kinds of flags:

marker     A word the LLM-marker list knows LLMs over-use, and that you rarely use.
phrase     Same, for multi-word phrases and constructions ("not only ... but also").
underused  A common English word you use at under 1/5 of its general-English rate,
           and the shortfall is significant for your corpus size (Poisson).
           Rare/technical words are never flagged this way: their absence from
           your corpus means nothing. Never "high": new topics bring new words.
context    A common word that is in the top 5% most surprising positions for
           your own unseen writing, where your model strongly prefers another
           word in the same slot.

Replacement candidates are synonyms (curated map + WordNet) and words you have
used in the same left/right frame, restricted to words in your corpus and
ranked by how much more likely the local window becomes under your model
(gain in bits; +3 means 8x more likely).
"""
from __future__ import annotations

import bisect
import heapq
import math
import re
from collections import Counter
from dataclasses import asdict, dataclass, field
from functools import lru_cache
from operator import itemgetter
from pathlib import Path

from wordfreq import zipf_frequency

from . import style
from .model import VoiceModel
from .alternatives import synonyms
from .text import BOS, EOS, PLACEHOLDERS, is_word, mask_markup, sentences, tokenize

DATA = Path(__file__).parent / "data"

# Discourse fillers for which "just delete it" is a legitimate suggestion.
DELETABLE = {
    "notably", "furthermore", "moreover", "additionally", "importantly", "ultimately",
    "essentially", "crucially", "remarkably", "seamlessly", "truly", "very", "really",
    "indeed", "arguably", "overall", "profoundly", "meticulously", "basically",
    "actually", "clearly", "simply", "significantly",
}
FUNCTION_ZIPF = 6.0   # the/of/and/to/is...: governed by syntax, not style choice
COMMON_ZIPF = 3.0     # below this a word is "rare" (often technical) in English
UNDERUSE_RATIO = 0.2  # flag only if you use a word at < 1/5 of the English rate
SEVERITY_RANK = {"high": 3, "medium": 2, "low": 1}


@lru_cache(maxsize=None)
def zipf(w: str) -> float:
    return zipf_frequency(w, "en")


def poisson_cdf(k: int, lam: float) -> float:
    """P(X <= k) for X ~ Poisson(lam)."""
    term = math.exp(-lam)
    total = term
    for i in range(1, k + 1):
        term *= lam / i
        total += term
    return min(total, 1.0)


def load_markers(path: str | Path | None = None):
    words: set[str] = set()
    patterns: list[tuple[str, re.Pattern]] = []
    for line in Path(path or DATA / "llm_markers.txt").read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("re:"):
            patterns.append((line, re.compile(line[3:], re.I)))
        elif " " in line:
            pat = re.escape(line.lower()).replace("'", "['’]")
            patterns.append((line, re.compile(rf"(?<!\w){pat}(?!\w)", re.I)))
        else:
            words.add(line.lower())
    return words, patterns


@dataclass
class Flag:
    kind: str
    severity: str
    text: str
    start: int
    end: int
    line: int = 0
    col: int = 0
    reason: str = ""
    suggestions: list = field(default_factory=list)
    stats: dict = field(default_factory=dict)


class Checker:
    def __init__(self, model: VoiceModel, markers_path=None, margin: float = 3.0, k: int = 5):
        self.m = model
        self.marker_words, self.marker_patterns = load_markers(markers_path)
        self.margin = margin
        self.k = k
        self._corpus_lower: str | None = None

    # -- helpers ---------------------------------------------------------
    def corpus_count(self, pattern: re.Pattern) -> int:
        if self._corpus_lower is None:
            self._corpus_lower = "\n".join(self.m.sentences).lower().replace("’", "'")
        return len(pattern.findall(self._corpus_lower))

    def usage(self, w: str) -> dict:
        c = self.m.count(w)
        z = zipf(w)
        return {
            "your_count": c,
            "your_per_million": round(c / max(self.m.total_words, 1) * 1e6, 2),
            "english_per_million": round(10 ** (z - 3), 2) if z > 0 else 0.0,
            "expected_count": round(10 ** (z - 9) * self.m.total_words, 2) if z > 0 else 0.0,
        }

    def suggest(self, seq: list[str], pos: int) -> list[dict]:
        """Replacement candidates for seq[pos], ranked by your model in context.

        Sources, in order of trust:
          synonym  curated map / WordNet, so meaning is roughly preserved
          frame    words you have written between the same left and right word
          delete   for discourse fillers ("Notably, ...")
        Every candidate must occur in your corpus at least twice.
        """
        m, N = self.m, self.m.order
        original = seq[pos]
        hist = seq[pos - N + 1:pos]
        right = seq[pos + 1:pos + N]
        left1, right1 = seq[pos - 1], (seq[pos + 1] if pos + 1 < len(seq) else EOS)
        frame = m.slots.get((left1, right1), {})
        base = m.logprob_seq(hist, [original] + right)

        def score(alt: str, source: str) -> dict | None:
            toks = [t.norm for t in tokenize(alt)]
            if not toks or any(not is_word(t) or m.count(t) < 2 for t in toks):
                return None
            gain = m.logprob_seq(hist, toks + right) - base
            return {"word": alt, "source": source, "gain_bits": round(gain, 1),
                    "seen_in_this_frame": frame.get(alt, 0)}

        syn = [s for s in (score(a, "synonym") for a in synonyms(original)) if s]
        content = zipf(original) < FUNCTION_ZIPF
        framed = []
        for w, c in heapq.nlargest(30, frame.items(), key=itemgetter(1)):
            if w == original or w in PLACEHOLDERS or (content and zipf(w) >= FUNCTION_ZIPF):
                continue  # don't propose "the" for "crucial"
            s = score(w, "frame")
            if s and not any(x["word"] == w for x in syn):
                framed.append(s)
        syn.sort(key=lambda d: -d["gain_bits"])
        framed.sort(key=lambda d: (-d["seen_in_this_frame"], -d["gain_bits"]))
        out = syn[: max(self.k - 2, 3)] + framed[:2]

        if original in DELETABLE:
            skip_comma = bool(right) and right[0] == ","
            rest = right[1:] if skip_comma else right
            with_w = m.logprob_seq(hist + [original] + ([","] if skip_comma else []), rest)
            without = m.logprob_seq(hist, rest)
            out.insert(0, {"word": "(delete)", "source": "delete",
                           "gain_bits": round(without - with_w, 1), "seen_in_this_frame": 0})
        return out[: self.k]

    # -- main ------------------------------------------------------------
    def check(self, text: str, kind: str = "md") -> dict:
        m, N = self.m, self.m.order
        masked = mask_markup(text, kind)
        sents = sentences(masked)
        p95 = m.calib["word_surprisal_pct"]["95"]
        flags: dict[tuple[int, int], Flag] = {}
        surprisals: list[float] = []
        token_sents: list[list[str]] = []

        for sent in sents:
            norms = [t.norm for t in sent]
            token_sents.append(norms)
            seq = [BOS] * (N - 1) + norms + [EOS]
            seen_word = False
            for i, tok in enumerate(sent):
                w = tok.norm
                if not is_word(w):
                    continue
                pos = i + N - 1
                s = m.surprisal(w, seq[pos - N + 1:pos])
                surprisals.append(s)
                initial = not seen_word
                seen_word = True
                proper = (not initial and tok.surface[:1].isupper()) or (len(tok.surface) > 1 and tok.surface.isupper())
                if w in PLACEHOLDERS or proper or (len(w) < 3 and w not in self.marker_words):
                    continue
                u = self.usage(w)
                z = zipf(w)
                flag = None
                if w in self.marker_words and (u["your_count"] < 3 or u["your_per_million"] < 2 * u["english_per_million"]):
                    flag = Flag("marker", "high" if u["your_count"] == 0 else "medium", tok.surface, tok.start, tok.end,
                                reason=f"LLM-favoured word; you used it {u['your_count']}x in {m.total_words:,} words")
                elif (z >= COMMON_ZIPF and u["expected_count"] >= 3
                      and u["your_count"] < UNDERUSE_RATIO * u["expected_count"]):
                    p = poisson_cdf(u["your_count"], u["expected_count"])
                    if p < 0.01:
                        flag = Flag("underused", "medium" if p < 0.001 else "low",
                                    tok.surface, tok.start, tok.end,
                                    reason=(f"you used it {u['your_count']}x; general English usage predicts "
                                            f"~{u['expected_count']:.0f}x in a corpus your size"))
                if flag is None and COMMON_ZIPF <= z < FUNCTION_ZIPF and s > p95:
                    sug = self.suggest(seq, pos)
                    # Only a meaning-preserving alternative justifies a context flag;
                    # frame-only candidates would flag every new technical term.
                    if any(x["gain_bits"] >= self.margin and x["source"] != "frame" for x in sug):
                        flag = Flag("context", "low", tok.surface, tok.start, tok.end,
                                    reason=f"unusual for you here ({s:.1f} bits; your 95th pct is {p95:.1f})",
                                    suggestions=sug)
                if flag:
                    if not flag.suggestions:
                        flag.suggestions = self.suggest(seq, pos)
                    flag.stats = {**u, "surprisal_bits": round(s, 2)}
                    flags[(tok.start, tok.end)] = flag

        # Multi-word markers, matched on the masked draft.
        for label, pat in self.marker_patterns:
            for mt in pat.finditer(masked):
                yours = self.corpus_count(pat)
                if yours >= 2:
                    continue
                key = (mt.start(), mt.end())
                flags[key] = Flag("phrase", "high" if yours == 0 else "medium", text[mt.start():mt.end()],
                                  mt.start(), mt.end(),
                                  reason=f"LLM-favoured construction ({label}); you used it {yours}x",
                                  stats={"your_count": yours})

        # Word flags inside a flagged phrase are redundant; overlapping phrases keep the longest.
        phrase_spans = sorted(((f.start, f.end) for f in flags.values() if f.kind == "phrase"),
                              key=lambda se: se[0] - se[1])
        kept: list[tuple[int, int]] = []
        for st, en in phrase_spans:
            if any(st < ke and en > ks for ks, ke in kept):
                flags.pop((st, en))
            else:
                kept.append((st, en))
        for key, f in list(flags.items()):
            if f.kind != "phrase" and any(ks <= f.start and f.end <= ke for ks, ke in kept):
                flags.pop(key)

        line_starts = [0] + [i + 1 for i, ch in enumerate(text) if ch == "\n"]
        for f in flags.values():
            ln = bisect.bisect_right(line_starts, f.start)
            f.line, f.col = ln, f.start - line_starts[ln - 1] + 1

        ordered = sorted(flags.values(), key=lambda f: f.start)
        n_words = len(surprisals)
        cal = m.calib
        above = sum(1 for s in surprisals if s > p95) / max(n_words, 1)
        sev = Counter(f.severity for f in ordered)
        style_rows = style.compare(style.features(token_sents), m.style) if token_sents else []
        mean_s = sum(surprisals) / max(n_words, 1)

        # Thresholds from held-out tests: your own unseen writing typically sits at
        # 0-1 high flags, 1-5 flags per 100 words, and 5-10% of words above p95.
        per100 = 100 * len(ordered) / max(n_words, 1)
        style_off = sum(r["off"] for r in style_rows)
        if sev["high"] >= 4 or per100 >= 8 or above >= 0.15:
            verdict = "does not sound like you"
        elif sev["high"] <= 1 and per100 <= 5 and above <= 0.12 and style_off <= 1:
            verdict = "close to your voice"
        else:
            verdict = "drifting"

        return {
            "summary": {
                "words": n_words,
                "sentences": len(sents),
                "mean_surprisal_bits": round(mean_s, 2),
                "your_mean_surprisal_bits": round(cal["word_surprisal_mean"], 2),
                "share_above_your_p95": round(above, 3),
                "your_share_above_p95": 0.05,
                "flags": {k: sev.get(k, 0) for k in ("high", "medium", "low")},
                "flags_per_100_words": round(100 * len(ordered) / max(n_words, 1), 1),
                "verdict": verdict,
                "short_draft": n_words < 150,
            },
            "style": style_rows,
            "flags": [asdict(f) for f in ordered],
        }


def worst_severity(report: dict) -> int:
    return max((SEVERITY_RANK[f["severity"]] for f in report["flags"]), default=0)


# ----------------------------------------------------------------------
# Rendering
# ----------------------------------------------------------------------

def _fmt_sugs(sugs: list[dict], n: int = 4) -> str:
    good = [s for s in sugs if s["gain_bits"] > 0][:n]
    if not good:
        return "(no better fit in your corpus; rephrase)"
    syn = [f"{s['word']} ({s['gain_bits']:+.1f})" for s in good if s["source"] != "frame"]
    frame = [f"{s['word']} ({s['gain_bits']:+.1f})" for s in good if s["source"] == "frame"]
    return ", ".join(syn) + (("   | same slot: " if syn else "same slot: ") + ", ".join(frame) if frame else "")


def render_text(report: dict, name: str, model: VoiceModel, max_flags: int = 60) -> str:
    s = report["summary"]
    out = [
        f"voiceprint check: {name}",
        f"  index: {model.meta.get('words', 0):,} words from {model.meta.get('documents', 0)} documents",
        "",
        "Voice",
        f"  verdict                   {s['verdict']}",
        f"  surprisal (bits/word)     {s['mean_surprisal_bits']:.2f}   yours on unseen text: {s['your_mean_surprisal_bits']:.2f}",
        f"  words above your p95      {100 * s['share_above_your_p95']:.1f}%   yours: 5%",
        f"  flags                     {s['flags']['high']} high, {s['flags']['medium']} medium, {s['flags']['low']} low"
        f"  ({s['flags_per_100_words']} per 100 words)",
    ]
    if s["short_draft"]:
        out.append("  note: under 150 words, so style numbers are noisy")
    out += ["", "Style (draft vs your typical p10-p90 range)"]
    for r in report["style"]:
        mark = "!" if r["off"] else " "
        lo, hi = r["yours_range"]
        out.append(f"  {mark} {r['label']:<36} {r['draft']:>7.2f}   yours {r['yours_mean']:.2f} (range {lo:.2f}-{hi:.2f}) {r['unit']}")
    flags = sorted(report["flags"], key=lambda f: (-SEVERITY_RANK[f["severity"]], f["start"]))[:max_flags]
    out += ["", f"Flags ({len(report['flags'])})"]
    for f in sorted(flags, key=lambda f: f["start"]):
        out.append(f"  {f['line']}:{f['col']:<4} {f['severity'].upper():<6} {f['kind']:<9} \"{f['text']}\"  {f['reason']}")
        if f["suggestions"]:
            out.append(f"  {'':<22}-> {_fmt_sugs(f['suggestions'])}")
    if len(report["flags"]) > max_flags:
        out.append(f"  ... {len(report['flags']) - max_flags} more (use --json or --max-flags)")
    return "\n".join(out)


def annotate(text: str, report: dict) -> str:
    out = text
    for f in sorted(report["flags"], key=lambda f: -f["start"]):
        alts = [s["word"] for s in f["suggestions"] if s["gain_bits"] > 0][:3]
        tag = f"[[{out[f['start']:f['end']]} -> {' | '.join(alts) if alts else '?'}]]"
        out = out[: f["start"]] + tag + out[f["end"]:]
    return out
