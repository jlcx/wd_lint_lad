# wikidata-lint / wikidata-fix

Two Rust binaries that scan a Wikidata JSON dump for issues in
**labels**, **descriptions**, and **aliases**, then turn the fixable
findings into a [QuickStatements v2 / CSV][qs] batch.

Detection rules (word lists, prefix lists, misspelling maps, QID/PID
skiplists, length thresholds, etc.) live in an external JSON rules file
so they can be tuned without recompiling. See
[`rules/en.json`](rules/en.json) for the canonical shape and
[`SPEC.md`](SPEC.md) for the detailed specification of every check.

```
                rules.json
                    │
                    ▼
  dump ──▶  wikidata-lint  ──▶  issues.jsonl  ──▶  wikidata-fix  ──▶  batch.csv
                                     │                  ▲
                                     │             rules.json
                                     ▼
                              (human review / grep / jq)
```

[qs]: https://www.wikidata.org/wiki/Help:QuickStatements

## Build

Requires Rust 1.85+ (edition 2024). The repo pins a stable toolchain
via `rust-toolchain.toml`.

```sh
cargo build --release
```

Binaries land at `target/release/wikidata-lint` and
`target/release/wikidata-fix`.

## `wikidata-lint` — the scanner

Streams a dump on stdin and emits one JSONL record per detected issue.

### Input format

The native Wikidata dump format: a single `[` on the first line, a
single `]` on the last, every other line one JSON entity terminated by
a trailing comma (except the last). The scanner tolerates the format
straight from `zcat latest-all.json.gz`.

### Flags

| Flag | Default | Meaning |
|---|---|---|
| `--rules <path>` | required | Path to the rules JSON file. |
| `--checks <ids>` | all | Comma-separated check IDs to enable. |
| `--format <fmt>` | `jsonl` | Output format. **Currently only `jsonl` is implemented.** |
| `--output <path>` | stdout | Where to write issue records. |
| `--threads <N>` | available cores | Parser/check parallelism. |
| `--progress` | off | Emit a progress line to stderr every N entities. |
| `--progress-interval <N>` | 1,000,000 | Entities per progress line. |
| `--verbose` / `-v` | off | Log non-fatal events (e.g. parse errors). |

### Examples

Scan the full dump with all checks enabled:

```sh
zcat latest-all.json.gz \
  | wikidata-lint --rules rules/en.json --progress \
  > issues.jsonl
```

Scan only specific checks:

```sh
zcat latest-all.json.gz \
  | wikidata-lint --rules rules/en.json \
      --checks description.misspelled,description.composite \
  > issues.jsonl
```

Write to a file rather than stdout:

```sh
zcat latest-all.json.gz \
  | wikidata-lint --rules rules/en.json --output issues.jsonl --progress
```

### Output

JSONL — one record per detected issue:

```json
{
  "qid": "Q12345",
  "lang": "en-gb",
  "field": "description",
  "check": "description.misspelled",
  "value": "the abandonned ship",
  "suggestion": "the abandoned ship",
  "details": null
}
```

`field` is one of `"label"`, `"description"`, `"alias"`. `suggestion`
is `null` for checks without a canonical fix. `details` is an array of
sub-check IDs for `description.composite`, or `{"new_max_len": <int>}`
for `aliases.long`/`descriptions.long`, otherwise `null`.

### Available checks

Note: trailing `!` is detected but **not auto-stripped**. Too many
band/stage names end with `!` (`!!!`, `Against Me!`, `Empire! Empire!`,
`¡Mayday!`, `Haloo Helsinki!`, ...) for the fix to be safe to
automate. Records ending with `!` flag as `description.ends_with_punctuation`
and route to the unfixable report with reason `nonperiod_punct` for
human review. Only `.` and `?` are auto-stripped.

**Fixable** (the fixer can mechanically correct these):

- `description.misspelled`
- `description.starts_with_lowercase_nationality`
- `description.contains_lowercase_nationality`
- `description.contains_html_entity`
- `description.contains_double_space`
- `description.space_before_comma`
- `description.contains_trademark`
- `description.ends_with_punctuation`
- `description.starts_with_label`
- `description.bad_start` (only when the prefix is in `bad_start_strip_prefixes`; others reject as `unstrippable_bad_start`)
- `description.composite`

**Detection-only** (reported, not auto-fixed):

- `description.too_long`
- `description.starts_capitalized`
- `description.marketing_imperative`
- `description.promotional`
- `description.multi_sentence`
- `description.contains_obituary`
- `aliases.long`
- `descriptions.long`

## `wikidata-fix` — the fixer

Reads scanner JSONL on stdin and emits **one CSV per `(field, lang)`
column** into a directory you specify via `--output-dir`. Each file has
the shape `qid,<column>` with rows only for items that have a fix for
that specific column. No empty cells anywhere.

This shape sidesteps QuickStatements' CSV importer, which interprets
empty `Lxx` / `Dxx` / `Axx` cells as "set this field to empty"
— destructively blanking unrelated fields if you fed it a single
sparse CSV.

