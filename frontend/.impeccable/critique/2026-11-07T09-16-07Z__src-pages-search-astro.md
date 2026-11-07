---
score: 20
p0: 0
p1: 2
method: dual-agent
target: src/pages/search.astro
timestamp: 2026-11-07T09-16-07Z
slug: src-pages-search-astro
---
# Critique: /search (Live Contracts)

Method: dual-agent (A: grok-4.7 · B: Claude parent). Questions to the user were skipped because the run is autonomous; the open questions are listed at the end instead.

## Verdict

The FTS rollout made search work and fast; it did not make search *good*. The visual system is authored and consistent with Browse, but the page treats a query result set like a landing page, and the data quality of titles and snippets is the weakest link. Three defects found here were fixed and deployed during the run (#84, #85).

## Nielsen heuristics (synthesized; A scored 19/40, B scored 23/40)

| # | Heuristic | Score | Key issue |
|---|-----------|-------|-----------|
| 1 | Visibility of system status | 2 | Count shows, but no "1–12 of N", no sort label (newest filed). |
| 2 | Match system / real world | 2 | "full markdown text" hint; ACC/CIK unexpanded. Comment banners in snippets fixed in #85. |
| 3 | User control and freedom | 2 | No first/last/jump; past-the-end pages now redirect (#84). |
| 4 | Consistency and standards | 3 | Same DocketRow/pagination as Browse; Browse filters missing here. |
| 5 | Error prevention | 2 | AND/OR/NOT silently searched as words; quote syntax undisclosed. |
| 6 | Recognition rather than recall | 2 | No example queries, no form/date/company chips. |
| 7 | Flexibility and efficiency | 1 | Type, Search, Next. Nothing else. |
| 8 | Aesthetic and minimalist design | 2 | Full hero with entrance animation repeats on every results page. |
| 9 | Error recovery | 2 | Zero hits: "Try a different term", no example, no link to Browse. |
| 10 | Help and documentation | 2 | One hint line. |
| **Total** | | **20/40** | **Acceptable-minus** |

## Anti-patterns

- Detector (CLI) on search.astro and DocketRow.astro: 0 findings.
- Browser overlay (B): long snippet line length (~79ch at wide viewports), all-caps meta and results bar, h1→h3 skip, `repeating-linear-gradient` hatch on the empty states.
- A adds: `.rise` entrance replays on every page; frosted header; stacked-paper pseudo-sheets behind every snippet. These are pre-existing site-wide choices, not introduced by FTS.

## Priority issues

- **P1 Rail titles are unreliable.** `displayTitle` yields clause numbers ("1. Terms of Employment"), signatory names, or bare `EX-10.2`. The backslash case is fixed (#84); the rest needs a fallback of `EX-10.n · company` and full-width titles.
- **P1 No way to narrow 18k hits.** Add form/date/company filters (Browse already has them), a range label, and a page jump.
- **P2 Hero in the way once a query exists.** Collapse to a compact field plus count; stop replaying `.rise`; sentence-case the query in the results bar.
- **P2 Empty and zero states don't teach.** Show 2–3 live example queries and say that quotes match phrases.
- **P3 A11y details.** `aria-current="page"` on the pager, a results heading, a visible focus ring on the search input, disabled pager contrast (~2.3:1).

## Personas

- **Alex (power user):** cannot sort, filter to 8-K, or jump to page 204; `autofocus` steals the caret after each Next; boolean operators silently ignored.
- **Jordan (first-timer):** "markdown" means nothing; three identical "Securities Purchase Agreement" titles; CIK/ACC unexplained.

## Fixed during this run

- #84: past-the-end page now 302s to the last page; markdown hard breaks/escapes no longer leak into titles. Deployed `ef340c32`.
- #85: HTML comments and zero-width characters dropped from snippets (3 of 12 cards were affected). Deployed `0b64a77a`.

## Open questions

1. When the rail title and the snippet disagree, which should lead visually?
2. Should page 2+ drop the hero entirely?
3. Should a phrase hit inside a representation be labeled "mention" versus a document that *is* that instrument?
