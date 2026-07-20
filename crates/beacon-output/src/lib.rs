//! Deciding what the player actually hears.
//!
//! Plugins **propose** utterances; they never speak. Everything that decides
//! what survives lives here, so that behaviour is consistent across games and
//! is not reimplemented badly by each plugin.
//!
//! This exists because detection was never the hard part. The proof of concept
//! detected plenty and arbitrated almost nothing, which made it an auditory
//! mess: every lift in a room announced itself, every frame, forever. A tool
//! that says everything is as unusable as one that says nothing.
//!
//! # Determinism
//!
//! Nothing here reads the clock. Callers pass the current time in, which is
//! what allows a recorded session to be replayed frame by frame and asserted
//! against a fixture. See `docs/decisions/0012-determinism-and-replay.md`.

use std::collections::HashMap;
use std::time::Duration;

pub mod sink;

/// How urgent an utterance is. Higher classes interrupt lower ones.
///
/// The ordering is the whole point: `Critical` must be able to cut off an
/// ambient scenery description mid-word, because by the time the sentence
/// finishes the player is already dead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Scenery, cone scans, flavour. First thing to go when it gets busy.
    Ambient = 0,
    /// Facing a chest, an NPC in soft-target range.
    Interaction = 1,
    /// Arrival, zone entry, blocked by an obstacle.
    Navigation = 2,
    /// Incoming attack, death, low health. Barges in.
    Critical = 3,
}

impl Priority {
    /// The lowest verbosity level at which this class is still spoken.
    fn min_verbosity(self) -> u8 {
        match self {
            Priority::Critical => 0,
            Priority::Navigation => 1,
            Priority::Interaction => 2,
            Priority::Ambient => 3,
        }
    }
}

/// A proposed utterance. Plugins emit these; the [`Arbiter`] decides.
#[derive(Debug, Clone)]
pub struct Intent {
    pub text: String,
    pub priority: Priority,
    /// Rate limiting bucket. Intents sharing a category compete for the same
    /// budget, so a chatty subsystem cannot crowd out a quiet one.
    pub category: String,
    /// Intents sharing a collapse key in one frame reduce to the single
    /// nearest instance. This is the fix for twelve floor triggers announcing
    /// themselves at once: report the closest, ignore the rest.
    pub collapse_key: Option<String>,
    /// Used to pick the winner when collapsing. Absent sorts last.
    pub distance: Option<f32>,
    /// Suppress identical text for this long. Absent means no suppression.
    pub dedup_for: Option<Duration>,
}

impl Intent {
    pub fn new(text: impl Into<String>, priority: Priority, category: impl Into<String>) -> Self {
        Intent {
            text: text.into(),
            priority,
            category: category.into(),
            collapse_key: None,
            distance: None,
            dedup_for: None,
        }
    }

    pub fn collapse(mut self, key: impl Into<String>, distance: f32) -> Self {
        self.collapse_key = Some(key.into());
        self.distance = Some(distance);
        self
    }

    pub fn dedup_for(mut self, window: Duration) -> Self {
        self.dedup_for = Some(window);
        self
    }
}

/// An utterance that survived arbitration and should be spoken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utterance {
    pub text: String,
    pub priority: Priority,
    /// Whether this should cut off whatever is currently speaking.
    pub interrupt: bool,
}

/// Why an intent was dropped. Recorded so that "why did it not say that?" is
/// answerable, which during tuning matters as much as the output itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    Verbosity,
    Collapsed,
    Duplicate,
    RateLimited,
    OverBudget,
}

/// Per-category budget. Spending is silent rather than queued: stale spatial
/// information is worse than none, because the player has already moved.
#[derive(Debug, Clone)]
struct Bucket {
    tokens: f32,
    capacity: f32,
    refill_per_sec: f32,
    last_refill: Duration,
}

impl Bucket {
    fn new(capacity: f32, refill_per_sec: f32, now: Duration) -> Self {
        Bucket {
            tokens: capacity,
            capacity,
            refill_per_sec,
            last_refill: now,
        }
    }