### Flags

| Flag | Default | Meaning |
|---|---|---|
| `--rules <path>` | required | Same rules file the scanner used. |
| `--output-dir <path>` | required | Directory to write per-column CSVs into. Created if missing. |
| `--enable <ids>` | all fixable | Comma-separated check IDs to enable. |
| `--disable <ids>` | none | Comma-separated check IDs to disable from the enabled set. |
| `--unfixable <path>` | discard with stderr count | Path for the unfixable-report JSONL. |

### Property records are skipped

Records whose `qid` starts with `P` (Wikidata properties) are dropped
silently before any fix is attempted — they don't appear in the CSV
*or* in the unfixable report. Property descriptions follow conventions
distinct from item descriptions ("with X as a string", etc.) and
aren't worth auto-fixing. The fixer prints a one-line count of
skipped property records to stderr so you can see how many were
filtered. Lexemes (`L`-prefix) and other entity types still flow
through normal processing.

### Coalescing

The fixer groups input records by `(qid, lang, field)` and applies each
applicable fix to a shared working string in input order. Each
surviving group becomes one row in the CSV file named after its column
(`Den.csv` for `(description, en)`, `Den-gb.csv` for `(description,
en-gb)`, `Lfr.csv` for `(label, fr)`, etc.).

**A group is all-or-nothing.** If any record in a group is
detection-only or rejected by the safety pass, the *entire* group is
routed to the unfixable report rather than emitted partially. The
practical consequence: feed the fixer a JSONL stream filtered to the
fixable checks. See the workflow below.

### Safety pass

Each post-fix value is rejected (routed to the unfixable report) if:

- It is empty.
- It is longer than `thresholds.description_max_len` (descriptions) or
  250 characters (labels and aliases).
- It contains a control character (including TAB, LF, CR).

If the post-fix value equals the original `value`, the cell is
silently suppressed (no-op edits are not emitted).

### Post-fix guideline check

A description fix may correct one issue (a misspelling, an HTML
entity) while leaving the description still violating other guideline
rules — for example, a description that's both misspelled and starts
with `"is a "`. After the safety pass, the result is checked against
`bad_starts_descriptions`; if the post-fix value still starts with any
listed prefix, the group is rejected with reason `post_fix_bad_start`.
This prevents the fixer from emitting half-fixed values that would
need a manual second pass.

This is description-only. Labels and aliases aren't validated this way
(no current fix targets them, and the convention is different).

### Unfixable report

JSONL with each input record echoed verbatim plus a `"reason"` field.
Reasons: `parse_error`, `detection_only`, `disabled`, `safety_bounds`,
`control_chars`, `partial_html`, `nonperiod_punct`, `would_blank`,
`composite_partial`, `post_fix_bad_start`.

## Typical pipeline against a full dump

The fixer's all-or-nothing per-group routing means you should filter
the scanner output to fixable checks before piping. Either run the
scanner with `--checks` set to the fixable list, or filter the JSONL
between the two binaries.

**Filter at the scanner** (simplest — half the work, smaller artifact):

```sh
FIXABLE='description.misspelled,description.starts_with_lowercase_nationality,description.contains_lowercase_nationality,description.contains_html_entity,description.contains_double_space,description.space_before_comma,description.contains_trademark,description.ends_with_punctuation,description.starts_with_label,description.bad_start,description.composite'

zcat latest-all.json.gz \
  | ./target/release/wikidata-lint --rules rules/en.json --checks "$FIXABLE" --progress \
  > fixable.jsonl

./target/release/wikidata-fix --rules rules/en.json \
    --output-dir batches/ \
    --unfixable skipped.jsonl \
  < fixable.jsonl
```

`batches/` will contain one file per `(field, lang)` combination —
`batches/Den.csv`, `batches/Den-gb.csv`, etc. **Paste each file into
QuickStatements separately, using its CSV import.** Each file is dense
(no empty cells), so nothing outside what you intended to fix gets
touched.

**Or scan everything first, filter later** (keeps the full report for
review; useful when you also want detection-only findings):

```sh
zcat latest-all.json.gz \
  | ./target/release/wikidata-lint --rules rules/en.json --progress \
  > issues.jsonl

# Sample / inspect
wc -l issues.jsonl
jq -r '.check' < issues.jsonl | sort | uniq -c | sort -rn

# Filter to fixable and run the fixer
jq -c 'select(.check as $c | [
    "description.misspelled",
    "description.starts_with_lowercase_nationality",
    "description.contains_lowercase_nationality",
    "description.contains_html_entity",
    "description.contains_double_space",
    "description.space_before_comma",
    "description.contains_trademark",
    "description.ends_with_punctuation",
    "description.starts_with_label",
    "description.bad_start",
    "description.composite"
  ] | index($c))' issues.jsonl \
  | ./target/release/wikidata-fix --rules rules/en.json \
      --output-dir batches/ \
      --unfixable skipped.jsonl
```

Human review is expected between the JSONL and the QuickStatements
batches — `grep`, `jq`, sort, hand-edit, drop rows you don't want.

