# Wikidata Lint — Specification

A two-binary Rust system that scans a Wikidata JSON dump for issues in
**labels**, **descriptions**, and **aliases**, then optionally turns
fixable findings into a QuickStatements batch. All textual rules (word
lists, substring lists, misspelling maps, QID/PID skiplists, etc.) live
in an external JSON **rules file** so they can be tuned without
recompiling.

## System overview

```
                rules.json
                    │
                    ▼
  dump ──▶  wikidata-lint  ──▶  issues.jsonl  ──▶  wikidata-fix  ──▶  batch.qs
                                     │                  ▲
                                     │             rules.json
                                     ▼
                              (human review,
                               grep, sort, edit)
```

Two tools, one shared rules file:

1. **`wikidata-lint`** (the scanner): streams a Wikidata JSON dump and
   emits one JSONL record per detected issue. Pure detection — does not
   propose anything beyond the per-check `suggestion` field.
2. **`wikidata-fix`** (the fixer): reads the JSONL, applies canonical
   fixes for the subset of checks where a mechanical correction is
   well-defined, and emits QuickStatements v1 syntax. Detection-only
   checks (e.g. promotional language, over-long descriptions) are
   passed through to a separate report file rather than turned into
   edits.

The split exists because (a) the scanner is bulk-streaming and stable;
(b) fix logic iterates faster and benefits from a JSONL review
checkpoint where humans can grep, hand-edit, or drop rows before edits
go to Wikidata; (c) only a subset of checks have canonical fixes.

The remainder of this document specifies each tool. Sections marked
**[scanner]** describe `wikidata-lint`; sections marked **[fixer]**
describe `wikidata-fix`.

## [scanner] Overview

A single-binary Rust program that scans a Wikidata JSON dump for issues
in **labels**, **descriptions**, and **aliases**, replacing a collection
of ad-hoc Python scripts.

## Inputs

1. **Dump stream** — the Wikidata JSON dump on stdin, in its native format:
   the file opens with a single `[` line, ends with a single `]` line, and
   every other line is one entity's JSON object terminated by a trailing
   comma (except the last). The program must:
   - Skip the opening `[` and closing `]` lines.
   - Strip a single trailing `,` from each entity line before parsing.
   - Tolerate files that come straight from `zcat latest-all.json.gz`.

2. **Rules file** — path supplied via `--rules <path>` (required). A single
   JSON document; schema below.

3. **Optional flags**
   - `--checks <list>` — comma-separated check IDs to enable; default is
     all. (See *Checks* below.)
   - `--format <jsonl|csv|tsv>` — output format; default `jsonl`.
   - `--output <path>` — defaults to stdout.
   - `--progress` — emit a progress line to stderr every N entities
     (configurable via `--progress-interval`, default 1,000,000).
   - `--threads <N>` — parsing parallelism; default = available cores. The
     dump reader stays single-threaded; per-line parsing and rule
     evaluation are parallelized over a worker pool.

## Language handling

Every check that reads a textual field must apply to **every English
variant present on the entity**, not just `en`. Concretely, for each
field type (`labels`, `descriptions`, `aliases`), iterate over keys
matching:

- exactly `en`, **or**
- starting with `en-` (case-insensitive), e.g. `en-us`, `en-gb`,
  `en-ca`, `en-simple`, `en-x-...`.

Each emitted issue record must include the language code that triggered
it. Per-language results are independent: an item may produce a
description hit in `en` and a separate one in `en-gb`.

For checks that compare a description against the entity's label
(`label_prefix` — see below), the comparison must be done *within the
same language code*. If `descriptions["en-us"]` exists but
`labels["en-us"]` does not, fall back to `labels["en"]` if present;
otherwise skip the prefix comparison for that record.

## Rules file schema

```json
{
  "nationalities_lower": ["palestine", "palestinian", "..."],
  "misspellings": { "abandonned": "abandoned", "...": "..." },
  "bad_starts_descriptions": ["a ", "an ", "the ", "It ", "is ", "http://"],
  "marketing_imperatives": ["Discover ", "Enjoy ", "Indulge ", "Buy "],
  "promotional_substrings": ["the best ", " finest ", "Christ-centered"],
  "promotional_exempt_substrings": ["award"],
  "trademark_chars": ["®", "™"],
  "html_entity_substrings": ["&amp;", "&#91;", "&#93;"],
  "multi_sentence_markers": [". The", ". A ", ". An "],
  "obituary_markers": ["Obituary"],

  "skip_qids": {
    "promotional": ["Q749290"],
    "long_aliases": ["Q633110", "Q892935"],
    "long_descriptions": ["Q31", "Q8", "P131"],
    "multi_sentence": ["Q749290"]
  },

  "excluded_p31_for_long_aliases": [
    "Q13442814", "Q4167410"
  ],

  "thresholds": {
    "description_max_len": 140,
    "descgust_score_threshold": 4
  }
}
```

