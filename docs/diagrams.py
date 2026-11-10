"""Generate the README diagrams (docs/*.svg).

    python docs/diagrams.py

Plain SVG with an embedded stylesheet, so GitHub renders it as an image.
Every diagram draws its own white card, so it reads the same in light and dark mode.
"""
from __future__ import annotations

from html import escape
from pathlib import Path

OUT = Path(__file__).parent

SANS = "-apple-system, 'Segoe UI', Helvetica, Arial, sans-serif"
MONO = "ui-monospace, SFMono-Regular, Consolas, 'Liberation Mono', monospace"

C = {
    "ink": "#0f172a", "muted": "#64748b", "line": "#e2e8f0", "card": "#ffffff",
    "blue": "#2563eb", "blue_bg": "#eff6ff", "blue_ln": "#bfdbfe",
    "amber": "#b45309", "amber_bg": "#fffbeb", "amber_ln": "#fcd34d",
    "red": "#dc2626", "red_bg": "#fef2f2", "red_ln": "#fecaca",
    "green": "#15803d", "green_bg": "#f0fdf4", "green_ln": "#bbf7d0",
    "slate_bg": "#f8fafc", "slate_ln": "#cbd5e1", "gray_bar": "#cbd5e1",
}

STYLE = f"""
text {{ font-family: {SANS}; fill: {C['ink']}; }}
.mono {{ font-family: {MONO}; }}
.muted {{ fill: {C['muted']}; }}
.b {{ font-weight: 700; }}
.sb {{ font-weight: 600; }}
.title {{ font-size: 26px; font-weight: 750; }}
.sub {{ font-size: 14px; fill: {C['muted']}; }}
.h {{ font-size: 15px; font-weight: 700; }}
.lane {{ font-size: 12px; font-weight: 700; letter-spacing: .08em; }}
.arrow {{ stroke: {C['ink']}; stroke-width: 1.6; fill: none; }}
.arrow-m {{ stroke: {C['muted']}; stroke-width: 1.4; fill: none; }}
"""


