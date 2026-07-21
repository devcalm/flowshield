//! A bounded, LRU-ordered map with lazy TTL expiry — one shard of the store.
//!
//! This is the data structure that makes the memory backend *bounded* (FR-071,
//! R-04): it never holds more than `cap` live entries, and when a fresh key would
//! push it over that limit the **least-recently-used** entry is evicted. LRU (not
//! FIFO or random) is a deliberate correctness choice for a rate limiter: evicting
//! a key that is *actively being limited* would reset its state to fresh — i.e.
//! hand it a full allowance — briefly defeating the limit. LRU keeps the keys that
//! are currently seeing traffic and sheds the cold ones, so eviction only ever
//! forgets keys that are not being limited anyway.
//!
//! # Design
//!
//! A classic intrusive doubly-linked list laid over a slab:
//!
//! - `nodes` is a slab (`Vec<Option<Node>>`); a node's identity is its index.
//! - `index` maps each [`Key`] to its slab index for O(1) lookup.
//! - `prev`/`next` are slab indices ([`NIL`] as the sentinel), threading every
//!   live node from `head` (most-recently-used) to `tail` (least-recently-used).
//! - `free` recycles slab slots so the slab stabilises at the working-set size and
//!   never grows without bound under key churn.
//!
//! Every operation is O(1). The shard is **not** thread-safe on its own; the store
//! wraps each shard in a `Mutex`, which is where the ADR-005 atomic
//! check-and-consume is enforced.

use flowshield_core::key::Key;
use std::collections::HashMap;
use std::time::Duration;

/// Sentinel "no node" index for the intrusive links. A shard can therefore hold up
/// to `u32::MAX - 1` entries — far beyond any sane bound.
const NIL: u32 = u32::MAX;

struct Node<S> {
    key: Key,
    state: S,
    /// Absolute expiry on the injected monotonic clock (ADR-017): `now + ttl` at
    /// the time of insertion. Compared against `now`, never against wall-clock.
    expires_at: Duration,
    prev: u32,
    next: u32,
}

/// One shard: a capacity-bounded, LRU-ordered, lazily-expiring key→state map.
pub(crate) struct LruShard<S> {
    nodes: Vec<Option<Node<S>>>,
    free: Vec<u32>,
    index: HashMap<Key, u32>,
    head: u32, // most-recently-used
    tail: u32, // least-recently-used (evicted first)
    cap: usize,
}

