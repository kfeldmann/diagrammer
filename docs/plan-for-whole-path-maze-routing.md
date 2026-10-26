# Plan: Option A — whole-path maze routing (M20 series)

> **Status: complete.** M20.1–M20.6 all landed; see `milestones.md` for the
> per-milestone record. This document is the original plan, kept as the
> design rationale — the shipped implementation matches it except where a
> milestone records a measured deviation (notably M20.4's rejection of
> grounding forced sides on the search).

## Design summary

M20 replaces the candidate-ladder regime with a single regime: **every edge is routed by one whole-path A* search from source port to target port over a track graph, against one obstacle model.** Frame-crossing topology is a search output. Candidate generators, the rep model, and the coupled/decoupled duality are deleted. M16's soft occupancy + bounded rip-up loop survives unchanged in spirit and is re-grounded on the new router.

Invariants that do **not** change (from AGENTS.md): index correspondence, canonical-space transforms, GitHub-renderable SVG, deterministic output, baked metrics. Layout (placement of nodes/frames) is untouched by M20 — only `EdgePath` construction changes, plus downstream label anchors which ride the polyline as today.

### The obstacle model (new, in one place)

One struct, replacing `Obstacles`' node/segment/soft/border/corridor partition:

1. **Node interiors** — as today (`ROUTE_PAD` clearance, port-touch exemption).
2. **Solid frames** — unrelated frames, fully blocked: interior *and* border band (perpendicular crossing is not a thing for a solid rect; nothing may touch it).
3. **Ancestor frames' border bands** — blocked for parallel riding only (`BORDER_PAD`, `BORDER_HALF`, perpendicular-crossing exemption survive literally; the track graph keeps the crossing-slot seeding that exists in `track_route` today).
4. **Peer occupancy — soft + port corridors:** as M16 (soft segments negotiable at `SOFT_RIDE_COST`; port approach corridors hard for riding). Done differently, though: see "staged routing" below.
5. **Titles, margins, canvas** — as today.

A per-edge classifier (pure function of the containment tree + endpoint node indices, precomputed once per edge) yields: node world, solid-frame list, ancestor-frame list. This replaces `edge_world`'s per-LCA-context logic.

#### The frame-passage rule, precisely

For edge `e` with endpoints `s`, `t`:

- **Hard obstacles:** every group frame that is *not* an ancestor-or-self of `s` and *not* an ancestor-or-self of `t` — including sibling groups and unrelated deep subtrees. Solid. The router cannot puncture them at all. This kills the "threading through an unrelated group" problem by construction, not by cost tuning — no cost function to get wrong.
- **Invisible-but-rideable-not:** the frames on the ancestor chains of `s` and `t` (and only those). The router freely chooses *where* along each such frame to cross, subject to your stated rule: perpendicular crossing legal, parallel riding on the border barred. Notably the router may now cross an ancestor frame — including the LCA's own frame and re-cross the same frame twice if the geometry makes that cheap — something the rep model could never express.
- The LCA concept, rep crossings, stub/rep decomposition, M14 coupling: all *deleted*. The edge is one polyline from port to port. The containment tree's only remaining routing role is the ancestor-or-solid classification.

One consequence worth naming: because unrelated frames are fully solid, routing becomes *infeasible* in exactly the cases nothing could route anyway (an endpoint fully enclosed by a surrounding unrelated frame's cavity can't exist by construction, since groups are laid out on disjoint rects — good). The genuinely hard cases are edges blocked by a wall of sibling frames; the router must go around, and sometimes the only corridor is between two sibling frames — which is fine and legal, since gaps between frames are open space. This is more robust than the cost-based version precisely because "no path" means "no path," not "expensive path I have to tune down."

One term still needs to exist in the cost model: a strong preference for crossing each ancestor frame as few times as possible (typically once, near where the edge enters anyway). Without it A* will happily pick, say, a 1-bend path that exits through the top and re-enters through the left over a shorter span. A per-frame-crossing penalty of a few hundred handles it deterministically.

### The router

