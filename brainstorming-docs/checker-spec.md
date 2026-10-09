# Checker spec — independent geometry checker for edge routing

This document specifies the routing checker: a standalone workspace crate
(`tools/checker/`) that scores a set of edge polylines against the routing
goals. It is written for the agent-based routing improvement (see
`plan-for-agent-based-routing-improvement.md`) and intentionally stands
**outside** the production pipeline: agents optimize against it, the
orchestrator owns it, and its code is never trusted to overlap with the code
it judges.

## 1. Purpose and role in the plan

The checker is the single yardstick for the "agent-based routing
improvement" work: agents propose polylines; the checker scores them;
the orchestrator compares agents on the same corpus and accepts or rejects.

Three hard rules of engagement:

- **The checker is orchestrator-owned.** Only the orchestrator edits
  `experiments/checker/`. Agents treat it as read-only. This is the main
  anti-gaming defense.
- **The checker is geodesically independent.** It may `use diagrammer`
  to *read* types (`Layout`, `NodeRect`, `GroupRect`, `EdgePath`) and — in
  the `--emit-baseline` path only — to run the production pipeline end to
  end (`parse` → `resolve` → `layout`), the same way a user would. It must
  not `use` or copy production checking logic (`scan_violations`,
  `seg_clear_padded`, `Obstacles`, `edge_world`, lane assignment). If a
  helper's only source is production code, the checker re-derives it from
  first principles. Dependencies: the `diagrammer` crate + `std` only —
  no new crates, no dev-dependency font machinery (it never measures
  text).
- **The checker must pass its own self-tests.** The builder (orchestrator or
  first agent under review) adds hand-constructed fixture cases (Section 6)
  and, for every fixture, asserts both the expected violations and the
  expected tolerance/geometry values; a checker build whose fixtures fail is
  not allowed to score anything.

### 1.1 Baseline

Baseline = the current engine's output as it exists the day the checker is
built, **captured before the first agent touches a prototype**, and kept
frozen for the whole plan. Concretely:

- Run the current engine (`layout::layout(&diagram)`) over the full corpus
  and store the resulting cases via the `--emit-baseline` subcommand
  (Section 2) as `experiments/checker/baseline/<case>.json` — serialized
  node rects, group rects, and edge polylines with `label_at`.
- The checker takes `Layout` structs as input (Section 2); baseline case
  data is loaded from these files. If a case's source file is available
  (examples under `examples/*.dgmr`), the case entry also stores the path so
  the checker's `--svg` mode can render a reference SVG.
- Rationale: routes are scored against the fixed node/group geometry they
  share, so a candidate route set for an agent is the baseline case's
  nodes/groups kept verbatim with the edge polylines replaced.

Note: the baseline is frozen, but nothing stops the orchestrator from
re-capturing it later (e.g. after an accepted improvement lands) — that is
an orchestrator action, recorded in the checker README, and it starts a new
comparison epoch (Section 8).

### 1.2 Consistency properties (verified in self-tests)

Three properties the checker's tests enforce, because an inconsistent score
is worse than no score:

- **Translation invariance.** Shifting every point of a case's geometry by
  the same (dx, dy) must not change the score or any violation set (after
  the same shift is applied to nodes and groups). Fixture: score
  `baseline/infra`, score the same layout shifted by (137.5, -204.25),
  require identical output.
- **Super-flip guard.** If any two edges in a scored polyline set *fully
  coincide* on a segment (same endpoints within `f32` epsilon), the checker
  reports a `super_flipped` violation (hard) and the case score is 0. This
  catches a scorer-gaming failure mode where an agent outputs one polyline
  and assigns it to several edges.
- **Permutation invariance (pairwise).** The violation *set* (as a set of
  unordered pairs) must be invariant under reordering the edge list of the
  input. (The score is trivially invariant — it is a count over pairs.)
  Fixture: score `baseline/sides` with the edges in their natural order
  and in 3 fixed, committed permutations; the pairwise violation set must
  be identical. No randomness: the permutations are written out in the
  test.

## 2. Inputs and invocation

The checker is a binary target in the workspace (`tools/checker/`, joining
`tools/gen-metrics-table/` as a workspace member; the root crate's own
dependency list must not change):

