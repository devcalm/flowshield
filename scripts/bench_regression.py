#!/usr/bin/env python3
"""FlowShield benchmark regression gate (ADR-014).

Criterion writes a machine-readable ``estimates.json`` per benchmark under
``target/criterion/<id>/new/``. This script turns those into a flat, diffable
results map and compares two such maps against a documented tolerance, exiting
non-zero when a benchmark regresses beyond it.

It is intentionally dependency-free (Python stdlib only) so it runs anywhere CI
does without a virtualenv.

Two subcommands compose into a regression gate on a *single* runner (so the two
runs share hardware and the comparison is apples-to-apples):

    # on the base revision
    cargo bench -p flowshield-benchmarks
    python3 scripts/bench_regression.py extract > base.json

    # on the candidate revision
    cargo bench -p flowshield-benchmarks
    python3 scripts/bench_regression.py extract > cur.json

    # gate: fail if any benchmark's median grew by more than the tolerance
    python3 scripts/bench_regression.py compare --baseline base.json \
        --current cur.json --tolerance 1.15

The tolerance defaults to 1.15 (a 15% median regression is allowed) to absorb the
variance of shared CI runners; ADR-014 calls this a living parameter, tightened as
dedicated benchmark hardware reduces noise. Metric compared is the **median**
(robust to the outliers ADR-014 warns about), in nanoseconds.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path


def extract(criterion_dir: Path) -> dict[str, float]:
    """Collect ``{benchmark_id: median_ns}`` from a criterion output directory."""
    results: dict[str, float] = {}
    for estimates in criterion_dir.rglob("new/estimates.json"):
        # id is the path between the criterion root and the trailing ``/new``.
        rel = estimates.parent.parent.relative_to(criterion_dir)
        bench_id = rel.as_posix()
        try:
            data = json.loads(estimates.read_text())
            median = data["median"]["point_estimate"]
        except (KeyError, ValueError, OSError) as exc:  # pragma: no cover - defensive
            print(f"warning: skipping {estimates}: {exc}", file=sys.stderr)
            continue
        results[bench_id] = median
    return results


def cmd_extract(args: argparse.Namespace) -> int:
    results = extract(Path(args.criterion_dir))
    if not results:
        print(
            f"error: no benchmark results under {args.criterion_dir} "
            "(did `cargo bench` run?)",
            file=sys.stderr,
        )
        return 2
    json.dump(dict(sorted(results.items())), sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


def cmd_compare(args: argparse.Namespace) -> int:
    baseline = json.loads(Path(args.baseline).read_text())
    current = json.loads(Path(args.current).read_text())
    tolerance = args.tolerance

    regressions: list[tuple[str, float, float, float]] = []
    improvements: list[tuple[str, float, float, float]] = []
    missing: list[str] = []

    for bench_id, base_ns in sorted(baseline.items()):
        if bench_id not in current:
            missing.append(bench_id)
            continue
        cur_ns = current[bench_id]
        ratio = cur_ns / base_ns if base_ns else float("inf")
        if ratio > tolerance:
            regressions.append((bench_id, base_ns, cur_ns, ratio))
        elif ratio < 0.90:
            improvements.append((bench_id, base_ns, cur_ns, ratio))

    new_benches = sorted(set(current) - set(baseline))

    def pct(ratio: float) -> str:
        return f"{(ratio - 1.0) * 100:+.1f}%"

    for bench_id, base_ns, cur_ns, ratio in improvements:
        print(f"  improved  {bench_id}: {base_ns:.1f}ns -> {cur_ns:.1f}ns ({pct(ratio)})")
    for bench_id in new_benches:
        print(f"  new       {bench_id}: {current[bench_id]:.1f}ns (no baseline)")
    for bench_id in missing:
        print(f"  missing   {bench_id}: in baseline but absent from current run")

    if regressions:
        print(
            f"\nREGRESSION: {len(regressions)} benchmark(s) exceeded the "
            f"{(tolerance - 1.0) * 100:.0f}% tolerance:",
            file=sys.stderr,
        )
        for bench_id, base_ns, cur_ns, ratio in regressions:
            print(
                f"  {bench_id}: {base_ns:.1f}ns -> {cur_ns:.1f}ns ({pct(ratio)})",
                file=sys.stderr,
            )
        return 1

    print(f"\nOK: no benchmark regressed beyond {(tolerance - 1.0) * 100:.0f}% tolerance.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    p_extract = sub.add_parser("extract", help="flatten criterion output to a results map")
    p_extract.add_argument(
        "--criterion-dir",
        default=os.path.join("target", "criterion"),
        help="criterion output directory (default: target/criterion)",
    )
    p_extract.set_defaults(func=cmd_extract)

    p_compare = sub.add_parser("compare", help="gate current results against a baseline")
    p_compare.add_argument("--baseline", required=True, help="baseline results JSON")
    p_compare.add_argument("--current", required=True, help="current results JSON")
    p_compare.add_argument(
        "--tolerance",
        type=float,
        default=1.15,
        help="max allowed median ratio before failing (default: 1.15 = +15%%)",
    )
    p_compare.set_defaults(func=cmd_compare)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
