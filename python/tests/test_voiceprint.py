import json
import random

import pytest

from voiceprint import cli
from voiceprint.check import Checker
from voiceprint.model import VoiceModel, build
from voiceprint.text import clean_tex, mask_markup, sentences, tokenize

SUBJECTS = ["the parser", "our method", "the agent", "this approach", "the runtime"]
VERBS = ["handles", "reduces", "checks", "tracks", "rewrites"]
OBJECTS = ["the input", "each request", "the error rate", "the queue", "every edge case"]
TAILS = ["in practice.", "without extra state.", "at low cost.", "as expected.", "in most runs."]


def corpus_docs(n_docs=6, n_sents=120, seed=0):
    rng = random.Random(seed)
    docs = []
    for d in range(n_docs):
        sents = []
        for _ in range(n_sents):
            s = f"{rng.choice(SUBJECTS).capitalize()} {rng.choice(VERBS)} {rng.choice(OBJECTS)} {rng.choice(TAILS)}"
            if rng.random() < 0.3:
                s = "However, " + s[0].lower() + s[1:]
            sents.append(s)
        docs.append(" ".join(sents))
    return docs


def build_model(tmp_path):
    texts = corpus_docs()
    docs = []
    for i, t in enumerate(texts):
        ss = sentences(t)
        docs.append((f"doc{i}", [[x.norm for x in s] for s in ss], [t[s[0].start:s[-1].end] for s in ss]))
    return build(docs, order=3)


def test_tokenize_normalizes():
    toks = [t.norm for t in tokenize("We don’t use 42 items, e.g. GPUs -- see --flag.")]
    assert "don't" in toks and "<num>" in toks and "e.g." in toks
    assert toks.count("—") == 1  # "--" is a dash, "--flag" is not


def test_sentence_split_ignores_inner_dots():
    ss = sentences('A driver for ".ini" files was added. It works.')
    assert len(ss) == 2


def test_mask_preserves_offsets():
    md = "# Title\n\nSome **bold** text with `code` and a [link](http://x.y).\n"
    masked = mask_markup(md)
    assert len(masked) == len(md)
    assert "bold" in masked and "code" not in masked and "http" not in masked


def test_clean_tex():
    tex = r"\begin{document}\section{Intro} We show \emph{this} \cite{a}. $x^2$ holds.\end{document}"
    out = clean_tex(tex)
    assert "Intro" in out and "this" in out and "<cite>" in out and "<math>" in out and "\\" not in out


def test_kn_is_a_distribution(tmp_path):
    m = build_model(tmp_path)
    vocab = list(m.tables[1][()][0])
    for hist in (["<s>", "<s>"], ["the", "parser"], ["never", "seen"]):
        total = sum(m.prob(w, hist) for w in vocab) + m.prob("zzz-unseen", hist)
        assert total == pytest.approx(1.0, abs=1e-6)


def test_check_flags_llm_words_but_not_own_style(tmp_path):
    m = build_model(tmp_path)
    ch = Checker(m)
    own = ch.check("However, the parser checks the input in practice. Our method tracks the queue as expected.")
    llm = ch.check("Furthermore, our method seamlessly leverages a comprehensive approach that plays a crucial role.")
    assert own["summary"]["flags"]["high"] == 0
    words = {f["text"].lower() for f in llm["flags"] if f["severity"] == "high"}
    assert {"furthermore", "seamlessly", "comprehensive"} <= words


def test_save_load_roundtrip(tmp_path):
    m = build_model(tmp_path)
    p = tmp_path / "v.vpi"
    m.save(p)
    m2 = VoiceModel.load(p)
    assert m2.prob("parser", ["<s>", "the"]) == pytest.approx(m.prob("parser", ["<s>", "the"]))


def test_cli_end_to_end(tmp_path, capsys):
    for i, t in enumerate(corpus_docs()):
        (tmp_path / f"p{i}.md").write_text(t, encoding="utf-8")
    idx = tmp_path / "voice.vpi"
    cli.main(["build", str(tmp_path), "-o", str(idx), "--order", "3"])
    draft = tmp_path / "draft.txt"
    draft.write_text("Moreover, the agent meticulously delves into the queue.", encoding="utf-8")
    capsys.readouterr()
    cli.main(["check", str(draft), "-i", str(idx), "--json"])
    report = json.loads(capsys.readouterr().out)
    assert report["summary"]["flags"]["high"] >= 2
    with pytest.raises(SystemExit):
        cli.main(["check", str(draft), "-i", str(idx), "--fail-on", "high"])


def test_pdf_repairs():
    from voiceprint.text import _fix_ligatures, split_glued

    assert split_glued("Iwouldalsoliketothanktheexperts") == "I would also like to thank the experts"
    assert split_glued("phytoplankton") == "phytoplankton"
    fixed = _fix_ligatures("We defne a domain-specifc fle; keep form, life, of.")
    assert fixed == "We define a domain-specific file; keep form, life, of."
