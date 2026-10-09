# Clanky session: discuss-plan-for-better-routing

| | |
|---|---|
| Provider | deepinfra |
| Model | zai-org/GLM-5.3-Flash |
| Created | 2026-10-09 03:49 |
| Contents | user and assistant messages |

## User

I'd like to discuss an idea... Please read ./plan-for-agent-based-routing-improvement.md. The plan is to develop a new routing system for diagrammer, and I'd like to have the LLM think of the most effective way to use child agents (subagents) to pursue a well-defined goal. What do you think about this plan? Can you think of how to guide the subagents?

## Assistant

I've read the plan, the milestones doc, and skimmed the routing machinery in `src/layout.rs`. Here's my honest read.

## What's good about the plan

**The Goal list is the strongest part.** Every item is mechanically checkable: parallel-riding and 5px clearances are rect/interval geometry, corner proximity is distance between elbow points, "may cross perpendicularly" is an angle test, "fewer segments prioritized" is a count. That means the whole exercise can be reduced to **generate-and-test against a scorer**, which is exactly the regime where subagents shine — they don't need to coordinate, they just need a shared, unambiguous yardstick.

## What's missing / risky

1. **No baseline measurement.** M16/M17 already implement global routing + rip-up with a *documented bounded residue* ("frozen least-bad routes"). Before spawning anything, score the current output on all six examples plus the dense stress fixtures. If the score is already 95+, the plan should say what score counts as success and what "better" means (residue cases? robustness on denser graphs?).
2. **The goals are lexicographic, not scalar.** "Must not pass through a node" is a *hard* constraint; "fewer segments" is a *soft preference*. A single weighted score invites agents to trade a pass-through violation for two fewer segments. The scorer must be lexicographic: hard violations first (severity-ordered), segment count as tiebreak.
3. **Unstated invariants.** The plan's goal list omits things that pin this codebase: determinism (no randomness/iteration-order leakage), permutation invariance of phase 1 (enshrined in a test), port-right-of-way, label knockout zones, frame-title detours, arrowhead backoff geometry. Subagents *will* violate these unless you put them in their system prompts verbatim and in the checker.
4. **File ownership.** Four agents editing `src/layout.rs` (9920 lines) simultaneously is a merge disaster. Prototypes must live in isolation.

## How I'd structure the subagent strategy

**Stage 0 — no agents (or one): build the yardstick.**
Write `experiments/checker/`: a standalone binary/lib that takes a `.dgmr` + a produced layout and emits a lexicographic score + a machine-readable violation report (edge indices, segment indices, violation type, coordinates). Plus a benchmark corpus: the 6 examples + `sides.dgmr` (the known hard case) + generated dense/nested stress graphs from the fixed LCG pattern the existing test uses. This is the highest-leverage artifact; a main agent can do it faster and more consistently than a subagent.

**Stage 1 — 4 parallel exploration agents, one paradigm each.**
Give each agent the same contract and let them differ only in algorithm, so results are comparable:

