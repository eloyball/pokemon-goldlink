//! Abuse protection in front of the room manager.
//!
//! - [`CookieJar`]: stateless return-routability check. A client must echo a
//!   cookie bound to its exact IP:port before it can create, join or leave a
//!   room, so spoofed source addresses can never claim a slot (and the relay
//!   never streams broadcasts at a victim). Challenges are smaller than the
//!   request that triggers them, so they cannot be used for amplification.
//! - [`FailureLimiter`]: per-IP token bucket charged only for rejected
//!   admissions, which makes brute-forcing room codes impractical while never
//!   slowing down a legitimate handshake.

use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

/// Cookies rotate every epoch; the current and previous epoch are accepted,
/// so a cookie stays valid for 60..120 seconds.
pub const COOKIE_EPOCH: Duration = Duration::from_secs(60);

/// Keyed SipHash over (ip, port, epoch). Keys are random per process, so a
/// relay restart simply invalidates every cookie and clients re-handshake.
pub struct CookieJar {
    keys: RandomState,
    started: Instant,
}

impl Default for CookieJar {
    fn default() -> Self {
        Self::new()
    }
}

impl CookieJar {
    pub fn new() -> Self {
        Self {
            keys: RandomState::new(),
            started: Instant::now(),
        }
    }

    fn epoch(&self, now: Instant) -> u64 {
        now.duration_since(self.started).as_secs() / COOKIE_EPOCH.as_secs()
    }

    fn cookie_at(&self, addr: SocketAddr, epoch: u64) -> u64 {
        // 0 means "no cookie" on the wire, so it is never issued.
        self.keys.hash_one((addr.ip(), addr.port(), epoch)).max(1)
    }

    pub fn issue(&self, addr: SocketAddr, now: Instant) -> u64 {
        self.cookie_at(addr, self.epoch(now))
    }

    pub fn verify(&self, addr: SocketAddr, cookie: u64, now: Instant) -> bool {
        if cookie == 0 {
            return false;
        }
        let epoch = self.epoch(now);
        cookie == self.cookie_at(addr, epoch)
            || (epoch > 0 && cookie == self.cookie_at(addr, epoch - 1))
    }
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Token bucket per source IP, charged one token per rejected admission.
pub struct FailureLimiter {
    buckets: HashMap<IpAddr, Bucket>,
    capacity: f64,
    refill_per_sec: f64,
}

impl FailureLimiter {
    /// `capacity` failures allowed in a burst, refilled at `refill_per_sec`.
    pub fn new(capacity: u32, refill_per_sec: f64) -> Self {
        Self {
            buckets: HashMap::new(),
            capacity: f64::from(capacity.max(1)),
            refill_per_sec,
        }
    }

    fn refill(&mut self, ip: IpAddr, now: Instant) -> &mut Bucket {
        let capacity = self.capacity;
        let refill = self.refill_per_sec;
        let bucket = self.buckets.entry(ip).or_insert(Bucket {
            tokens: capacity,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill).min(capacity);
        bucket.last = now;
        bucket
    }

    /// True while `ip` still has budget for another failed attempt.
    pub fn allows(&mut self, ip: IpAddr, now: Instant) -> bool {
        if !self.buckets.contains_key(&ip) {
            return true;
        }
        self.refill(ip, now).tokens >= 1.0
    }

    pub fn record_failure(&mut self, ip: IpAddr, now: Instant) {
        let bucket = self.refill(ip, now);
        bucket.tokens = (bucket.tokens - 1.0).max(0.0);
    }

    /// Drops buckets that have fully refilled, bounding memory use.
    pub fn prune(&mut self, now: Instant) {
        let capacity = self.capacity;
        let refill = self.refill_per_sec;
        self.buckets.retain(|_, b| {
            let elapsed = now.duration_since(b.last).as_secs_f64();
            b.tokens + elapsed * refill < capacity
        });
    }

    pub fn tracked_ips(&self) -> usize {
        self.buckets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_is_bound_to_ip_and_port() {
        let jar = CookieJar::new();
        let now = Instant::now();
        let a: SocketAddr = "10.0.0.1:5000".parse().unwrap();
        let b: SocketAddr = "10.0.0.1:5001".parse().unwrap();
        let cookie = jar.issue(a, now);
        assert!(jar.verify(a, cookie, now));
        assert!(!jar.verify(b, cookie, now));
        assert!(!jar.verify(a, 0, now));
        assert!(!jar.verify(a, cookie ^ 1, now));
    }

    #[test]
    fn cookie_survives_one_rotation_only() {
        let jar = CookieJar::new();
        let t0 = jar.started;
        let a: SocketAddr = "10.0.0.1:5000".parse().unwrap();
        let cookie = jar.issue(a, t0);
        assert!(jar.verify(a, cookie, t0 + COOKIE_EPOCH));
        assert!(!jar.verify(a, cookie, t0 + COOKIE_EPOCH * 2));
    }

    #[test]
    fn limiter_blocks_after_burst_and_refills() {
        let mut limiter = FailureLimiter::new(3, 1.0);
        let ip: IpAddr = "10.0.0.2".parse().unwrap();
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(limiter.allows(ip, t0));
            limiter.record_failure(ip, t0);
        }
        assert!(!limiter.allows(ip, t0));
        assert!(limiter.allows(ip, t0 + Duration::from_millis(1100)));
        limiter.prune(t0 + Duration::from_secs(10));
        assert_eq!(limiter.tracked_ips(), 0);
    }
}
