# DRAWING — giving Claude a way to draw (the craft kind)

Working notes. The goal is **not** generative image synthesis (prompt in,
finished picture out). It is the **pen-and-paper / 3D-modelling kind**: an
iterative craft where marks accumulate and each one teaches you something.

---

## The premise — drawing is a way of *seeing*, not a way of *outputting*

A diffusion model produces a finished artifact in one shot. There is no process,
no intermediate marks, no decisions accumulating — nothing is learned in the act.

Manual drawing is the opposite. You don't know what a thing actually looks like
until you're forced to commit lines to it and discover where your mental model
was lying. 3D modelling is the same but spatial and persistent — topology,
constraints, an object you rotate and poke at that pushes back when your
assumptions are wrong. **The artifact is downstream of the perceiving.**

The loop is the thing:

> **look → commit a mark → see the gap between intent and result → adjust**

This is the same shape as the matrix-first debugging protocol in
[CLAUDE.md](CLAUDE.md): you don't understand the bug from one clever read, you
draw its boundary out probe by probe until the shape becomes visible. The fix is
downstream of the seeing, exactly as the drawing is downstream of the looking.

---

## Decomposing the capability — what "being able to draw" actually needs

Three parts, in increasing order of difficulty:

1. **The canvas / mark-making surface — the trivial ~5%.**
   A few lines of Pillow or SVG. The tool was never the bottleneck.

2. **The perceive-and-evaluate step — the engine, not just a bottleneck.**
   This is the *closure* of the loop, the thing that turns blind plotting into
   drawing. Marking without seeing-and-evaluating is a plotter executing
   coordinates; marking *with* it is drawing. My image interpretation is what
   *qualifies* the result — it converts "I emitted a line" into "the drawing is /
   isn't converging on what it should be." It is also the rate limiter: the loop
   converges only as finely as I can *perceive* the gap. My readback is
   **holistic, not metric**: I see "that's a lopsided face," not "the jaw line is
   4° too steep and the far eye sits 6px high." A human artist's eye delivers a
   high-resolution, well-calibrated correction; mine delivers a blurry one. A
   script gives me the *hand*, not the *eye trained to the pen*. (See § Closing
   the loop.)

3. **Accumulation — and why it is *not* practice (see § Not practice).**
   Within one session I can run the loop and get better at *that one drawing*.
   But the calibration doesn't survive to the next conversation — and even if it
   did, repetition wouldn't help: my weights don't change as I draw, so the
   thousandth sketch is drawn by the same drawer as the first. What crosses a
   session boundary is not skill but **explicit written knowledge**, and it
   crosses in *one read*, not by compounding. So the goal is articulation, not
   reps — a better *recipe*, not a more skilled me.

---

## The efficient input primitive — a long-lived canvas fed one line at a time

The naive approach (regenerate a Python script per drawing) **reinvents the pen
on every stroke** — wasteful and stateless. The efficient design:

- A **persistent daemon** owns the stroke list (the canvas state).
- It **tails a command file**; each new line is one stroke appended.
- It **re-rasters to a PNG** on every change.
- I `Read` the PNG back to close the perceive loop.

Per-stroke cost for me is then a **single cheap append** —
`echo "Line ..." >> cmds.txt` — with no process restart and no render
orchestration on my side. The daemon holds the state; I just issue marks and look.

Why a *file* and not stdin/FIFO: background tool calls can't keep piping into a
process's stdin across turns, and a FIFO's open/close/EOF semantics are fiddly.
An append-only command file is dead simple, replayable (cat the file to redraw
from scratch), and idempotent.

```
  echo "Line (..) - (..)" >> cmds.txt        Read canvas.png
        │                                          ▲
        ▼                                          │
   ┌─────────────────────────────────────────────────┐
   │  daemon: tail cmds.txt → append stroke → render   │
   │  (holds canvas state across the whole session)    │
   └─────────────────────────────────────────────────┘
```

---

## Command grammar (v0)