class Svg:
    def __init__(self, w: int, h: int, title: str, desc: str):
        self.w, self.h = w, h
        self.parts: list[str] = []
        self.head = (
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" '
            f'role="img" aria-labelledby="t d"><title id="t">{escape(title)}</title><desc id="d">{escape(desc)}</desc>'
            f"<style>{STYLE}</style>"
            "<defs>"
            f'<marker id="ah" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse">'
            f'<path d="M0,0 L10,5 L0,10 z" fill="{C["ink"]}"/></marker>'
            f'<marker id="ahm" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">'
            f'<path d="M0,0 L10,5 L0,10 z" fill="{C["muted"]}"/></marker>'
            "</defs>"
            f'<rect x="1" y="1" width="{w - 2}" height="{h - 2}" rx="18" fill="{C["card"]}" stroke="{C["line"]}"/>'
        )

    def add(self, s: str) -> None:
        self.parts.append(s)

    def rect(self, x, y, w, h, fill, stroke, rx=10, sw=1, dash=None):
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{fill}" stroke="{stroke}" stroke-width="{sw}"{d}/>')

    def text(self, x, y, s, size=13, cls="", anchor="start", fill=None, weight=None, italic=False):
        attrs = f' class="{cls}"' if cls else ""
        if fill:
            attrs += f' fill="{fill}" style="fill:{fill}"'
        if weight:
            attrs += f' font-weight="{weight}"'
        if italic:
            attrs += ' font-style="italic"'
        self.add(f'<text x="{x}" y="{y}" font-size="{size}" text-anchor="{anchor}"{attrs}>{escape(s)}</text>')

    def rich(self, x, y, runs, size=13, anchor="start"):
        """runs: list of (text, dict(cls=, fill=, weight=, strike=))"""
        spans = []
        for t, o in runs:
            a = ""
            if o.get("cls"):
                a += f' class="{o["cls"]}"'
            if o.get("fill"):
                a += f' style="fill:{o["fill"]}"'
            if o.get("weight"):
                a += f' font-weight="{o["weight"]}"'
            if o.get("strike"):
                a += ' text-decoration="line-through"'
            if o.get("italic"):
                a += ' font-style="italic"'
            spans.append(f"<tspan{a}>{escape(t)}</tspan>")
        self.add(f'<text x="{x}" y="{y}" font-size="{size}" text-anchor="{anchor}" xml:space="preserve">{"".join(spans)}</text>')

    def line(self, x1, y1, x2, y2, stroke, sw=1, dash=None):
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.add(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{stroke}" stroke-width="{sw}"{d}/>')

    def arrow(self, d, muted=False):
        cls, m = ("arrow-m", "ahm") if muted else ("arrow", "ah")
        self.add(f'<path d="{d}" class="{cls}" marker-end="url(#{m})"/>')

    def chip(self, x, y, label, fill, stroke, color, size=12, mono=True, h=24, pad=8):
        w = chip_w(label, size, mono, pad)
        self.rect(x, y, w, h, fill, stroke, rx=6)
        self.text(x + w / 2, y + h / 2 + size * 0.36, label, size, "mono" if mono else "", "middle", fill=color)
        return x + w

    def badge(self, x, y, n, color):
        self.add(f'<circle cx="{x}" cy="{y}" r="13" fill="{color}"/>')
        self.text(x, y + 4.5, str(n), 13, "b", "middle", fill="#ffffff")

    def save(self, name: str) -> None:
        (OUT / name).write_text(self.head + "".join(self.parts) + "</svg>\n", encoding="utf-8")
        print(f"wrote docs/{name}")


def chip_w(label, size=12, mono=True, pad=8):
    return round(len(label) * size * (0.61 if mono else 0.56) + 2 * pad)


def panel(s: Svg, x, y, w, h, n, title, color):
    s.rect(x, y, w, h, C["card"], C["line"], rx=14)
    s.badge(x + 26, y + 27, n, color)
    s.text(x + 48, y + 32, title, 15, "h")


# ----------------------------------------------------------------------
# 1. How it works (overview)
# ----------------------------------------------------------------------

def how_it_works():
    s = Svg(1200, 680, "How voiceprint works",
            "Build a voice index from your writing once, then check each agent draft against it, "
            "revise the flagged words using your own vocabulary, and re-check until it passes.")
    s.text(40, 52, "How voiceprint works", cls="title")
    s.text(40, 78, "Build a model of your writing once. Check every draft an agent writes against it.", cls="sub")

    # Lane 1: build
    s.rect(24, 100, 1152, 196, C["blue_bg"], C["blue_ln"], rx=14)
    s.text(44, 126, "1  BUILD  ·  ONCE", cls="lane", fill=C["blue"])

    # Your writing
    s.rect(44, 142, 196, 134, C["card"], C["blue_ln"], rx=12)
    for i, dx in enumerate((0, 6, 12)):
        s.rect(64 + dx, 156 + dx, 26, 32, "#ffffff", C["slate_ln"], rx=3)
    s.text(108, 172, "Your writing", 15, "h")
    s.text(108, 191, "anything you wrote", 12, "muted")
    x = 60
    for ext in (".md", ".txt", ".docx"):
        x = s.chip(x, 212, ext, C["slate_bg"], C["slate_ln"], C["muted"], size=11, h=20, pad=6) + 6
    x = 60
    for ext in (".pdf", ".tex", ".rst"):
        x = s.chip(x, 238, ext, C["slate_bg"], C["slate_ln"], C["muted"], size=11, h=20, pad=6) + 6

    # Clean & tokenize
    s.rect(276, 142, 262, 134, C["card"], C["blue_ln"], rx=12)
    s.text(292, 168, "Clean & tokenize", 15, "h")
    s.text(292, 190, "before", 11, "muted")
    s.text(292, 207, 'Read **notes** 2 at https://x.io', 12, "mono")
    s.text(292, 232, "after", 11, "muted")
    s.rich(292, 249, [("read notes ", {"cls": "mono"}), ("<num>", {"cls": "mono", "fill": C["blue"]}),
                      (" at ", {"cls": "mono"}), ("<url>", {"cls": "mono", "fill": C["blue"]})], 12)
    s.text(292, 266, "markup, code, links and numbers removed", 11, "muted")

    # Learn
    s.rect(574, 142, 336, 134, C["card"], C["blue_ln"], rx=12)
    s.text(590, 168, "Learn your habits", 15, "h")
    rows = [("Word sequences", "which word you put after which"),
            ("Style", "sentence length, punctuation, we/I"),
            ("Calibration", "how surprising your own new text is")]
    for i, (a, b) in enumerate(rows):
        y = 196 + i * 26
        s.add(f'<circle cx="596" cy="{y - 4}" r="4" fill="{C["blue"]}"/>')
        s.rich(608, y, [(a, {"weight": "600"}), ("  " + b, {"fill": C["muted"]})], 12.5)

    # Voice index cylinder
    cx, top, bot = 1046, 156, 254
    s.add(f'<path d="M{cx - 96},{top} V{bot} A96,14 0 0 0 {cx + 96},{bot} V{top}" fill="{C["blue"]}" />')
    s.add(f'<ellipse cx="{cx}" cy="{top}" rx="96" ry="14" fill="#3b82f6"/>')
    s.text(cx, 206, "voice.vpi", 17, "mono b", "middle", fill="#ffffff")
    s.text(cx, 228, "your voice index", 12.5, "", "middle", fill="#dbeafe")

    s.arrow("M240,209 H272")
    s.arrow("M538,209 H570")
    s.arrow("M910,209 H946")

    # Lane 2: check
    s.rect(24, 316, 1152, 340, C["amber_bg"], C["amber_ln"], rx=14)
    s.text(44, 342, "2  CHECK  ·  EVERY DRAFT", cls="lane", fill=C["amber"])

    # index feeds the check
    s.arrow("M1046,270 V300 H471 V362", muted=True)
    s.text(760, 293, "compares against your voice index", 11.5, "muted", "middle")

    # Draft
    s.rect(44, 362, 248, 124, C["card"], C["amber_ln"], rx=12)
    s.text(60, 388, "Agent writes a draft", 15, "h")
    for i, ln in enumerate(["“Furthermore, this update", "seamlessly leverages a", "robust cache.”"]):
        s.text(60, 414 + i * 20, ln, 13.5, italic=True)

    # Check
    s.rect(330, 362, 282, 124, "#fff7ed", "#f59e0b", rx=12, sw=2)
    s.text(471, 390, "voiceprint check", 16, "mono b", "middle")
    s.text(471, 414, "for every word in the draft:", 12.5, "muted", "middle")
    s.text(471, 434, "would you have written it,", 13, "sb", "middle")
    s.text(471, 454, "here, this often?", 13, "sb", "middle")
    s.arrow("M292,424 H326")

    # General English
    s.rect(330, 528, 282, 52, C["card"], C["slate_ln"], rx=10)
    s.text(471, 550, "General English word frequencies", 12.5, "sb", "middle")
    s.text(471, 568, "separates “you avoid it” from “it’s just rare”", 11.5, "muted", "middle")
    s.arrow("M471,528 V490", muted=True)

    # Flags
    s.rect(650, 362, 318, 186, C["card"], C["red_ln"], rx=12)
    s.text(666, 388, "Flags, fixed with your own words", 15, "h")

    def flag(y, sev, word, alt, sev_col, sev_bg):
        s.rect(666, y - 14, 46, 19, sev_bg, sev_col, rx=5)
        s.text(689, y, sev, 10.5, "b", "middle", fill=sev_col)
        s.rich(722, y, [(word, {"strike": True, "fill": C["red"]}), ("  →  ", {"fill": C["muted"]}),
                        (alt, {"weight": "600", "fill": C["green"]})], 13)

    flag(416, "HIGH", "Furthermore,", "(delete)", C["red"], C["red_bg"])
    flag(442, "HIGH", "seamlessly", "simply", C["red"], C["red_bg"])
    flag(468, "HIGH", "leverages", "uses", C["red"], C["red_bg"])
    flag(494, "LOW", "robust", "simple", C["amber"], C["amber_bg"])
    s.line(666, 510, 952, 510, C["line"])
    s.rich(666, 532, [("verdict  ", {"fill": C["muted"]}), ("does not sound like you", {"weight": "700", "fill": C["red"]})], 13)
    s.arrow("M612,424 H646")

    # Revise loop
    s.arrow("M809,548 V612 H168 V490")
    s.rect(390, 600, 196, 24, C["amber_bg"], C["amber_bg"], rx=4)
    s.text(488, 617, "agent revises, then re-checks", 12.5, "sb", "middle", fill=C["amber"])

    # Pass
    s.rect(1004, 362, 156, 186, C["green_bg"], C["green_ln"], rx=12)
    s.text(1082, 392, "Pass ✓", 17, "b", "middle", fill=C["green"])
    s.text(1082, 412, "sounds like you", 12.5, "muted", "middle")
    for i, ln in enumerate(["“This update", "uses a simple", "cache.”"]):
        s.text(1082, 446 + i * 20, ln, 13.5, "", "middle", italic=True)
    s.text(1082, 532, "0 high flags", 11.5, "muted", "middle")
    s.arrow("M968,455 H1000")

    s.save("how-it-works.svg")


# ----------------------------------------------------------------------
# 2. How it learns
# ----------------------------------------------------------------------

def how_it_learns():
    s = Svg(1200, 1010, "How voiceprint learns your voiceprint",
            "Clean your text, pad sentences, slide a 4-word window, count what follows each context, "
            "blend long and short contexts with Kneser-Ney smoothing, compare your word use with general "
            "English, profile your style, and calibrate on held-out documents. Everything is saved to voice.vpi.")
    s.text(40, 52, "How voiceprint learns your voiceprint", cls="title")
    s.text(40, 78, "Following one sentence through voiceprint build. Numbers are illustrative.", cls="sub")

    blue, amber, green, red = C["blue"], C["amber"], C["green"], C["red"]

    # 1 Clean
    panel(s, 32, 100, 340, 240, 1, "Read and clean", blue)
    s.text(50, 150, "before", 11, "muted")
    s.rect(48, 156, 308, 54, C["slate_bg"], C["line"], rx=8)
    s.text(58, 177, "We then ship the **fix** to all 40", 11.5, "mono")
    s.text(58, 197, "users. Details: https://x.io", 11.5, "mono")
    s.text(50, 232, "after", 11, "muted")
    s.rect(48, 238, 308, 54, C["green_bg"], C["green_ln"], rx=8)
    s.rich(58, 259, [("we then ship the fix to all ", {"cls": "mono"}), ("<num>", {"cls": "mono", "fill": blue})], 11.5)
    s.rich(58, 279, [("users . details : ", {"cls": "mono"}), ("<url>", {"cls": "mono", "fill": blue})], 11.5)
    s.text(50, 318, "Markup, code, links and numbers are stripped", 11.5, "muted")
    s.text(50, 333, "or replaced, so only your wording is left.", 11.5, "muted")

    # 2 Sentences
    panel(s, 392, 100, 360, 240, 2, "Split into sentences, pad the start", blue)
    toks1 = ["<s>", "<s>", "<s>", "we", "then", "ship"]
    toks2 = ["the", "fix", "to", "everyone", ".", "</s>"]
    for row, toks in ((150, toks1), (182, toks2)):
        x = 410
        for t in toks:
            pad = t in ("<s>", "</s>", ".")
            x = s.chip(x, row, t, C["slate_bg"] if pad else C["blue_bg"], C["slate_ln"] if pad else C["blue_ln"],
                       C["muted"] if pad else blue) + 6
    s.text(410, 240, "Lowercased tokens, one sentence at a time.", 12, "muted")
    s.rich(410, 262, [("<s>", {"cls": "mono", "fill": C["muted"]}),
                      (" markers mean even the first word has a", {"fill": C["muted"]})], 12)
    s.text(410, 280, "context, so it learns how you open sentences.", 12, "muted")
    s.rich(410, 310, [("</s>", {"cls": "mono", "fill": C["muted"]}),
                      (" teaches it how you end them.", {"fill": C["muted"]})], 12)

    # 3 Window
    panel(s, 772, 100, 396, 240, 3, "Slide a 4-word window", blue)
    windows = [(["<s>", "<s>", "<s>"], "we"), (["<s>", "<s>", "we"], "then"), (["<s>", "we", "then"], "ship"),
               (["we", "then", "ship"], "the"), (["then", "ship", "the"], "fix")]
    for i, (ctx, nxt) in enumerate(windows):
        y = 144 + i * 31
        x = 790
        for t in ctx:
            x = s.chip(x, y, t, C["slate_bg"], C["slate_ln"], C["ink"], size=11.5, h=23, pad=7) + 5
        s.text(x + 6, y + 16, "→", 14, "b", fill=C["muted"])
        s.chip(x + 26, y, nxt, C["amber_bg"], C["amber_ln"], amber, size=11.5, h=23, pad=7)
    s.text(790, 322, "Each 3-word context → the word you wrote next.", 12, "muted")

    s.arrow("M372,220 H388")
    s.arrow("M752,220 H768")

    # 4 Count (right)
    panel(s, 612, 364, 556, 262, 4, "Count what follows each context", blue)
    ctxs = [("we then ship", [("the", 6), ("it", 3), ("a", 1)]),
            ("then ship", [("the", 9), ("it", 4), ("them", 2)]),
            ("ship", [("the", 11), ("it", 6), ("a", 3)])]
    for i, (ctx, items) in enumerate(ctxs):
        y = 418 + i * 38
        s.text(812, y, ctx + "  ▸", 12, "mono", "end", fill=C["ink"])
        for j, (w, n) in enumerate(items):
            sx = 828 + j * 112
            s.text(sx, y, w, 12, "mono")
            bx = sx + chip_w(w, 12, True, 0) + 6
            s.rect(bx, y - 10, n * 4.5, 11, blue if j == 0 else "#93c5fd", "none", rx=2)
            s.text(bx + n * 4.5 + 5, y, str(n), 11.5, "muted")
    s.line(630, 520, 1150, 520, C["line"])
    s.text(630, 544, "The full 3-word context keeps raw counts. Shorter contexts count how many", 12, "muted")
    s.text(630, 562, "different contexts a word completes (Kneser-Ney continuation counts).", 12, "muted")
    s.rich(630, 594, [("Also kept: a slot index  ", {"fill": C["muted"]}),
                      ("(the ▢ to) → fix, update, …", {"cls": "mono", "fill": C["ink"]})], 12)
    s.text(630, 612, "Suggestions for a flagged word come from here.", 12, "muted")
    s.arrow("M970,340 V360")

    # 5 Blend (left)
    panel(s, 32, 364, 560, 262, 5, "Blend long and short contexts", blue)
    s.text(52, 430, "P(the | we then ship)", 13.5, "mono b")
    s.rich(76, 458, [("= ", {"cls": "mono"}), ("(6 − D) / 10", {"cls": "mono", "fill": green, "weight": "700"}),
                     ("  +  λ × ", {"cls": "mono"}), ("P(the | then ship)", {"cls": "mono", "fill": blue})], 13)
    s.text(76, 476, "your own counts, discounted       weight left over for shorter contexts", 10.5, "muted")
    s.rich(100, 506, [("↳ ", {"fill": C["muted"]}), ("P(the | then ship)", {"cls": "mono", "fill": blue}),
                      (" = … + λ × P(the | ship)", {"cls": "mono"})], 12.5)
    s.rich(124, 532, [("↳ ", {"fill": C["muted"]}), ("P(the | ship)", {"cls": "mono", "fill": blue}),
                      (" = … + λ × P(the)", {"cls": "mono"})], 12.5)
    s.rich(148, 558, [("↳ ", {"fill": C["muted"]}), ("P(the)", {"cls": "mono", "fill": blue}),
                      (" = share of contexts “the” completes", {"cls": "mono"})], 12.5)
    s.line(50, 578, 574, 578, C["line"])
    s.text(52, 602, "Contexts you've used before mostly reuse your own counts. New contexts fall back", 12, "muted")
    s.text(52, 620, "to shorter ones. Nothing is ever zero, so every word in a draft gets a score.", 12, "muted")
    s.arrow("M612,495 H596")

    # 6 English
    panel(s, 32, 650, 368, 244, 6, "Compare with general English", blue)
    s.rect(52, 698, 10, 10, blue, "none", rx=2)
    s.text(66, 707, "you", 11.5, "muted")
    s.rect(98, 698, 10, 10, C["gray_bar"], "none", rx=2)
    s.text(112, 707, "general English", 11.5, "muted")
    words = [("thus", 8.0, 1.1), ("however", 14.7, 3.0), ("really", 0.0, 9.6), ("delve", 0.0, 0.3)]
    for i, (w, you, eng) in enumerate(words):
        y = 738 + i * 30
        s.text(116, y + 4, w, 12, "mono", "end")
        s.rect(126, y - 9, max(you * 10, 1.5), 9, blue, "none", rx=2)
        s.text(130 + you * 10, y - 1, f"{you:g}", 10.5, "muted")
        s.rect(126, y + 2, max(eng * 10, 1.5), 9, C["gray_bar"], "none", rx=2)
        s.text(130 + eng * 10, y + 10, f"{eng:g}", 10.5, "muted")
    s.text(52, 858, "per 10k words. “really”: common in English, never", 11.5, "muted")
    s.text(52, 874, "used by you → flagged when a draft uses it.", 11.5, "muted")

    # 7 Style
    panel(s, 416, 650, 368, 244, 7, "Learn your style habits", blue)
    heights = [6, 14, 28, 44, 58, 54, 44, 32, 22, 14, 8, 4]
    base = 790
    s.rect(436 + 2 * 24 - 3, 690, 7 * 24 + 2, base - 690, C["blue_bg"], "none", rx=4)
    s.text(436 + 2 * 24 + 1, 702, "your typical range", 10.5, "", fill=blue)
    for i, h in enumerate(heights):
        s.rect(436 + i * 24, base - h * 1.4, 18, h * 1.4, blue if 2 <= i <= 8 else "#93c5fd", "none", rx=2)
    s.line(432, base, 724, base, C["slate_ln"])
    s.text(436, 806, "sentence length (words): 5", 10.5, "muted")
    s.text(722, 806, "40", 10.5, "muted", "end")
    x = 434
    for lab in ("em dashes 0 / 1k", "we 11 / 1k", "contractions 0"):
        x = s.chip(x, 822, lab, C["slate_bg"], C["slate_ln"], C["ink"], size=10.5, mono=False, h=22, pad=7) + 6
    s.text(434, 874, "Measured per ~300-word chunk: mean and range.", 11.5, "muted")

    # 8 Calibration
    panel(s, 800, 650, 368, 244, 8, "Calibrate on unseen text", blue)
    for r in range(5):
        for c in range(5):
            held = r == c
            s.rect(820 + c * 20, 690 + r * 20, 15, 15, C["amber_ln"] if held else C["blue_ln"],
                   amber if held else blue, rx=3, sw=1 if not held else 1.5)
    s.text(934, 704, "Train on 4 documents,", 12, "")
    s.text(934, 722, "score the 5th.", 12, "")
    s.text(934, 740, "Repeat until every", 12, "muted")
    s.text(934, 758, "document is scored.", 12, "muted")
    curve = ("M820,846 C850,846 862,792 884,790 C908,788 922,820 960,832 "
             "C1000,842 1060,844 1150,846 Z")
    s.add(f'<path d="{curve}" fill="{C["blue_bg"]}" stroke="{blue}" stroke-width="1.5"/>')
    s.line(1058, 796, 1058, 848, red, 1.5, "4 3")
    s.text(1064, 808, "p95", 11.5, "b", fill=red)
    s.line(818, 846, 1152, 846, C["slate_ln"])
    s.text(820, 866, "How surprising your own unseen words are.", 11.5, "muted")
    s.text(820, 882, "Past p95 = unusual for you.", 11.5, "muted")

    s.arrow("M216,626 V646")

    # Footer: the index
    s.rect(32, 922, 1136, 62, C["blue_bg"], C["blue_ln"], rx=14)
    cx = 74
    s.add(f'<path d="M{cx - 18},938 V966 A18,5 0 0 0 {cx + 18},966 V938" fill="{blue}"/>')
    s.add(f'<ellipse cx="{cx}" cy="938" rx="18" ry="5" fill="#3b82f6"/>')
    s.rich(106, 950, [("Saved to one file, ", {"weight": "600"}), ("voice.vpi", {"cls": "mono", "weight": "700", "fill": blue})], 15)
    s.text(106, 972, "n-gram tables  ·  slot index  ·  your word counts  ·  style profile  ·  "
                     "calibration  ·  your sentences (for kwic)", 12.5, "muted")
    for x in (216, 600, 984):
        s.arrow(f"M{x},894 V918")

    s.save("how-it-learns.svg")


if __name__ == "__main__":
    how_it_works()
    how_it_learns()
