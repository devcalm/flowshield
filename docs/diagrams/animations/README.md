# Algorithm animations

Simulation-driven GIF animations of FlowShield's two v1 rate-limiting algorithms. Each frame is generated from a Python port of the **exact** admission logic in [`crates/flowshield-core/src/algorithm/`](../../../crates/flowshield-core/src/algorithm/), so the pictures cannot drift from the code.

They are embedded in, and explained by, [`../../architecture/algorithms.md`](../../architecture/algorithms.md).

| Animation | Algorithm | Shows |
|-----------|-----------|-------|
| [`token-bucket.gif`](token-bucket.gif) | Token Bucket | full bucket → burst drains it → reject → paced refill → idle refills only to the cap |
| [`gcra.gif`](gcra.gif) | GCRA | cold-key burst → reject → even one-per-`T` pacing → idle credit capped at `τ` |

**Regenerating.** The generator is [`generate.py`](generate.py) — a faithful Python port of the two algorithms' `step` logic driving a matplotlib animation. It is a documentation tool, not a project dependency, so its packages are installed ad hoc:

```sh
python3 -m pip install matplotlib pillow
python3 docs/diagrams/animations/generate.py   # rewrites the GIFs in this directory
```

If you change a scenario, keep it in sync with the algorithm modules and the worked-example tables in [`algorithms.md`](../../architecture/algorithms.md).
