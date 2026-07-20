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
    pub fn is_allowed(&self) -> bool {
        matches!(self, Decision::Allowed(_))
    }
    pub fn retry_after(&self) -> Option<Duration> {
        if let Decision::Rejected(r) = self {
            Some(r.retry_after)
        } else {
            None
        }
    }
}