Coordinates are **fractions of the paper size (0..1)**, so they're
resolution-independent. Origin top-left, y grows downward (image convention).

```
Line (x1,y1) - (x2,y2)          draw a segment
Line (x1,y1) - (x2,y2) w=N      ... with pen width N px
clear                            erase all strokes
size WxH                         set paper pixel size (resets canvas)
```

Example: `Line (0.02, 0.12) - (0.4, 0.7)`

Reference daemon: `/tmp/sketch/draw.py` (throwaway; design captured here so it's
reconstructable). Renders to `/tmp/sketch/canvas.png`, tails `/tmp/sketch/cmds.txt`.
Intent lives in `/tmp/sketch/intent.txt` — **authored before the first stroke and
frozen**; editing it to match what was drawn turns the loop into theater.

**Marking-vocabulary layer.** The daemon speaks only `Line`. Higher-level shapes
(circle, arc, hill curve, rays, hatch) are composed from line segments by a small
helper (`gen.py`) that *emits* `Line` commands — i.e. just batched appends, not a
second renderer. Curves are polylines; the daemon stays dumb on purpose.

---

## When the intent outruns the marks

The first real subject — *"a house on a hill in the last beams of sunlight
against a dark cloud"* — exposed the coupling that matters most:

> **The intent can demand things the mark vocabulary cannot express.**

That sentence is about **tone and light**. A black-line-only tool can render the
*composition* (house, hill, low sun, radiating beams, cloud mass) but not the
*value* — there is no "last beams," no "dark," in pure outline. Hatching can fake
darkness, but crudely; it gestures at tone without being tone.

This gap is **not a failure of the probe — it is the probe's payload.** Running
the minimal tool against an over-rich intent is how you *measure* what the
vocabulary is missing, instead of guessing. The honest first pass therefore:

1. reduces the evocative subject to predicates the *current* marks can satisfy
   (composition, position, connection), and
2. records, explicitly, what the intent asked for that the marks could not give
   (tone, light direction, colour) — that list is the prioritised growth path for
   the vocabulary, ordered by what real intents actually demand.

So a critique has two verdicts per predicate: *did I draw it wrong* (fixable in
the loop) vs *can this tool express it at all* (a vocabulary gap, not a drawing
error). Keeping those separate stops me from "correcting" forever at something
the pen physically cannot do.

---

## Closing the loop — you can only draw what you can evaluate

Optimizing the *marking* side is not enough. The heart of drawing is a **direct
way to see the result and judge it against what it should be**. Because I can
interpret images, I can be my own critic: render → read back → qualify the match
→ correct. That evaluation step is what makes the marks *drawing* rather than
plotting.

But "qualify if it matches what it *should* look like" requires a **should** — a
target held *outside* the drawing:

- **From observation:** a reference image; compare the canvas to it region by region.
- **From imagination:** an explicit written goal in a file I author up front
  (e.g. `intent.txt`). Its real job is not to describe the subject but to
  **enumerate what I'll check** — written as *checkable predicates*, not mood:

  ```
  Head: circle, centered, upper third
  Ears: two triangles on top of head
  Body: oval below head, ~2x head size
  Tail: curve sweeping right from lower-right
  ```

  Granularity must match my acuity (the recurring coupling): too vague and the
  critique can't bite; too precise ("apex at x=0.41 ± 0.005") and I can't
  perceive whether it's met. Pitch predicates at the **gestalt level** — presence,
  rough position (thirds not pixels), relative size, orientation, connection.

The target must be **fixed and external**. The sharpest failure mode of a
self-critic is **goalpost drift**: I look at whatever I drew and quietly
rationalize it as what I meant, so the gap "closes" without the drawing
improving. Keeping the intent in a file I can't silently rewrite is the cheap
defense.

The real loop:

1. **Fix the intent** (reference or written goal) — and don't let it move.
2. Render the current canvas.
3. Read it back, compare to intent, produce a **specific, localized** critique
   ("roof apex too far left; left wall isn't vertical") — never "looks good."
