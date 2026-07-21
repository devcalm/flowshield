#!/usr/bin/env python3
"""Generate faithful, simulation-driven GIF animations of FlowShield's two
rate-limiting algorithms: Token Bucket and GCRA.

The animations are driven by a Python port of the *exact* admission logic in
crates/flowshield-core/src/algorithm/{token_bucket,gcra}.rs so the pictures
cannot drift from the code. Output: docs/diagrams/animations/*.gif
"""
import math
import numpy as np
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.animation import FuncAnimation, PillowWriter
from matplotlib.patches import FancyBboxPatch, Rectangle, FancyArrowPatch

OUT = "/Users/alekseytsvetaev/Documents/Rust/flowshield/docs/diagrams/animations"

# ---- GitHub-dark palette ---------------------------------------------------
BG      = "#0d1117"
PANEL   = "#161b22"
GRID    = "#30363d"
TEXT    = "#e6edf3"
MUTED   = "#8b949e"
GREEN   = "#3fb950"
RED     = "#f85149"
BLUE    = "#58a6ff"
AMBER   = "#d29922"
PURPLE  = "#bc8cff"

plt.rcParams.update({
    "figure.facecolor": BG,
    "savefig.facecolor": BG,
    "axes.facecolor": BG,
    "text.color": TEXT,
    "axes.edgecolor": GRID,
    "axes.labelcolor": TEXT,
    "xtick.color": MUTED,
    "ytick.color": MUTED,
    "font.size": 12,
    "font.family": "DejaVu Sans",
})

FPS = 20
DT = 0.05          # virtual seconds per frame
T_END = 13.0
FRAMES = int(T_END / DT)
TIMES = [i * DT for i in range(FRAMES)]


def frame_for(t):
    """Which request (if any) fires in the half-open window [t, t+DT)."""
    return None


# ===========================================================================
# TOKEN BUCKET
# ===========================================================================
def build_token_bucket():
    CAP = 5
    INTERVAL = 1.0          # one token per second
    # Request arrival schedule (cost-1 each). Story:
    #   burst of 5 drains a full bucket -> 1 reject -> paced refill ->
    #   long idle refills to the cap -> a second burst shows the cap.
    reqs = [0.20, 0.30, 0.40, 0.50, 0.60,   # burst of 5 drains the full bucket
            0.70,                            # 6th: bucket empty -> REJECT
            2.50, 3.50, 4.50,                # paced: one token has refilled each time
            9.60, 9.85, 10.10, 10.35, 10.60]  # long idle refilled to the cap -> burst again

    # ---- exact-ish token-bucket sim (continuous level; admit on whole cost)
    level = float(CAP)       # fractional fill for a smooth bar
    last = 0.0
    events = []              # (kind, x, retry)
    # precompute per-frame level + any event flash
    ri = 0
    per_frame = []
    for t in TIMES:
        # refill continuously up to now
        level = min(CAP, level + (t - last) / INTERVAL)
        last = t
        fired = None
        # fire any request whose arrival time has been reached this frame
        while ri < len(reqs) and reqs[ri] <= t + 1e-9:
            rt = reqs[ri]
            if level >= 1.0:
                level -= 1.0
                fired = ("admit", rt, 0.0)
            else:
                retry = (1.0 - level) * INTERVAL
                fired = ("reject", rt, retry)
            events.append(fired)
            ri += 1
        per_frame.append((t, level, fired))

    # ---- figure ----
    fig = plt.figure(figsize=(9, 5), dpi=120)
    ax = fig.add_axes([0.06, 0.20, 0.40, 0.64])   # bucket
    axtl = fig.add_axes([0.52, 0.20, 0.45, 0.64])  # timeline

    fig.text(0.06, 0.945, "Token Bucket", fontsize=20, fontweight="bold", color=TEXT)
    fig.text(0.06, 0.905, "capacity = 5 tokens   ·   refill = 1 token / second   ·   cost = 1",
             fontsize=11, color=MUTED)

    # bucket geometry
    ax.set_xlim(0, 1); ax.set_ylim(0, CAP + 0.4); ax.axis("off")
    bx0, bx1 = 0.30, 0.70
    # bucket outline
    ax.add_patch(Rectangle((bx0, 0), bx1 - bx0, CAP, fill=False,
                           edgecolor=GRID, lw=2.4, zorder=2))
    # capacity ticks
    for k in range(CAP + 1):
        ax.plot([bx0 - 0.03, bx0], [k, k], color=GRID, lw=1)
        ax.text(bx0 - 0.05, k, str(k), ha="right", va="center", color=MUTED, fontsize=10)

    fill_patch = Rectangle((bx0, 0), bx1 - bx0, 0, facecolor=BLUE, alpha=0.85, zorder=1)
    ax.add_patch(fill_patch)
    # token unit separators drawn dynamically
    unit_lines = [ax.plot([bx0, bx1], [k, k], color=BG, lw=1.2, zorder=3)[0]
                  for k in range(1, CAP)]
    tokens_txt = ax.text(0.5, CAP + 0.24, "", ha="center", va="bottom",
                         color=TEXT, fontsize=13, fontweight="bold")
    ax.annotate("", xy=(0.86, CAP - 0.15), xytext=(0.86, CAP + 0.30),
                arrowprops=dict(arrowstyle="-|>", color=AMBER, lw=2))
    ax.text(0.5, -0.30, "+1 token / sec", ha="center", va="top",
            color=AMBER, fontsize=10)

    # timeline of decisions
    axtl.set_xlim(0, T_END); axtl.set_ylim(-1.2, 1.2)
    axtl.set_yticks([]); axtl.set_xlabel("time (s)")
    for s in ("left", "right", "top"):
        axtl.spines[s].set_visible(False)
    axtl.spines["bottom"].set_color(GRID)
    axtl.axhline(0, color=GRID, lw=1)
    axtl.text(0, 1.05, "decisions", color=MUTED, fontsize=10, va="bottom")
    playhead = axtl.axvline(0, color=TEXT, lw=1.4, alpha=0.8)
    verdict_txt = fig.text(0.52, 0.035, "", fontsize=12, color=MUTED)

    drawn = {"n": 0}

    def update(i):
        t, level, fired = per_frame[i]
        # bucket fill
        fill_patch.set_height(level)
        whole = int(math.floor(level + 1e-9))
        tokens_txt.set_text(f"{whole}  token{'s' if whole != 1 else ''} available")
        # drop arrow pulse when a token refills (level crosses an integer)
        # decisions timeline: draw markers up to current time
        while drawn["n"] < len(events) and events[drawn["n"]][1] <= t + 1e-9:
            kind, rt, retry = events[drawn["n"]]
            if kind == "admit":
                axtl.annotate("", xy=(rt, 0.72), xytext=(rt, 0.0),
                              arrowprops=dict(arrowstyle="-|>", color=GREEN, lw=2.2))
                axtl.plot(rt, 0.72, marker="o", color=GREEN, ms=4)
            else:
                axtl.plot([rt, rt], [0, -0.72], color=RED, lw=2.2)
                axtl.plot(rt, -0.72, marker="x", color=RED, ms=8, mew=2.5)
                axtl.annotate(f"retry\n{retry:.1f}s", xy=(rt, -0.78),
                              ha="center", va="top", color=RED, fontsize=8)
            drawn["n"] += 1
        playhead.set_xdata([t, t])
        # verdict flash
        if fired is not None:
            kind, rt, retry = fired
            if kind == "admit":
                verdict_txt.set_text(f"t={rt:4.2f}s   ALLOW   →  {int(level)} left")
                verdict_txt.set_color(GREEN)
            else:
                verdict_txt.set_text(f"t={rt:4.2f}s   REJECT  →  empty, retry in {retry:.1f}s")
                verdict_txt.set_color(RED)
        return ()

    anim = FuncAnimation(fig, update, frames=FRAMES, interval=1000 / FPS, blit=False)
    anim.save(f"{OUT}/token-bucket.gif", writer=PillowWriter(fps=FPS))
    plt.close(fig)
    print("wrote token-bucket.gif")


