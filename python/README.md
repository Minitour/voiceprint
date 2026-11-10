# voiceprint (Python reference implementation)

This is the original Python implementation of voiceprint. It is **not published**;
the shipped tool is the Rust crate at the repository root (`npx voiceprint`).

It is kept as a reference:

- `scripts/export_data.py` generates the bundled data files in `../data/`
  (general-English word frequencies and the synonym table) from `wordfreq` and WordNet.
- Its results are the oracle for the Rust port's parity tests.

```bash
uv venv && uv pip install -e ".[dev]"
```