```
diagrammer-checker <case-file.json> [--json <out.json>] [--svg <out-dir>]
                   [--svg-halos]
diagrammer-checker --emit-baseline <source.dgmr> <out-case.json>
```

- `<case-file.json>` — one or more `Layout` cases (Section 3).
- `--json <out.json>` — write the full report (Section 4) to a JSON file.
- `--svg <out-dir>` — additionally render a diagnostic SVG per case
  (Section 5).
- `--emit-baseline` — orchestrator-only: parse the source with the
  production pipeline, run the frozen engine, serialize one case JSON.
  This is the only path by which corpus/baseline cases are created.

The checker reads the serialized `Layout` structs directly — it does not
re-run the layout engine. This keeps agent work decoupled from the engine's
determinism guarantees: an agent can score a *proposed* polyline set for
existing node/group geometry by serializing it into a case file, without
building the engine.

The checker returns a non-zero process exit code iff any case contains any
hard violation (Section 4.2), so CI and agent drivers can use exit codes.

## 3. Case file format

A case file is a serialized scoring scenario. Since the checker needs only
polylines + node/group rects, the case file is a plain data file holding
one or more case objects (a JSON array). The structure is shown here in
YAML purely for readability — the on-disk format is JSON (see below):

```yaml
# (illustrative; on-disk format is a JSON array of these objects)
name: infra-baseline          # used in reports and file names
# Optional reference to the source .dgmr, for --svg context. Purely
# informational; the checker never reads it for scoring.
source: examples/infra.dgmr
nodes:                        # axis-aligned node rects
  - id: api
    x: 100.0
    y: 80.0
    w: 60.0
    h: 40.0
    shape: box                # box | cylinder (cylinder adds visual cap, scored as rect)
groups:                       # axis-aligned group frame rects
  - index: 0
    x: 80.0
    y: 60.0
    w: 120.0
    h: 90.0
edges:                        # ordered polylines, index = edge index
  - from: api
    to: db
    # points are in the same absolute page coordinates as nodes/groups
    points: [[100.0, 120.0], [130.0, 140.0], [130.0, 180.0]]
```

On-disk format: **JSON**, a case file is a JSON array of case objects with
exactly the fields above. The workspace has no serde, so the checker ships
a small hand-written reader/writer over this fixed schema (stable, small,
self-tested by round-tripping the corpus). Agents write case files by hand
only for tiny fixtures; baseline and corpus cases are produced by the
`--emit-baseline` subcommand, which reads a case's source `.dgmr`, runs the
frozen engine once, and writes the case JSON. No YAML parser is ever
implemented.

## 4. Output: the report

The report is JSON. It is the single interface between agents and the
orchestrator.

### 4.1 Shape

```json
{
  "schema_version": 1,
  "tolerances": { "parallel_gap": 5.0, "corner_gap": 5.0, "epsilon": 1e-6 },
  "epoch": 1,
  "cases": [
    {
      "name": "infra-baseline",
      "score": { "hard_violations": 0, "total_segments": 47 },
      "per_edge": [
        {
          "edge_index": 0,
          "from": "api", "to": "db",
          "segments": 3,
          "violations": [
            {
              "kind": "pass_through_node",
              "severity": "hard",
              "node_index": 2,
              "node_id": "cache",
              "segment_index": 1,
              "location": [130.0, 140.0],
              "penetration_depth": 6.5
            }
          ]
        }
      ],
      "pairs": [
        {
          "kind": "parallel_gap",
          "severity": "hard",
          "edge_a": 0, "segment_a": 1,
          "edge_b": 3, "segment_b": 2,
          "gap": 2.4,
          "location": [210.0, 95.0]
        }
      ]
    }
  ]
}
```

- `epoch` echoes the baseline epoch the case files came from (Section 8),
  so a stale report is self-identifying.
- `hard_violations` is the count over all hard violations.
- `total_segments` = Σ over edges of (waypoints − 1) — the soft score
  component (4.4); it is not a violation instance and never appears in
  `per_edge`/`pairs` lists.
- `pairs` is the list of *pairwise* violations (edge×edge and edge×group);
  per-edge violations (node pass-through) live under each edge's `violations`.
- Every violation carries a `location` (point coordinate, usually the worst
  point of the violation) and a `segment_index` for quick SVG annotation.
