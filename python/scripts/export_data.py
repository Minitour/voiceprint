"""Export the data files the Rust binary bundles (../../data/).

    python python/scripts/export_data.py

* en_zipf.tsv.gz      word<TAB>zipf for every entry in wordfreq's English list
                      (CC BY-SA 4.0, see data/NOTICE.md)
* en_synonyms.tsv.gz  word<TAB>alt|alt|... from the reference implementation's
                      ``synonyms()``: curated map + WordNet, inflected like the word
"""
from __future__ import annotations

import gzip
import math
import sys
import time
from pathlib import Path

import wordfreq

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from voiceprint.alternatives import curated, synonyms, wordnet_available  # noqa: E402
from voiceprint.check import load_markers  # noqa: E402

OUT = Path(__file__).resolve().parents[2] / "data"
SYNONYM_MIN_ZIPF = 2.0


def main() -> None:
    OUT.mkdir(exist_ok=True)
    if not wordnet_available():
        sys.exit("WordNet is not available: python -m nltk.downloader wordnet")

    freqs = wordfreq.get_frequency_dict("en", wordlist="best")
    zipf = {w: round(math.log10(f) + 9, 2) for w, f in freqs.items()}
    with gzip.open(OUT / "en_zipf.tsv.gz", "wt", encoding="utf-8", compresslevel=9) as f:
        for w, z in sorted(zipf.items(), key=lambda x: (-x[1], x[0])):
            if "\t" not in w and "\n" not in w:
                f.write(f"{w}\t{z:.2f}\n")
    print(f"en_zipf: {len(zipf):,} words")

    marker_words, _ = load_markers()
    heads = {w for w, z in zipf.items() if z >= SYNONYM_MIN_ZIPF and w.replace("-", "").replace("'", "").isalpha()}
    heads |= set(curated()) | marker_words
    t0 = time.time()
    rows = 0
    with gzip.open(OUT / "en_synonyms.tsv.gz", "wt", encoding="utf-8", compresslevel=9) as f:
        for w in sorted(heads):
            alts = [a for a in synonyms(w) if " " in a or zipf.get(a, 0) > 0]
            if alts:
                f.write(f"{w}\t{'|'.join(alts)}\n")
                rows += 1
    print(f"en_synonyms: {rows:,} words with alternatives ({time.time() - t0:.0f}s)")


if __name__ == "__main__":
    main()