    fn try_spend(&mut self, now: Duration) -> bool {
        let elapsed = now.saturating_sub(self.last_refill).as_secs_f32();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last_refill = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Tunable limits. Defaults are a starting point for community tuning, not
/// claims about what is correct.
#[derive(Debug, Clone)]
pub struct Config {
    /// 0 = critical only, 3 = everything.
    pub verbosity: u8,
    /// Utterances allowed through per frame, after everything else.
    pub max_per_frame: usize,
    /// Burst size for a category's rate limit.
    pub bucket_capacity: f32,
    /// Sustained rate per category, utterances per second.
    pub bucket_refill_per_sec: f32,
}

impl From<&beacon_config::ArbiterSettings> for Config {
    fn from(s: &beacon_config::ArbiterSettings) -> Self {
        Config {
            verbosity: s.verbosity,
            max_per_frame: s.max_per_frame,
            bucket_capacity: s.bucket_capacity,
            bucket_refill_per_sec: s.bucket_refill_per_sec,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            verbosity: 2,
            max_per_frame: 2,
            bucket_capacity: 3.0,
            bucket_refill_per_sec: 1.5,
        }
    }
}

/// Decides what is spoken.
///
/// Call [`resolve`] once per frame with everything the plugins proposed.
///
/// [`resolve`]: Arbiter::resolve
pub struct Arbiter {
    config: Config,
    buckets: HashMap<String, Bucket>,
    /// text -> time after which it may be said again.
    spoken_until: HashMap<String, Duration>,
    last_drops: Vec<(String, Dropped)>,
}

impl Arbiter {
    pub fn new(config: Config) -> Self {
        Arbiter {
            config,
            buckets: HashMap::new(),
            spoken_until: HashMap::new(),
            last_drops: Vec::new(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Adjusts verbosity mid-game. Bound to a hotkey: tolerance for chatter
    /// varies enormously between players, and between a first playthrough and
    /// a tenth.
    pub fn set_verbosity(&mut self, level: u8) {
        self.config.verbosity = level.min(3);
    }

    /// What was dropped during the last [`resolve`], and why.
    ///
    /// [`resolve`]: Arbiter::resolve
    pub fn last_drops(&self) -> &[(String, Dropped)] {
        &self.last_drops
    }

    /// Reduces a frame's proposed intents to what should actually be spoken.
    ///
    /// `now` is elapsed time since session start, supplied by the caller so
    /// that replays are deterministic.
    pub fn resolve(&mut self, intents: Vec<Intent>, now: Duration) -> Vec<Utterance> {
        self.last_drops.clear();

        // 1. Verbosity gate. Cheapest filter, so it runs first.
        let mut surviving: Vec<Intent> = Vec::with_capacity(intents.len());
        for intent in intents {
            if self.config.verbosity < intent.priority.min_verbosity() {
                self.last_drops.push((intent.text, Dropped::Verbosity));
            } else {
                surviving.push(intent);
            }
        }

        // 2. Nearest-only collapse. Twelve triggers become one.
        surviving = self.collapse(surviving);

        // 3. Drop anything said too recently to bear repeating.
        surviving.retain(|intent| {
            let fresh = self
                .spoken_until
                .get(&intent.text)
                .is_none_or(|until| now >= *until);
            if !fresh {
                self.last_drops
                    .push((intent.text.clone(), Dropped::Duplicate));
            }
            fresh
        });

        // 4. Highest priority first, then nearest, so the budget below is
        //    spent on what matters rather than on arrival order.
        surviving.sort_by(|a, b| {
            b.priority.cmp(&a.priority).then_with(|| {
                let (da, db) = (
                    a.distance.unwrap_or(f32::MAX),
                    b.distance.unwrap_or(f32::MAX),
                );
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        // 5. Rate limit and frame budget.
        let mut out = Vec::new();
        for intent in surviving {
            if out.len() >= self.config.max_per_frame {
                self.last_drops.push((intent.text, Dropped::OverBudget));
                continue;
            }

            // Critical bypasses rate limiting. If the player is about to be
            // hit, a spent budget is not a reason to stay quiet.
            if intent.priority != Priority::Critical {
                let (cap, refill) = (
                    self.config.bucket_capacity,
                    self.config.bucket_refill_per_sec,
                );
                let bucket = self
                    .buckets
                    .entry(intent.category.clone())
                    .or_insert_with(|| Bucket::new(cap, refill, now));

                if !bucket.try_spend(now) {
                    self.last_drops.push((intent.text, Dropped::RateLimited));
                    continue;
                }
            }

            if let Some(window) = intent.dedup_for {
                self.spoken_until.insert(intent.text.clone(), now + window);
            }

            out.push(Utterance {
                text: intent.text,
                // Only critical barges in. Everything else waits its turn.
                interrupt: intent.priority == Priority::Critical,
                priority: intent.priority,
            });
        }

        out
    }

    /// Keeps only the nearest intent per collapse key.
    fn collapse(&mut self, intents: Vec<Intent>) -> Vec<Intent> {
        // Index of the current winner for each key.
        let mut winners: HashMap<String, usize> = HashMap::new();
        let mut keep: Vec<bool> = vec![true; intents.len()];

        for (i, intent) in intents.iter().enumerate() {
            let Some(key) = intent.collapse_key.as_ref() else {
                continue;
            };

            match winners.get(key) {
                None => {
                    winners.insert(key.clone(), i);
                }
                Some(&best) => {
                    let d_new = intent.distance.unwrap_or(f32::MAX);
                    let d_best = intents[best].distance.unwrap_or(f32::MAX);
                    if d_new < d_best {
                        keep[best] = false;
                        winners.insert(key.clone(), i);
                    } else {
                        keep[i] = false;
                    }
                }
            }
        }

        let mut out = Vec::with_capacity(intents.len());
        for (i, intent) in intents.into_iter().enumerate() {
            if keep[i] {
                out.push(intent);
            } else {
                self.last_drops.push((intent.text, Dropped::Collapsed));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: f32) -> Duration {
        Duration::from_secs_f32(s)
    }

    fn arbiter() -> Arbiter {
        Arbiter::new(Config {
            verbosity: 3,
            max_per_frame: 8,
            ..Config::default()
        })
    }

    #[test]
    fn verbosity_gates_by_priority_class() {
        let mut a = Arbiter::new(Config {
            verbosity: 0,
            ..Config::default()
        });
        let out = a.resolve(
            vec![
                Intent::new("bush to the north", Priority::Ambient, "cone"),
                Intent::new("incoming attack", Priority::Critical, "combat"),
            ],
            secs(0.0),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "incoming attack");
        assert_eq!(
            a.last_drops(),
            [("bush to the north".into(), Dropped::Verbosity)]
        );
    }

    #[test]
    fn collapses_many_triggers_to_the_nearest() {
        // The doom notes case: every lift in the room announcing itself.
        let mut a = arbiter();
        let out = a.resolve(
            vec![
                Intent::new("lift far", Priority::Navigation, "nav").collapse("lift", 90.0),
                Intent::new("lift near", Priority::Navigation, "nav").collapse("lift", 12.0),
                Intent::new("lift mid", Priority::Navigation, "nav").collapse("lift", 40.0),
            ],
            secs(0.0),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "lift near");
    }

    #[test]
    fn different_collapse_keys_do_not_interfere() {
        let mut a = arbiter();
        let out = a.resolve(
            vec![
                Intent::new("chest", Priority::Navigation, "nav").collapse("chest", 50.0),
                Intent::new("door", Priority::Navigation, "nav").collapse("door", 80.0),
            ],
            secs(0.0),
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn suppresses_repeats_inside_the_dedup_window() {
        let mut a = arbiter();
        let say = || {
            vec![Intent::new("blocked by bush", Priority::Navigation, "nav").dedup_for(secs(2.0))]
        };

        assert_eq!(a.resolve(say(), secs(0.0)).len(), 1);
        assert_eq!(a.resolve(say(), secs(1.0)).len(), 0, "still inside window");
        assert_eq!(a.resolve(say(), secs(2.5)).len(), 1, "window elapsed");
    }

    #[test]
    fn rate_limits_a_chatty_category() {
        let mut a = Arbiter::new(Config {
            verbosity: 3,
            max_per_frame: 100,
            bucket_capacity: 3.0,
            bucket_refill_per_sec: 0.0,
        });
        let intents: Vec<_> = (0..10)
            .map(|i| Intent::new(format!("item {i}"), Priority::Ambient, "cone"))
            .collect();

        let out = a.resolve(intents, secs(0.0));
        assert_eq!(out.len(), 3, "burst capacity only");
        assert_eq!(
            a.last_drops()
                .iter()
                .filter(|(_, d)| *d == Dropped::RateLimited)
                .count(),
            7
        );
    }

    #[test]
    fn rate_limit_buckets_are_per_category() {
        let mut a = Arbiter::new(Config {
            verbosity: 3,
            max_per_frame: 100,
            bucket_capacity: 1.0,
            bucket_refill_per_sec: 0.0,
        });
        let out = a.resolve(
            vec![
                Intent::new("a", Priority::Ambient, "cone"),
                Intent::new("b", Priority::Ambient, "cone"),
                Intent::new("c", Priority::Ambient, "proximity"),
            ],
            secs(0.0),
        );
        // One from each category; the second "cone" is dropped.
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|u| u.text == "a"));
        assert!(out.iter().any(|u| u.text == "c"));
    }

    #[test]
    fn critical_bypasses_an_exhausted_bucket() {
        let mut a = Arbiter::new(Config {
            verbosity: 3,
            max_per_frame: 100,
            bucket_capacity: 0.0,
            bucket_refill_per_sec: 0.0,
        });
        let out = a.resolve(
            vec![
                Intent::new("scenery", Priority::Ambient, "combat"),
                Intent::new("incoming attack", Priority::Critical, "combat"),
            ],
            secs(0.0),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "incoming attack");
    }

    #[test]
    fn only_critical_interrupts() {
        let mut a = arbiter();
        let out = a.resolve(
            vec![
                Intent::new("death", Priority::Critical, "combat"),
                Intent::new("entering kakariko", Priority::Navigation, "nav"),
            ],
            secs(0.0),
        );
        assert!(out[0].interrupt);
        assert!(!out[1].interrupt);
    }

    #[test]
    fn frame_budget_keeps_the_most_urgent() {
        let mut a = Arbiter::new(Config {
            verbosity: 3,
            max_per_frame: 1,
            ..Config::default()
        });
        let out = a.resolve(
            vec![
                Intent::new("scenery", Priority::Ambient, "cone"),
                Intent::new("low health", Priority::Critical, "combat"),
            ],
            secs(0.0),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "low health");
    }

    #[test]
    fn nearer_wins_within_a_priority_class() {
        let mut a = Arbiter::new(Config {
            verbosity: 3,
            max_per_frame: 1,
            ..Config::default()
        });
        let out = a.resolve(
            vec![
                Intent::new("far chest", Priority::Navigation, "a").collapse("x", 90.0),
                Intent::new("near door", Priority::Navigation, "b").collapse("y", 5.0),
            ],
            secs(0.0),
        );
        assert_eq!(out[0].text, "near door");
    }

    #[test]
    fn buckets_refill_over_time() {
        let mut a = Arbiter::new(Config {
            verbosity: 3,
            max_per_frame: 100,
            bucket_capacity: 1.0,
            bucket_refill_per_sec: 1.0,
        });
        let say = || vec![Intent::new("tick", Priority::Ambient, "cone")];

        assert_eq!(a.resolve(say(), secs(0.0)).len(), 1);
        assert_eq!(a.resolve(say(), secs(0.1)).len(), 0);
        assert_eq!(a.resolve(say(), secs(1.5)).len(), 1, "refilled");
    }

    #[test]
    fn resolve_is_deterministic_for_the_same_inputs() {
        // The property golden-file replay testing depends on.
        let run = || {
            let mut a = arbiter();
            let mut all = Vec::new();
            for frame in 0..30 {
                let t = secs(frame as f32 / 60.0);
                all.extend(a.resolve(
                    vec![
                        Intent::new("chest", Priority::Interaction, "prox")
                            .collapse("chest", frame as f32),
                        Intent::new("wall", Priority::Ambient, "cone").dedup_for(secs(0.5)),
                    ],
                    t,
                ));
            }
            all
        };
        assert_eq!(run(), run());
    }
}