Notes:
- All string lists are matched **case-sensitively** by default, except
  `misspellings` (see *Misspellings* below).
- `nationalities_lower` is the union of country names and nationality
  adjectives; entries may be all-lowercase or mixed-case, mirroring the
  existing word list. Matching is described under *Capitalization*.
- `skip_qids.<check>` is consulted only by the named check. QIDs and
  PIDs both allowed (the existing scripts mix them).

## Checks

Each check has a stable string ID. The output record's `check` field
takes one of these values.

### `description.too_long`
Emit when `len(value) > thresholds.description_max_len`. UTF-8
character count, not byte count.

### `description.starts_with_label`
Emit when the description for a language starts with that language's
label (or `en` label as fallback per *Language handling*).

### `description.starts_capitalized`
Emit when the first character has the Unicode `Uppercase` property.

### `description.ends_with_punctuation`
Emit when the last character is in the ASCII punctuation class
(`!"#$%&'()*+,-./:;<=>?@[\]^_\`{|}~`).

### `description.contains_trademark`
Emit when any string in `trademark_chars` is a substring.

### `description.contains_html_entity`
Emit when any string in `html_entity_substrings` is a substring.

### `description.contains_double_space`
Emit when `"  "` is a substring.

### `description.contains_obituary`
Emit when any string in `obituary_markers` is a substring.

### `description.space_before_comma`
Emit when `" ,"` is a substring.

### `description.bad_start`
Emit when the description starts with any string in
`bad_starts_descriptions` (prefix match, case-sensitive).

### `description.marketing_imperative`
Emit when the description contains any string in
`marketing_imperatives` as a substring.