impl<S> LruShard<S> {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        // Internal invariant, enforced in every build (not just debug): a shard that
        // can hold nothing would insert-then-immediately-evict every key, so each key
        // would read back absent and be treated as fresh — a silent fail-*open* of the
        // limit. This is defence-in-depth behind the builder, which already rejects a
        // zero total capacity with a typed `Error::Config`; reaching here with `0` is a
        // FlowShield bug, so we fail loudly rather than under-limit silently.
        assert!(cap >= 1, "shard capacity must be at least 1");
        Self {
            nodes: Vec::new(),
            free: Vec::new(),
            index: HashMap::new(),
            head: NIL,
            tail: NIL,
            cap,
        }
    }

    /// Number of live entries currently held.
    pub(crate) fn len(&self) -> usize {
        self.index.len()
    }

    fn node(&self, i: u32) -> &Node<S> {
        self.nodes[i as usize]
            .as_ref()
            .expect("live index must point at an occupied slab slot")
    }

    fn node_mut(&mut self, i: u32) -> &mut Node<S> {
        self.nodes[i as usize]
            .as_mut()
            .expect("live index must point at an occupied slab slot")
    }

    /// Detach node `i` from the list, mending its neighbours and the head/tail ends.
    ///
    /// `i`'s *own* `prev`/`next` are deliberately left stale. The sole caller
    /// ([`remove_key`](Self::remove_key)) `take()`s the node out of the slab on the
    /// next line, so its slot becomes `None` and any erroneous later access panics
    /// loudly through [`node`](Self::node) rather than silently following a dangling
    /// index; a re-link would in any case go through [`push_front`](Self::push_front),
    /// which overwrites both fields. Clearing them here would be dead writes.
    fn unlink(&mut self, i: u32) {
        let (prev, next) = {
            let n = self.node(i);
            (n.prev, n.next)
        };
        if prev != NIL {
            self.node_mut(prev).next = next;
        } else {
            self.head = next; // `i` was the head
        }
        if next != NIL {
            self.node_mut(next).prev = prev;
        } else {
            self.tail = prev; // `i` was the tail
        }
    }

    /// Link node `i` in as the new head (most-recently-used).
    fn push_front(&mut self, i: u32) {
        let old_head = self.head;
        {
            let n = self.node_mut(i);
            n.prev = NIL;
            n.next = old_head;
        }
        if old_head != NIL {
            self.node_mut(old_head).prev = i;
        } else {
            self.tail = i; // list was empty; `i` is both head and tail
        }
        self.head = i;
    }

    /// Remove `key` entirely, returning its node if it was present — *regardless of
    /// expiry*. Used both to reclaim expired entries and to evict the LRU tail.
    fn remove_key(&mut self, key: &Key) -> Option<Node<S>> {
        let i = self.index.remove(key)?;
        self.unlink(i);
        let node = self.nodes[i as usize]
            .take()
            .expect("indexed slot must be occupied");
        self.free.push(i);
        Some(node)
    }

    /// Take the owned state for `key`, **always removing the entry from the map** —
    /// whether it was present, absent, or expired. This unconditional removal is
    /// intentional and is one half of the store's update cycle: the caller applies
    /// the algorithm's `step` to the returned `Option<S>` and writes the result back
    /// with [`insert_front`](Self::insert_front), which reinserts the key at the MRU
    /// front. That remove-then-reinsert *is* how LRU order is maintained on every
    /// touch — do not mistake the always-remove behaviour for a bug.
    ///
    /// The state is returned only if the entry was present **and** unexpired; an
    /// expired entry is reclaimed and reported as `None`, so an expired key behaves
    /// exactly like a fresh one (ADR-005).
    pub(crate) fn take_current(&mut self, key: &Key, now: Duration) -> Option<S> {
        match self.remove_key(key) {
            Some(node) if node.expires_at > now => Some(node.state),
            _ => None, // absent, or expired and now reclaimed
        }
    }

    /// Insert `key` at the MRU front with `state` and `expires_at`, evicting the LRU
    /// entry if this pushes the shard over `cap`. The caller must have removed any
    /// prior entry for `key` first (via [`take_current`](Self::take_current)), so
    /// this never duplicates a key.
    pub(crate) fn insert_front(&mut self, key: Key, state: S, expires_at: Duration) {
        debug_assert!(
            !self.index.contains_key(&key),
            "insert_front requires the key to be absent (take_current first)"
        );
        let node = Node {
            key: key.clone(),
            state,
            expires_at,
            prev: NIL,
            next: NIL,
        };
        let i = match self.free.pop() {
            Some(i) => {
                self.nodes[i as usize] = Some(node);
                i
            }
            None => {
                let i = self.nodes.len() as u32;
                assert_ne!(i, NIL, "shard slab exhausted the index space");
                self.nodes.push(Some(node));
                i
            }
        };
        self.index.insert(key, i);
        self.push_front(i);

        if self.index.len() > self.cap {
            self.evict_lru();
        }
    }

    /// Drop the least-recently-used entry (the tail). Called only when over capacity,
    /// so the tail is guaranteed to exist and is never the just-inserted head.
    ///
    /// This is also what makes the *lazy* expiry strategy near-optimal, and why no
    /// separate expired-entry sweep is needed. Under the store's usage every access
    /// reinserts with `expires_at = now + ttl` for a single fixed `ttl`, so an
    /// entry's expiry time is a monotonic function of its last-access time — expiry
    /// order and LRU order coincide, and expired entries always collect at the tail.
    /// Evicting the tail therefore sheds the most-expired entry first; a still-live
    /// entry is only ever evicted when nothing in the shard is expired — exactly the
    /// intended "evict the coldest key" behaviour.
    fn evict_lru(&mut self) {
        if self.tail != NIL {
            let victim = self.node(self.tail).key.clone();
            self.remove_key(&victim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    // Emulate the store's per-key transition: take the current state, apply `f`,
    // write the result back at the MRU front.
    fn upsert<S, F: FnOnce(Option<S>) -> S>(
        shard: &mut LruShard<S>,
        key: &Key,
        now: Duration,
        ttl: Duration,
        f: F,
    ) {
        let cur = shard.take_current(key, now);
        let next = f(cur);
        shard.insert_front(key.clone(), next, now + ttl);
    }

    #[test]
    fn reloads_stored_state_within_ttl() {
        let mut s = LruShard::with_capacity(4);
        let k = Key::from("k");
        upsert(&mut s, &k, secs(0), secs(10), |prev| {
            assert_eq!(prev, None);
            1u32
        });
        upsert(&mut s, &k, secs(1), secs(10), |prev| {
            assert_eq!(prev, Some(1));
            2
        });
        assert_eq!(s.len(), 1, "the same key must not create a second entry");
    }

    #[test]
    fn expired_entry_presents_as_absent_and_is_reclaimed() {
        let mut s = LruShard::with_capacity(4);
        let k = Key::from("k");
        upsert(&mut s, &k, secs(0), secs(10), |_| 7u32);
        // Past the TTL: the prior state must be invisible.
        upsert(&mut s, &k, secs(20), secs(10), |prev| {
            assert_eq!(prev, None, "expired state must look absent");
            0
        });
    }

    #[test]
    fn evicts_least_recently_used_when_full() {
        let mut s = LruShard::with_capacity(2);
        let (a, b, c) = (Key::from("a"), Key::from("b"), Key::from("c"));
        upsert(&mut s, &a, secs(0), secs(60), |_| 1u32); // [a]
        upsert(&mut s, &b, secs(0), secs(60), |_| 1); // [b, a]
        upsert(&mut s, &a, secs(0), secs(60), |_| 2); // touch a -> [a, b]
        upsert(&mut s, &c, secs(0), secs(60), |_| 1); // insert c -> evict b -> [c, a]

        assert_eq!(s.len(), 2, "capacity must be enforced");
        // `b` was least-recently-used and must be gone; `a` (recently touched) stays.
        assert_eq!(
            s.take_current(&b, secs(0)),
            None,
            "LRU victim should be evicted"
        );
        assert_eq!(
            s.take_current(&a, secs(0)),
            Some(2),
            "recently-used key must survive eviction"
        );
    }

    #[test]
    fn capacity_one_evicts_previous_on_insert() {
        let mut s = LruShard::with_capacity(1);
        let (a, b) = (Key::from("a"), Key::from("b"));
        s.insert_front(a.clone(), 1u32, secs(60)); // [a]
        s.insert_front(b.clone(), 2, secs(60)); // insert b -> evict a -> [b]

        assert_eq!(s.len(), 1, "capacity of one holds exactly one entry");
        assert_eq!(s.take_current(&a, secs(0)), None, "A must be evicted");
        assert_eq!(s.take_current(&b, secs(0)), Some(2), "B must remain");
    }

    #[test]
    fn repeated_access_protects_the_hot_key() {
        let mut s = LruShard::with_capacity(3);
        let (a, b, c, d) = (
            Key::from("a"),
            Key::from("b"),
            Key::from("c"),
            Key::from("d"),
        );
        upsert(&mut s, &a, secs(0), secs(60), |_| 1u32); // MRU→LRU: [a]
        upsert(&mut s, &b, secs(0), secs(60), |_| 1); // [b, a]
        upsert(&mut s, &c, secs(0), secs(60), |_| 1); // [c, b, a]  (tail = a)

        // Touch `a` three times: each touch moves it to the MRU front, giving
        // [a, c, b]. The least-recently-used entry is now `b` (untouched since insert).
        for _ in 0..3 {
            upsert(&mut s, &a, secs(0), secs(60), |p| p.unwrap());
        }

        upsert(&mut s, &d, secs(0), secs(60), |_| 1); // insert d -> evict LRU

        assert_eq!(s.len(), 3, "capacity must be enforced");
        assert_eq!(
            s.take_current(&b, secs(0)),
            None,
            "the least-recently-used key B must be evicted, not the hot key A"
        );
        assert!(s.take_current(&a, secs(0)).is_some(), "hot key A survives");
        assert!(s.take_current(&c, secs(0)).is_some(), "C survives");
    }

    #[test]
    fn empty_shard_transitions_keep_head_and_tail_consistent() {
        let mut s: LruShard<u32> = LruShard::with_capacity(4);

        // Operations on an empty shard must be no-ops leaving head/tail unset.
        assert!(s.remove_key(&Key::from("absent")).is_none());
        assert_eq!(s.take_current(&Key::from("absent"), secs(0)), None);
        assert_eq!(s.head, NIL);
        assert_eq!(s.tail, NIL);
        assert_eq!(s.len(), 0);

        // Inserting the first element makes it both head and tail.
        let k = Key::from("k");
        s.insert_front(k.clone(), 1u32, secs(60));
        assert_ne!(s.head, NIL);
        assert_eq!(s.head, s.tail, "a single element is both head and tail");
        assert_eq!(s.len(), 1);

        // Removing the last element must reset both ends back to NIL.
        assert!(s.remove_key(&k).is_some());
        assert_eq!(s.head, NIL, "head reset after removing the last element");
        assert_eq!(s.tail, NIL, "tail reset after removing the last element");
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn never_exceeds_capacity_under_churn() {
        let mut s = LruShard::with_capacity(8);
        for i in 0..10_000u64 {
            let k = Key::from(format!("k-{i}"));
            upsert(&mut s, &k, secs(0), secs(60), |_| i as u32);
            assert!(s.len() <= 8, "shard grew past its bound at i={i}");
        }
        // The slab recycles slots rather than growing unboundedly with churn.
        assert!(
            s.nodes.len() <= 9,
            "slab should stabilise near capacity, was {}",
            s.nodes.len()
        );
    }
}