- For the permutation-invariance test (1.2), a violation's identity is the
  unordered pair (kind, segment coordinates rounded to 2 decimals); the
  test compares identity sets, not indices.

### 4.2 Violation kinds and the comparison order

Severity is exactly two levels: hard (goal violations) and soft (the
preference). The plan's Goal list maps as follows:

| kind | severity | definition |
|---|---|---|
| `pass_through_node` | hard | a segment enters a node rect (see 4.3.1) |
| `parallel_gap_edge_edge` | hard | two parallel segments closer than `parallel_gap` (4.3.2) |
| `parallel_gap_edge_group` | hard | a segment parallel to a group outline closer than `parallel_gap` (4.3.2) |
| `corner_gap` | hard | a corner (elbow) of one edge within `corner_gap` of another edge's corner or of a group outline corner (4.3.3) |
| `non_orthogonal` | hard | a segment with dx ≠ 0 and dy ≠ 0 beyond `epsilon` (4.3 preamble) |
| `label_off_path` | hard | an edge's `label_at` anchor is not on its own polyline (4.3.5) |
| `super_flipped` | hard | two edges' polylines fully coincide (1.2) — case score 0 |

There is exactly one soft component (`total_segments`) and it is a case
metric, not a per-instance violation. A case is "clean" iff
`hard_violations == 0`.

**Comparison order for candidate route sets.** Candidate A beats candidate
B iff:

1. A's `hard_violations` count is lower, or
2. hard counts equal, and A's **kind signature** is lexicographically
   lower — the per-kind hard counts, kinds in a fixed order
   (`pass_through_node`, `super_flipped`, `non_orthogonal`,
   `label_off_path`, `parallel_gap_edge_edge`, `parallel_gap_edge_group`,
   `corner_gap`), compared as a tuple. So at equal totals, a candidate
   that traded a `pass_through_node` for one extra `parallel_gap` loses
   here, not at level 1.
3. equal through 2, and A's `total_segments` is lower (the soft goal —
   fewer segments prioritized, per the plan), or
4. equal through 3, tie broken by comparing the lexicographically-sorted
   serialized polylines (a total order, so tests can assert equality
   unambiguously).

This ordering exists so agents' results can be ranked mechanically; humans
reviewing output should still look at SVGs (Section 5).

### 4.3 Geometry definitions (normative)

All coordinates are `f32` in absolute page space, matching `Layout`.
Segments are closed (endpoints included); containment in rectangles is
defined per-test (4.3.1 uses the open interior; the outline tests in
4.3.2/4.3.3 use the drawn frame lines themselves). Orthogonality
assumption: polylines are axis-aligned; a segment is "horizontal" if its
endpoints share a y coordinate (within `epsilon`) and "vertical" otherwise;
a segment whose endpoints differ in both coordinates is reported as a
`non_orthogonal` violation (hard, per the table in 4.2) rather than
interpreted. The angle question is thus reduced to the axis classification:
parallel = same orientation; perpendicular = opposite orientation; any other
relation is a malformed segment.

#### 4.3.1 `pass_through_node`

Segment S (closed) vs node rect R (grown by nothing; the box itself is
visual). Violation iff S ∩ R° ≠ ∅ where R° is the open interior, **with
grazing allowed**: a segment that lies exactly along the rect boundary or
touches a corner does not pass through. Formally: violation iff any point of
S has `R.x < x < R.x+R.w` and `R.y < y < R.y+R.h`. Depth = the depth of
the worst point: for an axis-aligned segment inside an axis-aligned rect
this is `min(distance to each of the four rect edges)` evaluated at the
point of the segment with the largest such value.

Rationale for "open interior": the existing renderer shortens the last
segment by `ARROW_BACKOFF` into the box boundary and ports sit exactly on
boundaries, so a strict-closed test would flag every legal edge terminus.
Agents should still avoid boundary-riding (it looks bad and violates the
gap goals vs. the frame), but it is scored under `parallel_gap_edge_group`
if the boundary belongs to a group outline, and otherwise left unpenalized.

#### 4.3.2 `parallel_gap_*`

Two segments are in violation iff:

- they are parallel (same orientation),
- their projections on the perpendicular axis overlap by more than
  `epsilon` (strictly positive-length overlap; a shared endpoint alone is
  NOT an overlap — corners are handled by 4.3.3),
