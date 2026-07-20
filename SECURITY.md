# Security Policy

## Project status

FlowShield is in **Phase 0 (design)**. There is no released code yet, and therefore no supported versions. This policy establishes the process for when releases begin.

## Supported versions

| Version | Supported |
|---------|-----------|
| unreleased (pre-1.0) | Best-effort; no guarantees |

Once FlowShield reaches its first stable release, this table will list the version lines that receive security fixes, consistent with the versioning policy in [ADR-015](docs/adr/ADR-015-versioning-and-msrv.md).

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, use **GitHub's private vulnerability reporting** ("Report a vulnerability" under the repository's *Security* tab), or contact the maintainers privately. Please include:

- a description of the issue and its impact,
- steps to reproduce (a proof of concept if possible),
- affected component(s) and version(s), and
- any suggested mitigation.

You can expect an acknowledgement within a few business days. We will coordinate a fix and a disclosure timeline with you, and credit reporters who wish to be credited.

## Scope

Security-relevant concerns for FlowShield include, among others: unsafe defaults, resource-exhaustion / denial-of-service vectors (e.g., unbounded per-key state), and configuration that silently disables protection. These are treated as first-class requirements — see the Security requirements in [`docs/design/Requirements.md`](docs/design/Requirements.md) (FR-070…FR-073) and [ADR-007](docs/adr/ADR-007-backend-degradation-policy.md).
