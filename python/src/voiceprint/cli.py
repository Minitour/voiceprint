"""voiceprint command line."""
from __future__ import annotations

import argparse
import fnmatch
import json
import os
import re
import sys
from pathlib import Path

DEFAULT_INDEX = Path(os.environ.get("VOICEPRINT_INDEX", Path.home() / ".voiceprint" / "voice.vpi"))


def _load(path):
    from .model import VoiceModel

    p = Path(path)
    if not p.exists():
        sys.exit(f"No index at {p}. Build one first: voiceprint build <files or folders you wrote> -o {p}")
    return VoiceModel.load(p)


def _read_input(arg: str) -> tuple[str, str]:
    if arg == "-":
        return sys.stdin.read(), "<stdin>"
    return Path(arg).read_text(encoding="utf-8", errors="ignore"), arg


def cmd_build(a):
    from .model import build
    from .text import iter_files, read_file, sentences

    docs = []
    for f in iter_files(a.paths):
        if any(fnmatch.fnmatch(f.name, pat) for pat in a.exclude):
            continue
        try:
            text = read_file(f)
        except Exception as e:  # unreadable PDF etc.
            print(f"  skip {f}: {e}", file=sys.stderr)
            continue
        sents = sentences(text)
        n_words = sum(1 for s in sents for t in s if t.norm[:1].isalpha())
        if n_words < a.min_words:
            print(f"  skip {f}: only {n_words} words", file=sys.stderr)
            continue
        docs.append((str(f), [[t.norm for t in s] for s in sents],
                     [" ".join(text[s[0].start:s[-1].end].split()) for s in sents]))
        print(f"  read {f}  ({n_words:,} words)", file=sys.stderr)
    if not docs:
        sys.exit("No usable documents found.")
    if a.dump_clean:
        d = Path(a.dump_clean)
        d.mkdir(parents=True, exist_ok=True)
        for i, (name, _, surf) in enumerate(docs):
            stem = f"{i:02d}-{Path(name).parent.name}-{Path(name).stem}"[:80]
            (d / f"{stem}.clean.txt").write_text("\n".join(surf), encoding="utf-8")
    m = build(docs, order=a.order)
    m.save(a.output)
    meta, cal = m.meta, m.calib
    print(f"Built {a.output}: {meta['documents']} docs, {meta['words']:,} words, "
          f"vocab {meta['vocabulary']:,}, order {meta['order']}, {meta['seconds']}s")
    print(f"Calibration ({meta['calibration']}): mean {cal['word_surprisal_mean']:.2f} bits/word, "
          f"p95 {cal['word_surprisal_pct']['95']:.2f}")
    if meta["words"] < 30000:
        print("Note: under ~30k words the model is thin; add more of your writing if you can.")


def cmd_check(a):
    from .check import SEVERITY_RANK, Checker, annotate, render_text, worst_severity
    from .text import draft_kind

    m = _load(a.index)
    text, name = _read_input(a.file)
    report = Checker(m, markers_path=a.markers, margin=a.margin, k=a.suggestions).check(
        text, kind=a.kind or draft_kind(None if a.file == "-" else a.file))
    if a.json:
        print(json.dumps(report, indent=2, ensure_ascii=False))
    elif a.annotate:
        print(annotate(text, report))
    else:
        print(render_text(report, name, m, max_flags=a.max_flags))
    if a.fail_on != "never" and worst_severity(report) >= SEVERITY_RANK[a.fail_on]:
        sys.exit(1)


def cmd_suggest(a):
    from .check import Checker
    from .text import BOS, EOS, tokenize

    m = _load(a.index)
    left = [t.norm for t in tokenize(a.left)]
    right = [t.norm for t in tokenize(a.right)]
    original = a.word.lower() if a.word else "<blank>"
    seq = [BOS] * (m.order - 1) + left + [original] + right + [EOS]
    pos = m.order - 1 + len(left)
    for s in Checker(m, k=a.k).suggest(seq, pos):
        print(f"{s['word']:<20} {s['source']:<8} {s['gain_bits']:+6.1f} bits   seen in this frame {s['seen_in_this_frame']}x")


def cmd_complete(a):
    from .text import tokenize

    m = _load(a.index)
    hist = [t.norm for t in tokenize(a.prefix)]
    for w, p in m.continuations(hist, k=a.k):
        print(f"{w:<20} {p:.4f}")