- and the perpendicular distance between them is < `parallel_gap` (5.0).

Note the direction of the comparison: `< parallel_gap` is a violation;
exactly 5.0 is legal. Coincident parallel segments (distance ≈ 0, including
exactly equal) are a violation with `gap: 0.0`.

Two boundary cases fall out of the overlap rule and are pinned by
fixtures (Section 6, cases 4 and 4b): a parallel segment whose projection merely *touches*
the other's projection at one coordinate (zero-length overlap) is not a
`parallel_gap` violation, even at distance 0 — that contact is a corner
adjacency, judged by 4.3.3 if the contact point is an actual bend. This
keeps T-junction-adjacent and chain-shared geometries from double-counting
as both a gap and a corner violation.

`parallel_gap_edge_group`: the same test with one segment being an edge
segment and the other a group frame edge (the four outline segments of a
`GroupRect`). Same overlap and distance rules.

Corner-touching of an edge segment with a group outline (perpendicular
touch) is legal per the plan goals ("may cross perpendicularly") and is
**not** a `corner_gap` violation — see 4.3.3 for what `corner_gap` covers.

#### 4.3.3 `corner_gap`

A corner is a polyline waypoint that is an actual bend: consecutive segments
with different orientation. Collinear duplicate waypoints (zero-length
segments, or a waypoint lying on the straight line between its neighbors)
are NOT corners — collapse them first (Section 4.5 normalization).

Violation iff the Euclidean distance between (Chebyshev is the
alternative; an implementation must document its choice — the spec
default, used in all fixtures, is Euclidean):

- a corner of edge A and a corner of edge B, or
- a corner of an edge and any corner of a group rect (the 4 rect corners),

is < `corner_gap` (5.0), **except** when the two involved polylines are
actually connected at that point (Section 4.6).

A corner of an edge and a group-rect corner is exempt iff the corner lies
exactly on the group outline (i.e., the edge legally terminates or turns
on the frame crossing point).

#### 4.3.4 `pass_through_group` — NOT a violation

Explicitly: segments may pass through groups (plan §Goal). Group frames
are only scored as outlines (4.3.2/4.3.3). The checker does not treat a
group's interior as an obstacle.

#### 4.3.5 Label knockouts — out of scope, with one guard

The plan's goal list does not include edge-label collision. The checker does
NOT score label placement. One structural guard is kept:
if `label_at` is present for an edge, the checker verifies the anchor lies
on the edge's own polyline (within `epsilon`); if not, it reports
`label_off_path` (hard) — a structural integrity check, not a goal from the
plan, kept because a detached label would break the render contract
(M13). Agents that do not move labels will never see this.

### 4.4 The soft goal: segment count