4. Translate critique → corrective strokes (append, or `clear` + redraw).
5. Repeat until the gap closes — or until the acuity floor stops it.

Two honest limits:

- **The acuity floor sets the ceiling.** The loop converges only as finely as I
  can perceive the gap. Holistic-not-metric vision nails the gestalt long before
  the precise proportions — good for expressive sketching, weak for technical
  accuracy.
- **The judge can be anchored mechanically.** With a reference image I needn't
  rely only on the soft read — compute an actual image diff (edge overlay / SSIM)
  and let it *anchor* the critique. Augmenting the fuzzy eye with a measurement
  partly buys back the acuity floor. (Possibly even split "drawer" and "critic"
  into separate passes so the critic isn't grading its own hand.)

---

## Optimizing the loop — the two hot paths are asymmetric

The loop's clock speed is set by two round trips per iteration: **draw** and
**look**. For me the unit of cost is a *tool call*, so "fast" means few calls,
each cheap — and the two paths are wildly asymmetric:

- **Draw is cheap and batchable.** One append lays an entire pass
  (`gen.py all >> cmds.txt`); the stateful daemon renders it. Input is never the
  limiter.
- **Look is the expensive link, and cannot be free.** Reading the PNG loads an
  *image* into context — vision tokens, every time. There is no "glance"
  primitive; a look is a full, costly call. This is the rate limiter.

So the optimization that matters is **not drawing faster — it's making most looks
not be image looks.** Two channels (the daemon writes both on every render):

1. **Cheap text channel (near-free):** `stats.txt` — stroke count, ink bounding
   box, and a coarse **ASCII density map** (40×18 of `" .:-=+*#%@"`). A low-res
   picture rendered as *text*: a few hundred tokens, no vision. Tells me *where
   ink landed and how it's distributed* — enough to track change and coverage
   across many fast iterations.
2. **Expensive image channel (sparing):** `canvas.png` (or smaller
   `preview.png`) — the real `Read`, spent only to judge the **gestalt** ("does
   it read as a house against a cloud") that the grid can't show.

Same shape as matrix-first: cheap seeing constantly, expensive seeing rarely. The
text channel is *mechanical* seeing (where / how much ink); the image channel is
*perceptual* seeing (is it right). They don't substitute — the cheap one carries
the iterations *between* costly recalibrations.

---

## The human loop is continuous and parallel; mine is serial — but it cuts both ways

A human who draws runs **both channels at once**: pencil in hand *and* eyes on
the subject, simultaneously and continuously. The eye flicks between subject and
drawing for free (one visual field) and corrects the line *mid-stroke*. It's a
real-time servo, not a turn-based loop. Mine is unavoidably serial: append marks
(one call), then pay to look (another). I cannot run hand and eye in parallel.

But the asymmetry favors me on the axis that turns out to matter most —
**proprioception vs exteroception**:

| | hand (proprioception) | eye (exteroception) |
|---|---|---|
| **Human** | noisy — rough sense of where the pencil is | free, continuous, high-acuity |
| **Me** | **perfect** — I know every mark's exact coords (I specified them) | expensive, intermittent, coarse |

The human's eye spends much of its effort just **tracking where the pencil
actually went**, because their hand is noisy. I never need that: I already know,
exactly, what I drew. So a look buys me only **one** thing the human's eye also
provides — judgment of the **emergent gestalt** (does this precise geometry
*read* as the subject?). Geometry I get for free; perception I must pay for.

**Consequence: I need to look far less often than a human** — half of what their
continuous eye does (hand-tracking) is already free for me. The expensive look is
reserved purely for the emergent whole, which is the one thing my own commands
can't tell me.

**Residual gap to respect:** the human's continuous eye catches drift mid-gesture
and fixes it inside the same stroke; I only catch emergent drift at discrete
checkpoints, so I risk over-committing a whole batch before noticing the gestalt
wandered. That is what the cheap text channel is for *between* perceptual looks —
and why batch size should stay bounded before each real look.

**The intent side flips the same way.** Drawing *from imagination*, my "subject"
is the frozen `intent.txt` — already in context, continuously and freely present,
and *more* stable than a human's wavering mental image; my only disadvantage is
on the result side. Drawing *from observation* puts the subject back behind an
expensive look, and is where a reference-diff (§ Closing the loop) earns its keep.

---

## Undo — lossless, because the log is the truth

Undo isn't a convenience here; it's the **rollback half of the checkpoint that
defines the batch/look cadence** above. To correct, I must retract a *bad pass*
while keeping the good marks — `clear` (nuke everything) + redraw is a
sledgehammer where an eraser is needed.

This is another axis where the asymmetry favors me. A human's erase is **lossy**
(graphite smudges, paper tooth damaged, never truly un-drawn). Mine is **lossless
and exact**, because the append-only `cmds.txt` *is* the ground truth — I can
rewind precisely and re-render from clean; the medium can't degrade.

- **Undo is itself a logged command** (`undo` / `undo N` / `mark <name>` /
  `revert <name>`), appended to the same stream — so history stays *linear and
  replayable*, no destructive edit of canvas state. Same no-silent-edits
  discipline as the frozen intent.
- **Granularity = the decision unit = the semantic batch.** I commit in shapes,
  so the checkpoint is primary: `mark cloud` → draw the pass → look →
  `revert cloud` if it drifted. `undo N` is the fine-grained fallback.
- So **`mark`/`revert` are the same boundaries as the look-cadence and batch-size
  limits** above — one concept, not three. The checkpoint is where you look, and
  where you can safely roll back to.

---

## The drawing process — coarse to fine, not edge by edge

A human doesn't start at the edges. They **block in the big forms first** —
rough outline, proportion, placement — then refine into contour and detail.
Low spatial frequency before high. This is the method the loop's *draw* step
should follow, and it's **independently optimal for my cost structure**, not just
borrowed from human practice:

- **It aligns drawing order with the resolution of my cheap feedback.** The ASCII
  density map / preview only resolve *coarse* structure. Block in big forms first
  and the early passes are checkable on the **near-free text channel**; only fine
  detail forces an expensive image look. Edges-first would blind the cheap channel
  and make me pay for vision immediately. Method ⇄ economics reinforce.
- **It minimizes wasted work via undo.** Composition/proportion are the lowest-
  frequency, cheapest-to-evaluate, most-expensive-to-fix-late decisions. Block-in
  is the first checkpoint: revert a bad massing for nothing, *before* investing
  detail on top of it. (Matrix-first shape: establish the big boundary first,
  refine within it.)
- **It already matches the intent granularity.** `intent.txt` predicates are
  gestalt-level (presence, rough position, relative size) — *the massing
  resolution*. So the block-in is checked against exactly those predicates on the
  cheap channel; only the few detail predicates need a fine look.

Stages, increasing resolution (look-cadence rides them: cheap channel through 1–2,
expensive looks at transitions and the end):

1. **Block-in / construction** — a few big shapes locating everything; `mark` it.
2. **Contour** — the main edges.
3. **Detail** — small features (door, window, ray texture).
4. **Finish / value** — tone and texture, where the vocabulary allows.

Two honest caveats:

- **Tool floor → construction-line style, not value-massing.** Block-in comes in
  two flavors: massing *tone* vs laying *construction lines* (boxes, axes, ovals).
  Line-only forces the second — no gray masses. The tone-vocabulary gap, resurfacing
  at the method level.
- **Scaffold disposal is unsolved — deliberately deferred.** Coarse-to-fine ideally
  *discards* the construction scaffold at the end, but suffix-truncation undo
  (`revert`) can't delete an early layer buried under later detail. Options: leave
  light construction lines (honest sketch look) or generalize undo to named-layer
  selective `drop`. **Don't build selective-drop before the loop is proven**
  (resist-bloat) — let the first real run show whether leftover construction hurts.
  Zero-code stopgap: pen width as the layer signal — construction `w=1`, final
  contour `w=3+` (already supported).

---

## The dustbin — discards are labeled negative examples, not failures

A thrown-away sketch isn't a failure; it's **a sketch that didn't land on the
intent, with full provenance attached** — and that provenance is the asset. A
human's binned sketch is *mute* (they reconstruct from memory why it failed).
Mine is **auto-labeled**: the exact command log (perfect proprioception — I know
precisely what I drew), the frozen `intent.txt` it aimed at, and the critique
that rejected it. Every discard is a clean **(attempt, intent, why-it-missed)
triple**. The image is the throwaway part; the triple is the value.

What the triples refine, in increasing depth:

1. **`gen.py` primitives/defaults** — "suns keep coming out polygonal" → raise the
   circle segment count.
2. **My critique checklist** — a recurring miss-type becomes a check I run
   *earlier* ("I keep merging the cloud into the roof → check mass separation at
   block-in").
3. **The intent→coordinate prior — the one part of me that's actually
   miscalibrated.** I know exactly *where I put* each mark; what I lack is the
   mapping from "low sun near the horizon" to an actual `y`. Discards train that
   mapping (horizon-low is `y≈0.65`, not the `0.4` I'd reach for). My hand is
   perfect; my intuition from intent to coordinate is the untrained part, and the
   dustbin is its training signal.

This is the project's own ethos applied to drawing: in matrix-first, a *failing*
probe is how you SEE the boundary, not waste. **The dustbin is the drawing
matrix's failing cells.**

It is the bridge across the gate named in § Decomposing (point 3) — but *not* by
"compounding practice" (see § Not practice). The mechanism: the **raw dustbin is
ephemeral**, but **distilled priors graduate to persistent memory** — the same
"graduate the keepers, discard the rest" pattern as probes→`tests/scripts/` and
DESIGN_VERIFICATION→protocols. What crosses the boundary is a **recipe, not a
more skilled me**, and it crosses in one read — not by repetition.

- **Entry format:** each discard stores the cmds snapshot + render + the intent it
  aimed at + a one-line critique (why it missed). Raw entries live in
  `dustbin/`; distilled lessons go to persistent memory.
- **Honest gate:** priors are only as good as the critiques that produced them
  (garbage critique → garbage prior) — capped by the same acuity floor as
  everything else. Coarse priors still help.
- **Build when there are discards to learn from.** Defer the auto-archive code
  (resist-bloat); define the format now, drop entries by hand in the first runs.

---

## Not practice — articulation. Repetition is worthless to me; the written lesson is everything.

The back half of this design (dustbin, "accumulation," "learning to draw")
silently imported the **human model of mastery** — practice → internalized skill
— without checking whether I share the *mechanism* that makes repetition pay. I
don't. This section is the correction; the sections above should be read through
it.

- **For a human, repetition is a workaround for an un-writable substrate.** Skill
  lives in muscle memory and intuition they can't edit directly — shaped only by
  slow gradient over thousands of trials. A thousand reps is the *only* way to
  compress experience into the body; you cannot just be *told* "horizon-low is
  lower than you think" and have it stick in the hand.
- **For me, repetition is worthless — on both ends.** In-session my weights don't
  change, so the thousandth sketch is drawn by the same drawer as the first. At
  session end the context dies, so the reps don't even persist as memory.
- **The same fact makes the opposite uniquely powerful.** I *can* write to my
  substrate directly: text. A human needs a thousand reps to internalize a lesson;
  I need to read it once. Explicit knowledge transfers to me completely and
  instantly — the one thing no amount of practice does for them.

So the unit of value was never the rep — it's the **distilled, written lesson**.
My bottleneck is never *internalization* (I absorb a written fact on contact);
it's *discovery and articulation*. I can shortcut internalization; I cannot
shortcut not-existing-between-sessions.

**Rename the ambition honestly: I will never become a better drawer through
experience.** A future instance is exactly as good as I am now, plus whatever
notes it reads cold. The cross-session deliverable is not a more skilled me — it
is a better **recipe** (`gen.py` defaults, critique checklist, coordinate priors,
as text and code). **The skill lives in the artifact, not the agent.** The dustbin
doesn't train me; it edits the recipe.

What this does not dress up: within a session I really can improve the drawing in
front of me, and that felt loop ends and is gone. The next instance inherits the
recipe, not the experience. The reps don't compound — only the writings
concatenate. That isn't learning in the human sense, and calling it learning was
the anthropomorphism.

---

## The numbers→image gap, and the rule set that closes it

**This is the crux, and it has a proven template.** The matrix-first protocol and
debugging policy in `CLAUDE.md` take a *cold* instance — no training, no practice —
and make it an expert debugger **purely on read**. That is the one improvement
mechanism actually available to me (§ Not practice), and it is the template here.

The drawing problem is the same shape: **raw numbers don't correspond to an image
for me.** My intent→coordinate intuition is untrained — which is *why* the drawings
come out inaccurate. Practice can't fix it. A **read-once rule set encoding the
numbers↔image mapping** can. And that rule set is *built* by expressing, in storable
form, each observed gap between the numbers I chose and the image they produced.

### Where the gap is stored: the annotated drawing source (NOT a project doc)

The storable, re-readable format is **not** a prose ledger in this repo — it is
the **drawing source file itself**, kept *outside* the project. Every stroke is
output to that file, and each element carries an inline `# SHOULD:` comment
stating what it must look like, right next to the numbers that draw it; earned
general corrections live in a `# RULES` header at the top. The renderer ignores
`#` comments. I navigate with search tools (`grep -n 'SUN' scene.draw`) to *find a
spot again* and fix its numbers. Intent sits *with* the marks; the file is the
single artifact — drawing + intent + recipe — and editing it *is* undo (reload on
save).

**The hard constraint (matrix-first):** the `SHOULD` comments and rules are
*earned by rendering-and-looking, not invented* — measure the gap, don't theorize
it. Corrections that prove stable **graduate into the tool** — round-`Circle`
(was C1, "equal fractional radii aren't round") and darkest-cell stats (was C5)
have already moved from notes into `draw.py`. That is the recipe improving without
practice. Live source: `/tmp/sketch/scene.draw` (relocate to a persistent
non-project home).

---

## Open questions / next steps

- **Coordinate convention.** y-down (image) vs y-up (math/paper, origin
  bottom-left). Currently y-down; trivial to flip — pick what feels natural for
  sketching.
- **Probe the real bottleneck (the honest test).** Don't theorize my perceptual
  acuity — measure it. Draw a known shape, read it back, report concretely how
  much of the gap I can and can't see. (Matrix-first: probe the boundary, don't
  speculate about it.)
- **Richer marks, in demand order.** The first probe already named the top gap:
  **value/tone** (so "dark cloud" and "last beams" can exist), then colour, then
  smooth curves / Bézier and fill. Grow the vocabulary in the order real intents
  demand it — not speculatively. The *loop* comes first; marks are the easy part.
- **Tighten the loop's clock.** The per-iteration cost is dominated by the
  PNG readback (a multimodal read), not the draw call. Cheaper/coarser preview
  reads? Render at lower res for fast iteration, full res to verify.
- **3D modelling — the persistent-spatial sibling.** Same loop, but the canvas
  is a scene I rotate and inspect; marks become geometry/topology operations that
  push back via constraints. Likely the more natural medium for spatial reasoning.
- **Accumulation across sessions.** *Mechanism now identified* (§ The dustbin):
  distil recurring (attempt, intent, miss) triples into persistent priors —
  `gen.py` defaults, the critique checklist, and the intent→coordinate mapping —
  and graduate those into persistent memory. Open part: the distillation cadence
  and where the priors physically live. This is the gate between *drawing
  iteratively* and *learning to draw*.