### `description.promotional`
Emit when the description contains any string in
`promotional_substrings` AND does not contain any string in
`promotional_exempt_substrings` (case-insensitive substring test for
the exemption, matching the original behavior of "skip if `award` is
anywhere in the lowercased description"). Skip QIDs in
`skip_qids.promotional`.

### `description.composite`
Composite score replicating `descgusting.py`. Counts +1 for each of:
- `description.starts_with_label`
- `description.too_long`
- `description.starts_capitalized`
- `description.ends_with_punctuation`
- `description.contains_trademark` (each of `®` and `™` counts
  separately, as in the original)
- `description.bad_start`
- `description.contains_double_space`
- `description.contains_obituary`
- `description.contains_html_entity` (specifically `&amp;`)
- `description.space_before_comma`

Emit when the sum is `>= thresholds.descgust_score_threshold`. The
record's `details` field lists which sub-checks fired.

### `description.multi_sentence`
Emit when any string in `multi_sentence_markers` is a substring. Skip
QIDs in `skip_qids.multi_sentence`. Skip property entities (IDs
starting with `P`).

### `description.misspelled`
Tokenize the description on Unicode whitespace. For each token, look up
the literal token, then `token.to_lowercase()`, then a "capfirst"
form (uppercase first char, rest unchanged) in `misspellings`. Emit
when any token matches; the suggested replacement is the description
with each matched token swapped for its correction (preserving the
matched form: lowercase match → use lowercase value; capfirst match →
use capfirst value).

### `description.starts_with_lowercase_nationality`
Tokenize on whitespace. Emit when the first token, exactly as written,
is in `nationalities_lower`. Suggested fix: capitalize first character
of the description, leave rest unchanged.

### `description.contains_lowercase_nationality`
Tokenize on whitespace. For each token after the first, emit if the
token is in `nationalities_lower`. Additionally, if the token contains
a single `-`, split on it and emit if either half is in the set. (This
preserves the hyphen-handling in the original.)

### `aliases.long`
Streaming "high-water mark" check, English variants only. For each
entity, if it has a `claims.P31` whose first claim has a
`mainsnak.datavalue.value.id` not in `excluded_p31_for_long_aliases`,
and the entity is not in `skip_qids.long_aliases`, then for each alias
in each `en*` language: if its character length exceeds the running
maximum, emit a record and update the running maximum. Maximum is
shared across all `en*` languages (one global counter), matching the
spirit of the original.

### `descriptions.long`
Streaming "high-water mark" check on description length. Skip QIDs/PIDs
in `skip_qids.long_descriptions`. One global counter across all `en*`
variants.

## Output

JSONL is the default and the canonical format. One record per detected
issue, one record per line:

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

Field rules:
- `field` is one of `"label"`, `"description"`, `"alias"`.
- `suggestion` is `null` when the check has no canonical fix.
- `details` is `null` for most checks. For `description.composite` it is
  an array of sub-check IDs that fired. For
  `aliases.long`/`descriptions.long` it is `{"new_max_len": <int>}`.
- For `aliases.long` the `value` is the alias string; for the
  description checks it is the description string.

CSV/TSV format flattens the same fields, with `details` JSON-encoded.
Header row required.

## Performance & robustness

- Use a streaming JSON parser per line (`serde_json::from_str` on each
  line is sufficient given the dump's line-per-entity layout).
- Skip silently if a line fails to parse (log to stderr at `--verbose`).
- Do not allocate a fresh regex/automaton per entity. Compile all
  substring/prefix matchers once at startup; consider an Aho-Corasick
  automaton for the substring-list checks. Misspellings and the
  nationality set should live in `HashSet`/`HashMap`.
- Worker threads receive raw lines and emit `Vec<Issue>`; a single
  writer thread serializes output to keep ordering deterministic per
  input line. The two streaming high-water-mark checks
  (`aliases.long`, `descriptions.long`) run on the writer thread to
  preserve ordering and the running maximum's monotonicity.

## Exit codes

- `0` — completed, regardless of whether issues were found.
- `2` — bad CLI args or rules file.
- `3` — fatal I/O error mid-stream.

## [scanner] Example invocation

```sh
zcat latest-all.json.gz \
  | wikidata-lint --rules rules.json --checks description.misspelled,description.composite \
  > issues.jsonl
```

## [fixer] Overview

`wikidata-fix` consumes the scanner's JSONL on stdin and emits
QuickStatements v1 batch lines on stdout. Records whose check has no
canonical fix are routed to a separate "unfixable" report
(`--unfixable <path>`, default: discarded with a stderr count) rather
than silently dropped.

## [fixer] Inputs

- **stdin** — JSONL produced by `wikidata-lint`. Records that fail to
  parse are written to the unfixable report and counted; they do not
  abort the run.
- `--rules <path>` — same rules file as the scanner. Some fixes
  (notably misspelling replacement and label-prefix stripping) need
  the same word lists the scanner used.
- `--enable <list>` / `--disable <list>` — comma-separated check IDs
  controlling which fixes are applied. Default: all checks for which a
  canonical fix is defined below.
- `--unfixable <path>` — where to write detection-only records and
  records skipped by `--disable`.
- `--dry-run` — for each input record, write a one-line explanation to
  stderr (`<qid> <lang> <check>: <before> -> <after>`) and suppress
  QuickStatements output.

## [fixer] QuickStatements syntax used

This tool emits **QuickStatements v2 / CSV** (the same format the
existing Python scripts produce). One CSV file with a header row, one
data row per item.

Column conventions:

| Column     | Meaning                                 |
|------------|-----------------------------------------|
| `qid`      | Item identifier (first column).         |
| `L<lang>`  | Set the label in `<lang>`.              |
| `D<lang>`  | Set the description in `<lang>`.        |
| `A<lang>`  | Add an alias in `<lang>`.               |

`<lang>` uses the language code from the input record verbatim (`en`,
`en-gb`, `en-us`, …), so an `en-gb` description fix lands in column
`Den-gb`. The header is the union of every column used by any row in
the batch, sorted alphabetically after `qid` for determinism. A cell
is empty when that column does not apply to that item.

Values are written per RFC 4180: a value containing a comma, a double
quote, or a newline is wrapped in `"..."` and any embedded `"` is
doubled to `""`. Tab, newline, and carriage-return characters in input
values are rejected by the safety pass (see *Safety* below) rather
than escaped, so they should never reach output.

Alias removal is not expressible in CSV form, but the only
alias-related check (`aliases.long`) is detection-only, so this is not
a limitation in practice.

## [fixer] Per-check fix definitions

For every check listed below, the fixer computes a corrected value
that, after coalescing (see below), populates one cell of the CSV.
Records whose check is not listed here are detection-only and routed
to the unfixable report.

### `description.misspelled` → `D<lang>`
Use the scanner-provided `suggestion` directly.

### `description.starts_with_lowercase_nationality` → `D<lang>`
Capitalize the first character of `value`, leave the rest unchanged.

### `description.contains_lowercase_nationality` → `D<lang>`
For each whitespace-separated token in `value` (after the first) that
is in `nationalities_lower`, capitalize its first character; for
hyphenated tokens, capitalize the half(s) that match.

### `description.contains_html_entity` → `D<lang>`
Decode the specific entities listed in `html_entity_substrings`
(typically `&amp;` → `&`, `&#91;` → `[`, `&#93;` → `]`). Other entities
are left in place and the record is passed to the unfixable report
with a note.

### `description.contains_double_space` → `D<lang>`
Collapse runs of two or more ASCII spaces to one. Other whitespace
(tabs, NBSP) is left alone.

### `description.space_before_comma` → `D<lang>`
Replace each `" ,"` with `","`.

### `description.contains_trademark` → `D<lang>`
Strip every occurrence of each character in `trademark_chars`. Trim
the result.

### `description.ends_with_punctuation` → `D<lang>`
Strip a single trailing ASCII-punctuation character. Only applied when
the trailing punctuation is `.`, `!`, or `?` — other punctuation
(parens, brackets, quotes) is left untouched and the record routed to
the unfixable report.

### `description.starts_with_label` → `D<lang>`
Remove the leading copy of the label, then any leading `is a`, `is an`,
`was a`, `was an`, `are`, or `were` (one match, case-insensitive,
followed by whitespace), then trim. If the result is empty, route to
the unfixable report instead (we will not blank a description on the
basis of this check alone). Lowercase the first character of the
result.

### `description.composite` → coalesced fix into `D<lang>`
If **all** sub-checks listed in `details` have entries above, apply
each in the order they appear in `details` against an in-memory
working string, then write the final result to the `D<lang>` cell. If
any sub-check is detection-only, route the whole record to the
unfixable report — composite cleanup is all-or-nothing.

### Detection-only (always unfixable)
- `description.too_long`
- `description.bad_start` (too varied to mechanize safely)
- `description.marketing_imperative`
- `description.promotional`
- `description.multi_sentence`
- `description.contains_obituary`
- `aliases.long`
- `descriptions.long`

These are reported only.

## [fixer] Coalescing

The fixer collapses input records into one CSV row per `qid` in two
stages.

**Stage 1 — per-cell coalescing.** Group records by
`(qid, lang, field)`. Apply each applicable fix to a shared working
string in input order; the final value is the cell value. If any fix
in the group is detection-only or rejected by safety checks, the
entire group is routed to the unfixable report (better to skip than
to emit a half-cleaned description).

**Stage 2 — per-row assembly.** Group cells by `qid`. Each row has the
qid in column `qid` and the per-cell coalesced values in their
respective `L<lang>` / `D<lang>` columns. Cells with no fix for that
qid are left empty.

Aliases would be keyed by `(qid, lang, "alias", value)` rather than
`(qid, lang, "alias")` since each alias is a distinct string, but no
in-scope fix targets aliases.

## [fixer] Safety

- **Sanity bounds.** Refuse to emit any description longer than
  `thresholds.description_max_len`, or any label/alias longer than
  250 characters. Refuse to emit empty values (would clear the field).
- **Control characters.** If the resulting value contains a control
  character (including TAB, LF, CR), route to the unfixable report;
  do not attempt to escape.
- **No-op suppression.** If the post-fix value equals the original
  `value`, suppress output (don't bother with a no-op edit).
- **Determinism.** Row order follows the order in which each `qid`
  *first* appears in the input. Column order is `qid` first, then the
  remaining columns sorted alphabetically.

## [fixer] Output

QuickStatements v2 / CSV on stdout, with a header row. When
`--annotate` is passed, an extra trailing column `notes` carries a
semicolon-separated list of the check IDs that contributed to that
row, intended for human review (QuickStatements ignores unknown
columns; if your import path does not, drop `--annotate`).

The unfixable report is JSONL with each input record passed through
verbatim plus a `"reason"` field explaining why it was skipped
(`"detection_only"`, `"safety_bounds"`, `"composite_partial"`,
`"control_chars"`, etc.).

## [fixer] Exit codes

- `0` — completed.
- `2` — bad CLI args or rules file.
- `3` — fatal I/O error mid-stream.

## [fixer] Example invocation

```sh
wikidata-fix --rules rules.json --unfixable skipped.jsonl \
  < issues.jsonl > batch.csv
```