## Rules file

A single JSON document; see [`rules/en.json`](rules/en.json)
for the canonical shape and [`SPEC.md` §"Rules file schema"](SPEC.md)
for field-by-field semantics. The same file is used by both binaries.

Notable knobs:

- `thresholds.description_max_len` — `description.too_long` trigger and
  the fixer's safety upper bound for descriptions.
- `thresholds.descgust_score_threshold` — minimum composite score to
  emit `description.composite`.
- `skip_qids.<check>` — per-check QID/PID skiplist consulted only by
  the named check (currently `promotional`, `long_aliases`,
  `long_descriptions`, `multi_sentence`).
- `excluded_p31_for_long_aliases` — entities whose first P31 claim
  matches an entry here are excluded from `aliases.long`.
- All string lists are matched **case-sensitively** by default.
  Exceptions: `misspellings` (literal / lowercased / capfirst forms
  tried in order) and `promotional_exempt_substrings`
  (case-insensitive).
- `nationalities_lower` and `proper_adjectives_lower` are merged into
  a single runtime set by both
  `description.starts_with_lowercase_nationality` and
  `description.contains_lowercase_nationality`. They're separate
  config fields purely for organizational clarity:
  `nationalities_lower` for countries, demonyms, and nationality-
  prefix forms (`anglo`, `indo`, `sino`, ...); `proper_adjectives_lower`
  for the broader "should be capitalized in English" category —
  continents (`european`, `asian`, ...), religions (`christian`,
  `muslim`, ...), or anything else you'd capitalize. Either list
  alone is sufficient — the split is an organizing convenience.
  The check IDs both still contain "nationality" for backwards
  compatibility; their actual meaning is "lowercase token that
  should be capitalized."
- `bad_start_strip_prefixes` — subset of `bad_starts_descriptions` that
  the fixer is allowed to strip from the start of a description.
  Default in `rules/en.json` is the safe copular forms `"is an "`,
  `"was an "`, `"is a "`, `"was a "`, `"are "`, `"were "`. Articles
  (`"A "`, `"An "`, `"The "`) are deliberately omitted because they're
  load-bearing for proper nouns ("The Beatles"). When the description
  starts with a `bad_starts_descriptions` prefix that *isn't* in this
  list, the post-fix guideline check rejects with
  `post_fix_bad_start`. Stripping does *not* lowercase the first
  character of the result, so proper-adjective starts like
  "Guinean-born" survive intact. The scanner dispatches `bad_start`
  *last* among description checks so it runs after the
  suggestion-based fixes (`misspelled`,
  `starts_with_lowercase_nationality`) that operate on the original
  value — otherwise a misspelling fix would re-introduce the bad
  start that `bad_start` had just stripped.
- `ends_with_punctuation_exempt_suffixes` — literal end-of-description
  suffixes that exempt a value from `description.ends_with_punctuation`
  (e.g. `"Inc."`, `"Ltd."`, `"Jr."`). Case-sensitive end-of-string
  match. Defaults to empty if omitted. Independent of this list, three
  structural exemptions are always on:
  - **Balanced parens.** A description ending with `)` whose `(`/`)`
    are balanced overall — common Wikidata disambiguation pattern, e.g.
    `"ABC (band)"`.
  - **Dotted-acronym / single-letter-initial.** A description whose
    trailing whitespace-bounded token matches `(<letter>.)+` with at
    least one letter+period pair. Covers classic dotted initialisms
    (`"R.O.C."`, `"U.S.A."`, `"e.g."`, `"a.k.a."`) *and* the
    Firstname-L. pattern (`"Boney M."`, `"Jon B."`). Single-trailing-
    period words like `"USA."` are *not* exempt by this rule (the
    token has no letter-period pair structure — they look more like
    sentence ends).
  - **Trailing ellipsis.** A description whose final three or more
    characters are consecutive ASCII periods, e.g. `"foo bar baz..."`
    (truncation marker) or `"In the Woods..."` (band name). Two
    trailing periods aren't exempt — usually a typo. The Unicode
    ellipsis `"…"` is also exempt because it isn't ASCII punctuation.

## Exit codes

- `0` — completed (regardless of whether issues were found / records were unfixable).
- `2` — bad CLI args or rules file.
- `3` — fatal I/O error mid-stream.

## Performance

On commodity hardware the scanner runs at roughly **70–80k entities /
sec multithreaded** and **15–20k entities / sec / core
single-threaded**, parse-bound. A full `latest-all.json.gz` (≈100M
entities, ≈1.5 TB uncompressed) is in the order of a few hours
single-machine.

If you're pulling the dump from disk over a single pipe, `zcat` itself
becomes the wall-clock bottleneck before the scanner does. Consider:

```sh
pigz -dc latest-all.json.gz \
  | ./target/release/wikidata-lint --rules rules/en.json --progress \
  > issues.jsonl
```

The two streaming high-water-mark checks (`aliases.long`,
`descriptions.long`) run on the writer thread, so their running maxima
are deterministic regardless of `--threads`.
