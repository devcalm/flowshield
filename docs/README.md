# FlowShield Documentation

This directory holds **all** FlowShield documentation. It is intentionally richer than what ships to crates.io: the repository serves contributors, maintainers, and the design record, while the published crates serve library users.

## Map

| Directory | Purpose | Audience | Ships to crates.io? |
|-----------|---------|----------|---------------------|
| [`adr/`](adr/README.md) | Architecture Decision Records — the *why* behind every significant decision. | Contributors, advanced users | No (repo-only) |
| [`architecture/`](architecture/README.md) | Narrative architecture documentation and module guides. | Contributors, users | No (repo-only) |
| [`diagrams/`](diagrams/README.md) | C4 / Mermaid diagram sources and exports. | Contributors, users | No (repo-only) |
| [`benchmarks/`](benchmarks/README.md) | Benchmark methodology, scenarios, and published results. | Contributors, users, evaluators | No (repo-only) |
| [`design/`](design/) | Founding charter, Requirements (SRS), and the phased Plan. | Contributors, maintainers | No (repo-only) |
| [`research/`](research/README.md) | Research notes, prior-art surveys, competitive analysis. | Contributors, maintainers | No (repo-only) |

## Documentation hierarchy

```
README.md  (public front door)
  └─ docs/README.md  (this index)
       ├─ design/charter.md      ← founding brief / project charter
       ├─ design/Requirements.md ← Software Requirements Specification (what & why)
       ├─ design/Plan.md         ← phased roadmap (how & when)
       ├─ adr/                   ← decisions (ADR-001 … ADR-016)
       ├─ architecture/          ← narrative architecture & diagrams
       ├─ benchmarks/            ← performance methodology & results
       └─ research/              ← notes, prior art, competitive analysis
  └─ crate API docs (docs.rs, generated from doc comments)  ← for library users
```

## Two axes for classifying a document

Every document has **two independent properties**:

1. **Audience** — *public* (anyone browsing the repo) vs *internal* (maintainer scratch, see [`.ai/`](../.ai/README.md)).
2. **Packaging** — *repo-only* vs *shipped in a published crate*.

Almost all long-form docs are **public audience, repo-only packaging**: visible to everyone in Git, but not bundled into the crate tarball (they live at the workspace root, outside every crate's package directory, so Cargo never includes them). Library users read them here or on docs.rs, not inside the downloaded crate.