# ===========================================================================
# GCRA
# ===========================================================================
def build_gcra():
    RATE = 1.0
    BURST = 3
    T = 1.0 / RATE            # emission interval
    TAU = (BURST - 1) * T     # burst tolerance
    reqs = [0.30, 0.55, 0.80, 1.05,          # burst of 3 then a reject
            1.60, 2.60, 3.60,                 # paced admits, one per T
            9.60, 9.85, 10.10, 10.35, 10.60]  # idle then burst -> only 3 admit

    # ---- exact GCRA sim (mirrors gcra.rs::step, cost = 1) ----
    tat = 0.0                 # initial(now=0)
    events = []               # (t, admit, retry, tat_after, ahead_before)
    per_frame = []
    ri = 0
    for t in TIMES:
        fired = None
        while ri < len(reqs) and reqs[ri] <= t + 1e-9:
            rt = reqs[ri]
            earliest = tat - TAU          # (cost-1)*T = 0 for cost 1
            if t < earliest:
                retry = earliest - t
                fired = ("reject", rt, retry, tat)
                events.append(fired)
            else:
                tat = max(tat, t) + T
                fired = ("admit", rt, 0.0, tat)
                events.append(fired)
            ri += 1
        # "ahead": how far the schedule runs past now (the mirror of tokens)
        ahead = max(0.0, tat - t)
        per_frame.append((t, tat, ahead, fired))

    fig = plt.figure(figsize=(9, 5), dpi=120)
    fig.text(0.06, 0.945, "GCRA", fontsize=20, fontweight="bold", color=TEXT)
    fig.text(0.06, 0.905,
             "rate = 1 req/s  (T = 1s)   ·   burst = 3   ·   tolerance  τ = (burst−1)·T = 2s",
             fontsize=11, color=MUTED)

    # gauge: schedule-ahead vs tau (the mirror image of the token bucket)
    axg = fig.add_axes([0.06, 0.20, 0.40, 0.64])
    axg.set_xlim(0, 1); axg.set_ylim(0, TAU + 0.6); axg.axis("off")
    gx0, gx1 = 0.30, 0.70
    axg.add_patch(Rectangle((gx0, 0), gx1 - gx0, TAU, fill=False,
                            edgecolor=GRID, lw=2.4))
    axg.axhline  # noqa
    # tau limit line
    axg.plot([gx0 - 0.04, gx1 + 0.04], [TAU, TAU], color=RED, lw=1.6, ls="--")
    axg.text(gx1 + 0.06, TAU, "τ  (reject above)", color=RED, va="center", fontsize=9)
    for k in range(BURST):
        axg.plot([gx0 - 0.03, gx0], [k, k], color=GRID, lw=1)
        axg.text(gx0 - 0.05, k, f"{k}·T", ha="right", va="center", color=MUTED, fontsize=9)
    ahead_patch = Rectangle((gx0, 0), gx1 - gx0, 0, facecolor=PURPLE, alpha=0.85)
    axg.add_patch(ahead_patch)
    step_lines = [axg.plot([gx0, gx1], [k, k], color=BG, lw=1.2)[0] for k in range(1, BURST)]
    ahead_txt = axg.text(0.5, TAU + 0.35, "", ha="center", va="bottom",
                         color=TEXT, fontsize=12, fontweight="bold")
    axg.text(0.5, -0.32, "schedule ahead of now  (tat − now)", ha="center", va="top",
             color=MUTED, fontsize=9)

    # timeline: pacing of admitted requests
    axtl = fig.add_axes([0.52, 0.20, 0.45, 0.64])
    axtl.set_xlim(0, T_END); axtl.set_ylim(-1.2, 1.4)
    axtl.set_yticks([]); axtl.set_xlabel("time (s)")
    for s in ("left", "right", "top"):
        axtl.spines[s].set_visible(False)
    axtl.spines["bottom"].set_color(GRID)
    axtl.axhline(0, color=GRID, lw=1)
    axtl.text(0, 1.15, "admitted requests are paced ≥ T apart", color=MUTED, fontsize=9, va="bottom")
    playhead = axtl.axvline(0, color=TEXT, lw=1.4, alpha=0.8)
    verdict_txt = fig.text(0.52, 0.035, "", fontsize=12, color=MUTED)

    drawn = {"n": 0}
    last_admit_x = {"v": None}

    def update(i):
        t, tat, ahead, fired = per_frame[i]
        ahead_patch.set_height(min(ahead, TAU + 0.55))
        # color the gauge red when saturated (next request would reject)
        ahead_patch.set_facecolor(RED if ahead > TAU + 1e-9 else PURPLE)
        ahead_txt.set_text(f"{ahead:.2f}s ahead   ·   τ = {TAU:.0f}s")
        while drawn["n"] < len(events) and events[drawn["n"]][1] <= t + 1e-9:
            kind, rt, retry, tat_after = events[drawn["n"]]
            if kind == "admit":
                axtl.annotate("", xy=(rt, 0.72), xytext=(rt, 0.0),
                              arrowprops=dict(arrowstyle="-|>", color=GREEN, lw=2.2))
                axtl.plot(rt, 0.72, marker="o", color=GREEN, ms=4)
                if last_admit_x["v"] is not None:
                    gap = rt - last_admit_x["v"]
                    # only annotate the paced gaps; intra-burst ticks are too close
                    if gap >= 0.5:
                        axtl.annotate("", xy=(rt, 0.95), xytext=(last_admit_x["v"], 0.95),
                                      arrowprops=dict(arrowstyle="<->", color=MUTED, lw=1))
                        axtl.text((rt + last_admit_x["v"]) / 2, 1.0, f"{gap:.1f}s",
                                  ha="center", va="bottom", color=MUTED, fontsize=8)
                last_admit_x["v"] = rt
            else:
                axtl.plot([rt, rt], [0, -0.72], color=RED, lw=2.2)
                axtl.plot(rt, -0.72, marker="x", color=RED, ms=8, mew=2.5)
                axtl.annotate(f"retry\n{retry:.1f}s", xy=(rt, -0.78),
                              ha="center", va="top", color=RED, fontsize=8)
            drawn["n"] += 1
        playhead.set_xdata([t, t])
        if fired is not None:
            kind, rt, retry, tat_after = fired
            if kind == "admit":
                verdict_txt.set_text(f"t={rt:4.2f}s   CONFORM   →  tat advances to {tat_after:.1f}s")
                verdict_txt.set_color(GREEN)
            else:
                verdict_txt.set_text(f"t={rt:4.2f}s   REJECT    →  too early, retry in {retry:.1f}s")
                verdict_txt.set_color(RED)
        return ()

    anim = FuncAnimation(fig, update, frames=FRAMES, interval=1000 / FPS, blit=False)
    anim.save(f"{OUT}/gcra.gif", writer=PillowWriter(fps=FPS))
    plt.close(fig)
    print("wrote gcra.gif")


if __name__ == "__main__":
    build_token_bucket()
    build_gcra()
    print("done")
