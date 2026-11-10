---
name: write-like-me
description: Make prose sound like the user by checking drafts against `voiceprint`, a statistical n-gram model built from text they wrote, and revising what it flags. Use whenever writing or editing prose that will go out under the user's name (documents, emails, posts, READMEs, messages), or when the user asks for text that "sounds like me" or "in my voice".
---

# Write like me

`voiceprint` is a local CLI holding an n-gram model of the user's own writing.
It cannot steer your generation; it audits text after you write it. Instructions
here are the soft part. The `check` loop is the hard part: don't skip it.

The index lives at `$VOICEPRINT_INDEX` (default `~/.voiceprint/voice.vpi`).
If there is no index, stop and tell the user to build one
(`voiceprint build @sources.txt -o ~/.voiceprint/voice.vpi`, listing only files the user wrote).
Don't build one from text that the user didn't write.

## How to call it

When the project is managed by capa (it has a `capabilities.yaml` with a `voiceprint` tool
group), use `capa sh`. Otherwise, use the `voiceprint` CLI directly. If it isn't installed,
`npx voiceprint <command>` runs it without installing (or `npm install -g voiceprint`).

| Step | capa | CLI |
|---|---|---|
| Check a draft | `capa sh voiceprint check-draft --file <ABSOLUTE path>` | `voiceprint check draft.md --json` |
| Voice card | `capa sh voiceprint voice-card` | `voiceprint profile` |
| How the user uses a phrase | `capa sh voiceprint word-usage --phrase "in particular" --n 10` | `voiceprint kwic "in particular"` |
| What they'd put in a slot | `capa sh voiceprint suggest-word --left "results are" --word crucial --right "for deployment"` | `voiceprint suggest --left ... --word ... --right ...` |

Always pass an absolute path to `check-draft`; capa runs the tool from its own working directory.

## 1. Before drafting: read the voice card

Get the voice card (see the table above), or read `~/.voiceprint/voice-card.md` if it exists. Write to its style norms: sentence length,
punctuation habits, the transitions they actually use, whether they use "we"/"I",
and contractions. Avoid the "LLM-marker words you never use" list. The markers
listed as "genuinely used" are fine.

## 2. Draft

Write the draft to a file (e.g. `draft.md`), not just into chat, so it can be checked.

## 3. Check

Run the check step (see the table above). The JSON output contains:
- `summary.verdict`: `close to your voice` / `drifting` / `does not sound like you`
- `style[]`: features where `off: true` are outside the user's typical range
- `flags[]`: each has `kind`, `severity`, `text`, `line`, `col`, `reason`, and `suggestions`
  (`word`, `source`, `gain_bits`). A positive `gain_bits` means the user's model
  finds the local wording that many bits more likely (+3 is 8x).

## 4. Revise flag by flag, using judgment

The model knows the user's habits, not meaning. You supply the meaning.

- **high, marker/phrase**: always rewrite. Prefer a `synonym` suggestion that keeps
  the meaning. If none fits, rephrase the clause the way the voice card suggests,
  and don't swap in another marker word.
- **(delete)** suggestion: usually take it ("Furthermore, X" -> "X"), and repair the punctuation.
- **medium/low underused**: the user rarely uses this common word. Replace it if a
  synonym preserves the meaning. **Keep it** if it's a needed technical or topical term
  (new topics bring new words), a name, or a quoted identifier.
- **context**: the user would phrase this slot differently. Consider the synonym; ignore
  `same slot` (frame) candidates that change the meaning.
- **style off**: fix it structurally, e.g. split or merge sentences, remove em dashes the
  user never uses, add or remove "we".
- Before choosing a word, check how the user really uses it with the word-usage step, or ask
  what they would put in that slot with the suggest step.

Never trade correctness for voice. A technical term stays even if flagged.

## 5. Re-check until it passes

Re-run the check after each revision pass. You're done when there
are **0 high flags** and the verdict is `close to your voice`, or after **3 passes**.
If it still isn't passing by then, say what remains and why (usually technical
vocabulary the corpus has never seen).

## 6. Report

Tell the user the final verdict, the high/medium/low counts before and after,
and any flags you deliberately kept, with the reason.
