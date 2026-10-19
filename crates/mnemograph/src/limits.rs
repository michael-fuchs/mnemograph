//! Hard limits on agent writes, against runaway agents.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mnemograph_core::Author;

const WINDOW: Duration = Duration::from_secs(60);

/// Configurable limits on what agents may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteLimits {
    /// Largest page body, in bytes.
    pub max_page_bytes: usize,
    /// Largest note, in bytes.
    pub max_note_bytes: usize,
    /// Writes (write_page and add_note calls) each agent may make per minute.
    pub writes_per_minute: u32,
}

impl Default for WriteLimits {
    fn default() -> Self {
        Self {
            max_page_bytes: 200 * 1024,
            max_note_bytes: 4 * 1024,
            writes_per_minute: 60,
        }
    }
}

/// Enforces [`WriteLimits`]; shared by all requests.
pub struct Limiter {
    limits: WriteLimits,
    /// Times of each author's writes within the last minute, oldest first.
    recent_writes: Mutex<HashMap<Author, VecDeque<Instant>>>,
}

impl Limiter {
    pub fn new(limits: WriteLimits) -> Self {
        Self {
            limits,
            recent_writes: Mutex::default(),
        }
    }

    pub fn check_page(&self, body: &str) -> Result<(), String> {
        check_size("page body", body, self.limits.max_page_bytes)
    }

    pub fn check_note(&self, body: &str) -> Result<(), String> {
        check_size("note", body, self.limits.max_note_bytes)
    }

    /// Counts a write by `author`, or rejects it if the author already made the allowed number
    /// of writes within the last minute.
    pub fn count_write(&self, author: &Author) -> Result<(), String> {
        self.count_write_at(author, Instant::now())
    }

    fn count_write_at(&self, author: &Author, now: Instant) -> Result<(), String> {
        let mut recent_writes = self
            .recent_writes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Forget authors without writes in the window, so the map does not grow forever.
        recent_writes.retain(|_, times| {
            while times
                .front()
                .is_some_and(|t| now.duration_since(*t) >= WINDOW)
            {
                times.pop_front();
            }
            !times.is_empty()
        });
        let times = recent_writes.entry(author.clone()).or_default();
        if times.len() >= self.limits.writes_per_minute as usize {
            let wait = WINDOW.saturating_sub(now.duration_since(times[0]));
            return Err(format!(
                "write limit reached: at most {} writes per minute per agent; try again in {} seconds",
                self.limits.writes_per_minute,
                wait.as_secs().max(1)
            ));
        }
        times.push_back(now);
        Ok(())
    }
}

fn check_size(what: &str, text: &str, max_bytes: usize) -> Result<(), String> {
    if text.len() > max_bytes {
        Err(format!(
            "the {what} is {} bytes; the limit is {max_bytes} bytes. Split it up or shorten it.",
            text.len()
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(writes_per_minute: u32) -> Limiter {
        Limiter::new(WriteLimits {
            max_page_bytes: 10,
            max_note_bytes: 5,
            writes_per_minute,
        })
    }

    #[test]
    fn sizes_are_limited_in_bytes() {
        let limiter = limiter(1);
        assert!(limiter.check_page("0123456789").is_ok());
        let err = limiter.check_page("01234567890").unwrap_err();
        assert!(
            err.contains("page body is 11 bytes; the limit is 10 bytes"),
            "{err}"
        );
        assert!(
            limiter.check_note("äää").is_err(),
            "umlauts are two bytes each"
        );
    }

    #[test]
    fn writes_are_limited_per_agent_within_a_sliding_minute() {
        let limiter = limiter(2);
        let a = Author::Agent("a".into());
        let b = Author::Agent("b".into());
        let start = Instant::now();

        assert!(limiter.count_write_at(&a, start).is_ok());
        assert!(
            limiter
                .count_write_at(&a, start + Duration::from_secs(10))
                .is_ok()
        );
        let err = limiter
            .count_write_at(&a, start + Duration::from_secs(20))
            .unwrap_err();
        assert!(err.contains("try again in 40 seconds"), "{err}");
        assert!(
            limiter
                .count_write_at(&b, start + Duration::from_secs(20))
                .is_ok(),
            "other agents are not affected"
        );
        assert!(
            limiter
                .count_write_at(&a, start + Duration::from_secs(61))
                .is_ok(),
            "the first write left the window"
        );
    }
}
