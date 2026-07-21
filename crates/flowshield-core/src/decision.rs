use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Decision {
    Allowed(Allowed),
    Rejected(Rejected),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Allowed {
    pub remaining: Option<u64>, // Option: some algos can't report it
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rejected {
    pub retry_after: Duration, // RELATIVE, never absolute (ADR-017 #4)
}

impl Decision {
    /// Construct an "allowed" decision, optionally reporting how many further
    /// requests are permitted right now (`None` when the algorithm cannot cheaply
    /// say). This is the constructor out-of-crate [`Algorithm`] implementors use to
    /// produce a decision, since [`Allowed`] is `#[non_exhaustive]` and cannot be
    /// built with a struct literal from another crate (NFR-020, ADR-013).
    ///
    /// [`Algorithm`]: crate::algorithm::Algorithm
    pub fn allowed(remaining: Option<u64>) -> Self {
        Decision::Allowed(Allowed { remaining })
    }

    /// Construct a "rejected" decision carrying the **relative** wait until the
    /// request could succeed (never an absolute timestamp — ADR-017 #4). The
    /// constructor counterpart to [`allowed`](Self::allowed) for out-of-crate
    /// extension authors.
    pub fn rejected(retry_after: Duration) -> Self {
        Decision::Rejected(Rejected { retry_after })
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, Decision::Allowed(_))
    }

    /// How many further requests are permitted **right now**, when the decision is
    /// [`Allowed`] and the algorithm can cheaply report it. `None` for a rejection,
    /// and `None` when the algorithm (or a degradation fallback) cannot say. This is
    /// what HTTP integrations surface as `X-RateLimit-Remaining`.
    pub fn remaining(&self) -> Option<u64> {
        match self {
            Decision::Allowed(a) => a.remaining,
            Decision::Rejected(_) => None,
        }
    }

    pub fn retry_after(&self) -> Option<Duration> {
        if let Decision::Rejected(r) = self {
            Some(r.retry_after)
        } else {
            None
        }
    }
}