def cmd_kwic(a):
    m = _load(a.index)
    pat = re.compile(rf"(?<!\w){re.escape(a.phrase)}(?!\w)", re.I)
    hits = 0
    for i, s in enumerate(m.sentences):
        for mt in pat.finditer(s):
            lo = max(0, mt.start() - a.width)
            hi = min(len(s), mt.end() + a.width)
            print(f"{s[lo:mt.start()]:>{a.width}} [{mt.group()}] {s[mt.end():hi]}")
            hits += 1
            break
        if hits >= a.n:
            break
    total = sum(len(pat.findall(s)) for s in m.sentences)
    print(f"-- {total} occurrences in your corpus ({m.count(a.phrase.lower())} as a single token)")


def cmd_profile(a):
    from .profile import voice_card

    m = _load(a.index)
    card = voice_card(m, a.markers)
    if a.output:
        Path(a.output).write_text(card, encoding="utf-8")
        print(f"Wrote {a.output}")
    else:
        print(card)


def cmd_info(a):
    m = _load(a.index)
    print(json.dumps({"meta": m.meta, "calibration": m.calib}, indent=2))


def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8")
        except Exception:
            pass

    p = argparse.ArgumentParser(prog="voiceprint", description="A statistical model of your own writing.",
                                fromfile_prefix_chars="@")  # voiceprint build @sources.txt
    sub = p.add_subparsers(dest="cmd", required=True)

    def with_index(sp):
        sp.add_argument("-i", "--index", default=str(DEFAULT_INDEX),
                        help=f"index file (default: $VOICEPRINT_INDEX or {DEFAULT_INDEX})")
        return sp

    b = sub.add_parser("build", help="build an index from your writing")
    b.add_argument("paths", nargs="+", help="files or folders (.tex .pdf .md .txt .docx .rst)")
    b.add_argument("-o", "--output", default=str(DEFAULT_INDEX))
    b.add_argument("--order", type=int, default=4, help="n-gram order (default 4)")
    b.add_argument("--min-words", type=int, default=100, help="skip documents shorter than this")
    b.add_argument("--exclude", action="append", default=[], metavar="GLOB",
                   help="skip files whose name matches (repeatable), e.g. --exclude 'draft*'")
    b.add_argument("--dump-clean", metavar="DIR", help="write cleaned text here to inspect preprocessing")
    b.set_defaults(fn=cmd_build)

    c = with_index(sub.add_parser("check", help="flag what in a draft doesn't sound like you"))
    c.add_argument("file", help="draft file, or - for stdin")
    c.add_argument("--json", action="store_true", help="machine-readable output (for agents)")
    c.add_argument("--annotate", action="store_true", help="print the draft with [[word -> alternatives]] inline")
    c.add_argument("--kind", choices=["md", "tex"], help="markup to mask (default: from extension)")
    c.add_argument("--markers", help="custom LLM-marker list")
    c.add_argument("--margin", type=float, default=3.0,
                   help="bits a replacement must gain to raise a context flag (default 3 = 8x likelier)")
    c.add_argument("--suggestions", type=int, default=5)
    c.add_argument("--max-flags", type=int, default=60)
    c.add_argument("--fail-on", choices=["high", "medium", "low", "never"], default="never",
                   help="exit 1 if any flag at this severity or above (for hooks/CI)")
    c.set_defaults(fn=cmd_check)

    s = with_index(sub.add_parser("suggest", help="what would you put in this slot?"))
    s.add_argument("--left", required=True, help="text before the slot")
    s.add_argument("--right", default="", help="text after the slot")
    s.add_argument("--word", help="the current word in the slot, to compare against")
    s.add_argument("-k", type=int, default=10)
    s.set_defaults(fn=cmd_suggest)

    co = with_index(sub.add_parser("complete", help="autocomplete the next token from your writing"))
    co.add_argument("prefix")
    co.add_argument("-k", type=int, default=10)
    co.set_defaults(fn=cmd_complete)

    k = with_index(sub.add_parser("kwic", help="show how you use a word or phrase"))
    k.add_argument("phrase")
    k.add_argument("-n", type=int, default=15)
    k.add_argument("--width", type=int, default=60)
    k.set_defaults(fn=cmd_kwic)

    pr = with_index(sub.add_parser("profile", help="write a voice card (markdown) summarizing your style"))
    pr.add_argument("-o", "--output")
    pr.add_argument("--markers")
    pr.set_defaults(fn=cmd_profile)

    inf = with_index(sub.add_parser("info", help="index metadata and calibration"))
    inf.set_defaults(fn=cmd_info)

    a = p.parse_args(argv)
    a.fn(a)


if __name__ == "__main__":
    main()
