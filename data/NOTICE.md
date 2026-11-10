# Bundled data

The voiceprint binary embeds the files in this folder.

| File | Source | License |
|---|---|---|
| `en_zipf.tsv.gz` | English word frequencies from [wordfreq](https://github.com/rspeer/wordfreq) by Robyn Speer, exported by `python/scripts/export_data.py` | [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) |
| `en_synonyms.tsv.gz` | Synonyms derived from [WordNet 3.0](https://wordnet.princeton.edu/) (Princeton University) plus `plain_alternatives.txt` | WordNet 3.0 license (below) for the WordNet-derived part; the rest is under the project license |
| `llm_markers.txt`, `plain_alternatives.txt` | Written for this project. The marker list draws on the Antislop work (Paech et al., 2025) and community lists | Project license |

`en_zipf.tsv.gz` is an adaptation of the wordfreq data and is shared under the same
CC BY-SA 4.0 license. wordfreq's own sources are listed in its README.

## WordNet 3.0 license

WordNet Release 3.0. This software and database is being provided to you, the LICENSEE, by
Princeton University under the following license. By obtaining, using and/or copying this
software and database, you agree that you have read, understood, and will comply with these
terms and conditions:

Permission to use, copy, modify and distribute this software and database and its
documentation for any purpose and without fee or royalty is hereby granted, provided that you
agree to comply with the following copyright notice and statements, including the disclaimer,
and that the same appear on ALL copies of the software, database and documentation, including
modifications that you make for internal use or for distribution.

WordNet 3.0 Copyright 2006 by Princeton University. All rights reserved.

THIS SOFTWARE AND DATABASE IS PROVIDED "AS IS" AND PRINCETON UNIVERSITY MAKES NO
REPRESENTATIONS OR WARRANTIES, EXPRESS OR IMPLIED. BY WAY OF EXAMPLE, BUT NOT LIMITATION,
PRINCETON UNIVERSITY MAKES NO REPRESENTATIONS OR WARRANTIES OF MERCHANT- ABILITY OR FITNESS FOR
ANY PARTICULAR PURPOSE OR THAT THE USE OF THE LICENSED SOFTWARE, DATABASE OR DOCUMENTATION WILL
NOT INFRINGE ANY THIRD PARTY PATENTS, COPYRIGHTS, TRADEMARKS OR OTHER RIGHTS.

The name of Princeton University or Princeton may not be used in advertising or publicity
pertaining to distribution of the software and/or database. Title to copyright in this
software, database and any associated documentation shall at all times remain with Princeton
University and LICENSEE agrees to preserve same.