`tolerances` in the report are constants of the checker, not per-case data:
`parallel_gap = 5.0` and `corner_gap = 5.0` (the 5px figures from the
plan's Goal list), `epsilon = 1e-6` (float hygiene). They are printed in
every report so a reader never has to look them up. Changing a tolerance is
an orchestrator-only act and invalidates cross-epoch comparisons (Section
8) — a report records the tolerances and epoch it was produced under, so
stale comparisons are self-identifying.

The soft goal itself: `total_segments` = Σ (waypoints − 1) after normalization (4.5). There is no
per-segment cost weighting, no bend penalty, no length term: the plan says
"fewer segments prioritized", and that is exactly the count. Length and
bend-shape quality remain out of scope for the checker (they are visible in
SVG review and the existing engine already optimizes them).

### 4.5 Normalization (per edge polyline, before any test)

1. Drop consecutive duplicate points (within `epsilon`).
2. Merge collinear runs: if waypoint i lies on the segment between i−1 and
   i+1 (within `epsilon`), drop it. Repeat until stable.
3. Classify each remaining segment horizontal/vertical/`non_orthogonal`.
4. `segments` count = number of remaining segments.

Normalization is idempotent and order-preserving. Agents may use it to
compare their polyline counts fairly; the checker always scores normalized
forms.

### 4.6 Connected-edge exemption (multi-edge routes)

Some existing engine routes are one logical edge split into several
`EdgePath`s with shared waypoints (e.g. cross-boundary chains through frame
crossing points). Two polylines that share a waypoint are still scored as
separate edges for gap tests — the goals apply between distinct drawn
lines — but the `corner_gap` test is waived for corner pairs that are the
*same* shared waypoint (identical coordinates within `epsilon`), since a
single physical corner is not "two corners touching". No other exemption
exists. If the plan later introduces genuinely-shared route objects, the
checker gains an explicit `same_route_group` field on edges; it is not
present in v1.

## 5. Diagnostic SVG (`--svg`)

For each case, render a diagnostic SVG (not for final human review of route
quality — compare against the production render for that — but for
locating violations):

- node rects: gray fill, black outline, id text;
- group rects: no fill, blue outline, index text at top-left;
- edge polylines: green stroke, 1px;
- every violation: a red marker (small cross or circle) at its `location`,
  plus the violation kind as text;
- corners: tiny dots, so corner-gap cases are visible;
- a 5px clearance halo option (`--svg-halos`): 5px-outlined translucent
  bands around each segment and frame outline, so near-miss geometry
  (exactly-at-tolerance) is visually distinguishable from clearly-clear.

The SVG is self-contained under the same rules as production output (no
scripts/CSS/external refs) — reuse the same discipline, not the production
renderer. The checker draws it from its own primitives; it must not depend
on `render::svg` (independence rule, Section 1). Layout of the SVG is
trivial: one `<svg>` with a `viewBox` of the case's bounding box grown by a
margin.

## 6. Self-test fixtures (must all pass before the checker scores anything)

Hand-constructed micro-cases, each asserting exact violations and exact
numbers. These are committed with the checker. Suggested set (the builder
adds more as edge cases surface):

1. **Clean cross.** Two edges crossing perpendicularly mid-segment; gap
   between the crossing segments is 0 but the relation is perpendicular →
   zero violations.
2. **Parallel ride.** Two horizontal segments 3px apart, overlapping
   projections by 40px → exactly one `parallel_gap_edge_edge`, `gap: 3.0`.
3. **Parallel exactly at tolerance.** Same as 2 but 5px apart → zero
   violations (boundary legality).
4. **Overlap-by-endpoint only.** Two parallel horizontal segments on
   different y, whose x-projections share exactly one x coordinate → zero
   `parallel_gap` violations (a single shared coordinate is not a
   positive-length overlap). Assert the exact expected set — this fixture
   pins the boundary between 4.3.2 and 4.3.3.
4b. **Corner-adjacent parallel overlap.** Two horizontal segments 3px
   apart whose projections overlap by 40px, where the overlap region
   includes one segment's endpoint corner → exactly one
   `parallel_gap_edge_edge` (overlap is projection-based, so the overlap
   itself violates regardless of the endpoint; the corner at the shared
   region edge is legal — perpendicular contact / a straight endpoint, not
   a bend — assert the exact expected set).
5. **Corner near corner.** Two L-shaped edges whose elbows are 4px apart
   diagonally → one `corner_gap`, `gap ≈ 4.0` (Euclidean).
6. **Corner near corner at tolerance.** Elbows exactly 5px apart → zero.
7. **Corner near group corner.** An edge elbow 3px from a group rect's
   corner (not on the outline) → one `corner_gap`.
8. **Corner on group outline.** An edge elbow exactly on a group outline,
   2px from the rect's corner → zero `corner_gap` (outline exemption).
9. **Pass-through.** A segment through the middle of a node → one
   `pass_through_node`, penetration depth asserted exactly.
10. **Grazing pass.** A segment along a node's boundary (open interior) →
    zero `pass_through_node`.
11. **Parallel to group outline.** A vertical segment 2px inside and
    parallel to a group's left frame edge → one `parallel_gap_edge_group`.
12. **Perpendicular crossing of a group outline.** A segment crossing a
    frame at 90° → zero violations.
13. **Non-orthogonal segment.** A 45° segment → one `non_orthogonal`.
14. **Coincident edges.** Two edges with identical polylines →
    `super_flipped` reported, case score 0.
14b. **Label integrity.** An edge whose `label_at` is off its polyline →
    one `label_off_path`; the same case with the anchor on the polyline →
    zero.
15. **Collinear normalization.** An edge with a redundant mid-waypoint →
    `segments` count reflects the normalized form; no violation generated
    by the duplicate point against itself.
16. **Translation invariance.** Fixture 2's case shifted by (137.5,
    -204.25) → identical violations modulo coordinates.
