# Test fixtures

| File | What it is | Source and license |
|---|---|---|
| `real_heldout.adoc` | The opening of Git's v2.54 release notes, used as held-out "real author" text | [git/git](https://github.com/git/git), `Documentation/RelNotes/2.54.0.adoc`, GPL-2.0 |
| `llm_rewrite.md` | An LLM-style rewrite of the same items, used as the "not this author" counterpart | Written for this project; it paraphrases the GPL-2.0 release notes above |

The validation in the main README trains on Git's other release notes, which ship with
Git for Windows in `mingw64/share/doc/git-doc/RelNotes`; they aren't included here.