- **Track graph** — `track_route`'s existing machinery, extended: (a) the per-edge bounding region widens beyond ports-and-obstacles so around-the-neighborhood detours are findable (budget: obstacles' extent + `AROUND_CLEAR` + margin, capped by canvas); (b) tracks seeded from solid-frame extents and ancestor-frame bands (centerline slots already exist); (c) cost terms: segment length + `BEND_PENALTY` per bend + `SOFT_RIDE_COST` on soft occupancy + **`FRAME_CROSS_PENALTY`** per perpendicular border crossing (new constant — tuned once, used for both ancestor frames and, if we ever allow slotted solid frames, never here since solid frames don't admit crossings).
- **Port selection.** `from=`/`to=` forced sides bind the node port as today (M11/M14 semantics preserved at the *node* only). Unforced sides: the search runs from the node's four plausible escape stubs (per `Escapes`) and the port's free coordinate along the node side is chosen by the search itself — the search picks the port coordinate, then `port_at_cross`-style separation is replaced by the pocket mechanism: routed peers reserve corridors, and the search pays to enter an occupied corridor. This *is* the frame fan — emergent instead of fanned.
- **Staged routing (M16 carried forward, unchanged in outline):** order edges deterministically (declaration order, as today), route each whole-path against current soft occupancy; then the bounded rip-up loop (`MAX_RIPUP_PASSES`) rescans violations and reroutes whole paths. Scan + reroute order fixed → deterministic; phases gated by the existing `layout_phase1` test hook.
- **Self-loops & forced-direct escaped stubs:** self-loops get a dedicated small generator as today (the search is wrong for loops); forced sides outside the search's natural port set seed the search's start nodes. No `forced_direct_path`/`contradicts` ladder — the forced port is simply where the search begins.

### Port & fan semantics that must be re-established explicitly

The current fan code (`frame rep-crossing fan`, `port separation pre-passes`, `port_alternates`, `pocket_rects`) has real behavior users see (edges into a group side spread out; port approach corridors). In the search regime these must be explicit, not emergent-by-luck:

1. **Pre-pass 1 (port separation) survives**, reframed: before any search, allocate each node's and ancestor-frame-side's *crossing coordinates* — i.e., for each side-of-frame that k edges will use, check they have distinct coordinates (seed tracks so crossings can't collapse). The A* returns crossing coordinates near these seeds.
2. **Reserved pockets survive** by literally keeping `pocket_rects` + `push_corridor` fed from the *new* paths' approach stubs.

### Milestone sequence

**M20.1 — Solid/ancestor classifier + track-graph region widening (foundation, no behavior change).**
Ancestor/solid classification per edge; extend `track_route` to accept the per-edge frame world and a widened bounding region; unit tests for classification (deep nesting, sibling walls, root-level endpoints). Existing routing untouched.

**M20.2 — Whole-path search for direct internal edges.**
Route every *within-one-level* edge via the search (start from the flat engine's port points). Compare snapshots: expect most paths identical (the search will re-derive the same Z because it costs the same), some improved. No deletion yet — the ladder for cross-boundary edges stays. This milestone is the retreat point: if search quality disappoints on direct edges, we learn it here cheaply.

**M20.3 — Whole-path search for cross-boundary edges; delete the rep model.**
The big one. `cross_pieces_candidate`, `around_target_candidate`, title detours, `mismatched_stub`, `route_lca`, `refresh_rep_crosses`, M14 coupled/decoupled duality all *deleted* — replaced by the search against the M20.1 frame world. Title detour becomes emergent (the search routes around the title text rect, which joins obstacles). `CROSS_FRAME_PAD` likely retires. Expect a large snapshot regeneration; budget a deliberate review pass over each.

**M20.4 — Frames bands, fans, pockets, repair loop re-grounded.**
Port separation pre-pass re-anchored on search outputs; reserved pockets from new stubs; the M16 rip-up loop repointed at whole-path reroutes (it already reroutes whole paths — verify). Dense-stress fixtures re-baselined.

**M20.5 — Labels re-tuned (M13 still holds its contract).**
`place_labels` runs on the new polylines unchanged, but the landscape changes (fewer within-frame stub geometry → candidate anchors change). Keep the M13 pass; re-tune any title-clearance interactions that depended on deleted code.

**M20.6 — Deletion & doc pass.**
Remove dead constants (`STUB_JOG_CLEARANCE`, possibly `CROSS_FRAME_PAD`, rep-related chain helpers), update module docs, grammar-tooling untouched. Update `docs/milestones.md` with the routing architecture story.

### Tests

- Classification unit tests (M20.1) — the containment-tree semantics deserve exhaustive nesting cases.
- Whole-path property tests: no polyline rides an ancestor frame's band parallel; no polyline inside a solid frame; port-touch rules; determinism (same input → byte-identical snapshots, already enforced).
- The existing M9/M16-style dense fixtures re-run as regression gates; new "wall of sibling groups" and "deep nesting" fixtures.
- Snapshot regeneration is expected at M20.3 and M20.4 (committed, reviewed deliberately — not `UPDATE_SNAPSHOTS=1 && done`).

### Risks & mitigations

- **Performance.** Per-edge A* over a track graph is O(tracks²) per edge; tracks scale with nodes+frames (tens–low hundreds on real diagrams). M16's rip-up loop reroutes on conflicts — reroute count is bounded by `MAX_RIPUP_PASSES × violation count`. Add a per-edge node budget with a documented terminal fallback (the plain Z, as today) so pathological cases degrade deterministically rather than hang. Non-goal: optimality; goal: good paths fast.
- **Quality regressions on the taste-encoded cases** (title band, port fans). M20.2 is the canary; fans and pockets are explicit milestones (M20.4), not assumed.
- **Snapshot churn** across M20.3/M20.4 — deliberate review, per the project's convention.
- **`label_at` coupling.** Rides the polyline; survives. But revisit the "may overhang a frame outline" rule after M20.3 if the router now produces longer runs along borders (labels that would cover a neighboring edge line remain the binding constraint, not the frame rule).

### Disallowed-by-design

No randomness, no hashmap-iteration leakage, no per-edge view of others' *hard* occupancy during the first pass (soft only, as M16 keeps it), no pathological looping (bounded passes, terminal-fallback ladder). This keeps the byte-stable invariant intact.

---

### My recommendation for where to slice risk first

If you want the fastest path to certainty, do **M20.1 + M20.2 in one sitting** — the direct-edge-only search is a contained change (no rep deletion), it exercises every piece of the new track-graph infrastructure, and it produces a quality signal on the examples the same day. If M20.2's snapshots look good, M20.3 is confident work; if not, adjusting the cost model is cheap because nothing structural has been deleted yet.