- A: grid/A* with layered track spacing (extends the existing `track_route` idea)
- B: visibility-graph + shortest-path with segment-count weight
- C: orthogonal channel/bus routing (assign corridors globally, then route within)
- D: iterative improvement (route all, then repair via the checker's violation report as the objective — essentially "let the scorer drive rip-up")

Each gets: the goal list verbatim, the invariant list (determinism, canonical space, no randomness), permission to write **only** `experiments/agentN/`, a read-only pointer to `src/layout.rs`'s obstacle world as reference, and instruction to output routes through a common trait signature: `fn route(world: &ObstacleWorld) -> Vec<Vec<Point>>`. Report format fixed: score, per-benchmark violations, segment counts vs. current baseline, honest failure notes.

**Stage 2 — one synthesizing agent (or the orchestrator).**
Takes the winning prototype(s), reports, and the checker; plans the incremental port into `layout.rs` respecting the index-correspondence invariant and the EdgePath/label_at contract; runs the full suite + `UPDATE_SNAPSHOTS=1` and diffs the snapshot changes as a qualitative review.

**Key guidance principles for the subagents themselves:**

- **Score first, code second.** Every prompt: "run the checker on the baseline before writing anything; your score report must include it."
- **Small verification loops.** Agents should be told to test per-benchmark incrementally, not big-bang.
- **Forbid gold-plating.** No new snapshot regeneration, no touching tests they don't own, no refactoring shared code.
- **Honest failure reporting over heroic hacks.** A prototype that scores worse than baseline but explains *why* (e.g., "grid resolution vs. 5px clearance mismatch") is more valuable than one that games the scorer.
- **Watch for scorer gaming.** Since agents optimize the metric, keep the checker separate from the agents (owned by the orchestrator), and spot-check winner outputs visually via rendered SVGs.

One more thought: 4 parallel agents is most useful in Stage 1 because the paradigms are genuinely independent. If you're tempted to parallelize the *port-into-layout* step too — don't; that step is all about conforming to existing invariants, which is a coherence-sensitive single-owner job.

Want me to draft the actual system prompts / agent contract text, or the checker spec, as the next step?

## User

Yes, please draft a spec for the checker. By "may cross perpendicularly", we don't have to check angles because the routing is orthographic - it only uses vertical or horizontal segments (or you can check angles, but only to see whether the lines are parallel or perpendicular).

## Assistant

The spec is drafted and committed to the repo at **`docs/checker-spec.md`** (I also added a pointer to it in `AGENTS.md`'s "read first" list). Here's a summary of what it contains and the key decisions I made while writing it:

## Structure of `docs/checker-spec.md`

| Section | Content |
|---|---|
| 1 | Role in the plan: orchestrator-owned, independent from production checking logic, must pass its own self-tests before scoring anything. Frozen baseline + **epochs** for re-baselining. |
| 1.2 | Three consistency properties: translation invariance, permutation invariance (pairwise violation sets), and the **super-flip guard** (two edges with identical polylines → score 0 — the anti-gaming tripwire). |
| 2–3 | CLI (`diagrammer-checker`, plus an orchestrator-only `--emit-baseline` that runs the frozen engine once), and the case-file schema. JSON on disk (no serde in the workspace, so a small hand-rolled reader/writer); YAML only appears in the doc as an illustration. |
| 4 | Report shape + the **lexicographic comparison order** (hard count → kind signature → segment count → polyline tiebreak). |
| 4.3 | The normative geometry: all six violation kinds with exact definitions. |
| 5 | Diagnostic SVG with violation markers, plus an optional `--svg-halos` mode drawing 5px clearance bands so at-tolerance vs clearly-clear geometry is visually distinguishable. |
| 6 | 19 hand-constructed fixtures (including the boundary-exactness ones: exactly 5.0px apart is legal, grazing a node boundary is legal, corner on a group outline is exempt). |
| 7–10 | Corpus (6 examples + the dense xorshift stress graph + bounded-residue cases), the agent-facing contract in one paragraph, explicit non-goals, and the acceptance workflow from checker verdict → engine change → snapshot review → new epoch. |

## Key decisions worth flagging for your review

1. **Angles → axis classification, as you specified.** Parallel = same orientation, perpendicular = opposite; anything else is a `non_orthogonal` hard violation. So the "may cross perpendicularly" goal needs no angle math at all — a perpendicular crossing simply never triggers the parallel-gap or corner-gap rules, and fixture 1 (clean cross) and 12 (frame crossing) pin that.

2. **Open-interior pass-through.** A segment may graze a node's boundary without violation. This is deliberate and load-bearing: ports sit exactly on box boundaries and the renderer shortens arrow ends into the boundary, so a closed test would flag every legal terminus. Fixture 10 pins it. (Boundary-riding along a group outline is still caught by `parallel_gap_edge_group`.)

3. **The overlap rule does double duty.** `parallel_gap` requires a strictly positive-length projection overlap — so two parallel segments whose projections merely *touch* at one coordinate aren't gap-violations even at distance 0; that contact is judged as a corner adjacency instead. This prevents T-junction-adjacent geometry from double-counting, and fixtures 4/4b pin the boundary.

4. **Shared-waypoint exemption (4.6).** Where the engine's chain routes share a frame-crossing waypoint, two polylines pass through the same point — one physical corner. The `corner_gap` test waives exactly that pair, nothing else. Without this, the checker would drown the baseline in false positives.

5. **Scoring is a comparison order, not a number.** Since "must not pass through" (hard) and "fewer segments" (soft) are different currencies, the spec defines a 4-level lexicographic ordering with a per-kind signature so a candidate can't trade a pass-through for a parallel-gap and claim victory at level 1.

One thing I want to flag honestly: the spec pins several exact-expectation fixtures (4, 4b, 8) where the "right" answer depends on how strictly you read boundary contact — I wrote down my reading and made the fixtures assert it exactly, so if you disagree with any of those readings, they're one-line changes to the spec *before* the checker is built, and painful to change after (agents will have optimized against them).

Next natural step, when you're ready: the system prompts + contract text for the four Stage-1 exploration agents, written against this spec.


---

_session total: 3569983 prompt tokens · 24679 completion tokens_
