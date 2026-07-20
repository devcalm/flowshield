# Research

Durable research artifacts that inform FlowShield's design: prior-art surveys, algorithm deep-dives, storage-backend evaluations, and competitive analysis.

Unlike [`../design/`](../design/) (which holds the *committed* requirements and plan) and [`../adr/`](../adr/README.md) (which holds *decisions*), this directory holds the *investigation* that fed those decisions. Keep entries factual and dated; when a piece of research drives a decision, link it from the relevant ADR.

> Raw, unpolished notes and exploratory scratch belong in [`../../.ai/`](../../.ai/README.md), not here. Promote research here once it has lasting reference value.

Planned / candidate topics:
- Rate-limiting algorithm survey (Token Bucket, GCRA, Sliding Window, Fixed Window, Leaky Bucket, Sliding Log).
- Storage-backend evaluation (Redis, Valkey, PostgreSQL, DynamoDB, FoundationDB).
- Prior art in Rust (existing rate-limiter crates) and other ecosystems (Envoy, cloud gateways).

**Status:** to be populated as research is formalized.