17. **Permutation invariance.** Fixture 2's edges swapped in order →
    pairwise violation set identical.
18. **Shared waypoint corners.** Two chains sharing a frame-crossing
    waypoint → the shared corner pair exempt from `corner_gap`, everything
    else still tested.

Each fixture asserts: exact violation kinds, exact `gap`/`penetration_depth`
numbers (within the doc's epsilon), and severity.

## 7. Corpus

Beyond fixtures, the checker scores a committed corpus:

- The six examples: `infra`, `subdirection`, `cluster`, `finance`, `sides`,
  `tencent` — serialized from the frozen baseline. (These match the sample
  sweep the production test-suite already asserts invariants over, so a
  checker/baseline disagreement on them is meaningful immediately.)
- The dense compound stress graph already used in the tests (24 nested-group
  nodes, 17 edges, generated by the fixed xorshift sequence at
  `src/layout.rs` around line 9740 — reproduce the generator, do not import
  test code) — serialized from the frozen baseline.
- Targeted cases capturing the documented bounded residue (M16/M17): where
  the current engine freezes an edge at its least-bad route, a case that
  isolates that region is the highest-value scoring target — improvement
  there is exactly what the plan is for. Sources: the M16/M17 sections of
  `docs/milestones.md` name the conflict shapes (`sides.dgmr` forced-side
  conflicts; the stacked-group M15-limitation regression in the test
  suite).

Baseline case files live in `experiments/checker/baseline/` (Section 1.1);
the scored corpus cases in `experiments/checker/corpus/`. Neither is ever
regenerated except by explicit orchestrator action, recorded in the checker
README. Agent proposals never live in either directory (Section 8).

## 8. Agent-facing contract (the whole interface, in one paragraph)

An agent writes **proposal case files** (new proposed polylines for a
case's fixed node/group geometry, stored under its own
`experiments/agentN/` directory — never in `corpus/` or `baseline/`), runs
`diagrammer-checker` over them, receives the JSON report, and returns to
the orchestrator: (a) the report files, (b) a one-line
per-case comparison to baseline in the comparison order of 4.2 (e.g.
"infra: hard 3→1, segments 47→52; sides: hard 0→0, segments 18→21 —
worse here, see notes"), (c) its
algorithm description, (d) honest notes on where its routes are worse.
The agent never edits the checker, the corpus, the baseline, or production
code; it may propose fixtures only via the orchestrator (a note in its
final report, applied by the orchestrator after review).

When the orchestrator accepts an improvement into the engine, it re-captures
the baseline files (Section 1.1) and records the **epoch** in the checker
README (`epoch 1: pre-agent baseline, <date>`; `epoch 2: ...`). Comparisons
are always within one epoch — an agent's numbers from an older epoch are
never mixed with a newer one's.

## 9. Out of scope (explicit non-goals)

- Edge-label placement scoring (except the `label_off_path` guard).
- Aesthetic length/curve/bend quality.
- Any change to production code, tests, or snapshots.
- Angle geometry beyond the parallel/perpendicular axis classification.
- Probabilistic or randomized checking (deterministic only).

## 10. Acceptance workflow (how a checker verdict becomes an engine change)

The checker only ranks candidates; it never changes production code. The
path from a good agent report to an engine change is:

1. Orchestrator reviews the winning agent's per-case JSON + diagnostic SVGs.
2. Orchestrator re-verifies on the frozen baseline corpus (the agent's
   numbers are claims until re-run here).
3. If accepted, the port-into-`layout.rs` work happens under the
   production invariants (index correspondence, canonical-space transform,
   determinism, permutation invariance of phase 1) — the checker's goals
   do not replace those, they add to them.
4. Full `cargo test`; `UPDATE_SNAPSHOTS=1` only if routes legitimately
   changed; snapshot diffs reviewed qualitatively (better or equal routes?
   any label knockouts broken?).
5. Re-capture the baseline (new epoch, Section 8).

Steps 3–5 are exactly the "Stage 2 synthesis" from the plan discussion; the
checker's scope ends before step 3.
