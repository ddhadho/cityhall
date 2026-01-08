//! Exponential Backoff Strategy
//!
//! Implements exponential backoff with jitter for retry logic.
//!
//! ## Usage
//!
//! ```rust
//! let mut backoff = ExponentialBackoff::new();
//!
//! loop {
//!     match attempt_operation().await {
//!         Ok(_) => {
//!             backoff.reset();  // Success - reset backoff
//!             break;
//!         }
//!         Err(e) => {
//!             let wait = backoff.next();  // Get next wait duration
//!             eprintln!("Error: {}, retrying in {:?}", e, wait);
//!             sleep(wait).await;
//!         }
//!     }
//! }
//! ```

use std::time::Duration;
use fastrand::Rng;

/// Exponential backoff calculator with jitter
/// 
/// Starts at 1 second and doubles each retry up to a maximum of 60 seconds.
/// Adds random jitter (±25%) to prevent thundering herd problem.
#[derive(Debug)]
pub struct ExponentialBackoff {
    /// Current backoff duration
    current: Duration,
    
    /// Minimum backoff duration (starting point)
    min: Duration,
    
    /// Maximum backoff duration (ceiling)
    max: Duration,
    
    /// Multiplication factor for each retry
    factor: u32,
    
    /// Random number generator for jitter
    rng: Rng,
    
    /// Number of attempts made
    attempts: u64,
}

impl ExponentialBackoff {
    /// Create a new backoff with default parameters
    /// 
    /// - Initial delay: 1 second
    /// - Max delay: 60 seconds
    /// - Factor: 2x per retry
    /// - Jitter: ±25%
    pub fn new() -> Self {
        Self::with_params(
            Duration::from_secs(1),
            Duration::from_secs(60),
            2,
        )
    }
    
    /// Create a backoff with custom parameters
    pub fn with_params(min: Duration, max: Duration, factor: u32) -> Self {
        Self {
            current: min,
            min,
            max,
            factor,
            rng: Rng::new(),
            attempts: 0,
        }
    }
    
    /// Get the next backoff duration
    /// 
    /// Returns the current duration with jitter applied, then increases
    /// the duration for the next call.
    pub fn next(&mut self) -> Duration {
        self.attempts += 1;
        
        let duration = self.current;
        
        // Add jitter: ±25% of current duration
        let jitter_factor = self.rng.f64() * 0.5 - 0.25; // -0.25 to +0.25
        let jittered = duration.mul_f64(1.0 + jitter_factor);
        
        // Increase for next time (with factor)
        self.current = (self.current * self.factor).min(self.max);
        
        jittered
    }
    
    /// Reset backoff to initial state
    /// 
    /// Call this after a successful operation to reset the backoff
    /// duration back to the minimum.
    pub fn reset(&mut self) {
        self.current = self.min;
        self.attempts = 0;
    }
    
    /// Get the current backoff duration (without jitter)
    pub fn current(&self) -> Duration {
        self.current
    }
    
    /// Get number of attempts made since last reset
    pub fn attempts(&self) -> u64 {
        self.attempts
    }
    
    /// Check if backoff has reached maximum
    pub fn is_maxed(&self) -> bool {
        self.current >= self.max
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_backoff_starts_at_minimum() {
        let backoff = ExponentialBackoff::new();
        assert_eq!(backoff.current(), Duration::from_secs(1));
        assert_eq!(backoff.attempts(), 0);
    }
    
    #[test]
    fn test_backoff_increases_exponentially() {
        let mut backoff = ExponentialBackoff::new();
        
        // First call: 1s (with jitter)
        let first = backoff.next();
        assert!(first.as_secs_f64() >= 0.75 && first.as_secs_f64() <= 1.25);
        assert_eq!(backoff.current(), Duration::from_secs(2)); // Next will be 2s
        assert_eq!(backoff.attempts(), 1);
        
        // Second call: 2s (with jitter)
        let second = backoff.next();
        assert!(second.as_secs_f64() >= 1.5 && second.as_secs_f64() <= 2.5);
        assert_eq!(backoff.current(), Duration::from_secs(4)); // Next will be 4s
        assert_eq!(backoff.attempts(), 2);
        
        // Third call: 4s (with jitter)
        let third = backoff.next();
        assert!(third.as_secs_f64() >= 3.0 && third.as_secs_f64() <= 5.0);
        assert_eq!(backoff.current(), Duration::from_secs(8)); // Next will be 8s
        assert_eq!(backoff.attempts(), 3);
    }
    
    #[test]
    fn test_backoff_caps_at_maximum() {
        let mut backoff = ExponentialBackoff::with_params(
            Duration::from_secs(1),
            Duration::from_secs(10), // Low max for testing
            2,
        );
        
        // Keep calling until we hit max
        for _ in 0..10 {
            backoff.next();
        }
        
        assert!(backoff.is_maxed());
        assert_eq!(backoff.current(), Duration::from_secs(10));
        
        // Further calls should stay at max
        let next = backoff.next();
        assert!(next.as_secs_f64() >= 7.5 && next.as_secs_f64() <= 12.5); // 10s ± 25%
        assert_eq!(backoff.current(), Duration::from_secs(10)); // Still maxed
    }
    
    #[test]
    fn test_backoff_reset() {
        let mut backoff = ExponentialBackoff::new();
        
        // Increase backoff several times
        backoff.next();
        backoff.next();
        backoff.next();
        
        assert_eq!(backoff.current(), Duration::from_secs(8));
        assert_eq!(backoff.attempts(), 3);
        
        // Reset
        backoff.reset();
        
        assert_eq!(backoff.current(), Duration::from_secs(1));
        assert_eq!(backoff.attempts(), 0);
        assert!(!backoff.is_maxed());
    }
    
    #[test]
    fn test_jitter_varies() {
        let  _backoff = ExponentialBackoff::new();
        
        // Collect multiple samples
        let mut samples = Vec::new();
        for _ in 0..10 {
            let mut b = ExponentialBackoff::new();
            samples.push(b.next().as_millis());
        }
        
        // Samples should vary due to jitter
        let min = *samples.iter().min().unwrap();
        let max = *samples.iter().max().unwrap();
        
        // With ±25% jitter on 1s, range should be 750-1250ms
        assert!(min >= 750, "Min sample {} should be >= 750ms", min);
        assert!(max <= 1250, "Max sample {} should be <= 1250ms", max);
        assert!(max > min, "Jitter should create variation");
    }
    
    #[test]
    fn test_custom_parameters() {
        let mut backoff = ExponentialBackoff::with_params(
            Duration::from_millis(500),  // Start at 500ms
            Duration::from_secs(5),       // Max 5s
            3,                            // 3x factor
        );
        
        assert_eq!(backoff.current(), Duration::from_millis(500));
        
        backoff.next();
        assert_eq!(backoff.current(), Duration::from_millis(1500)); // 500 * 3
        
        backoff.next();
        assert_eq!(backoff.current(), Duration::from_millis(4500)); // 1500 * 3
        
        backoff.next();
        assert_eq!(backoff.current(), Duration::from_secs(5)); // Capped at max
    }
    
    #[test]
    fn test_zero_attempts_initially() {
        let backoff = ExponentialBackoff::new();
        assert_eq!(backoff.attempts(), 0);
    }
    
    #[test]
    fn test_attempts_increment() {
        let mut backoff = ExponentialBackoff::new();
        
        for i in 1..=5 {
            backoff.next();
            assert_eq!(backoff.attempts(), i);
        }
    }
    
    #[test]
    fn test_attempts_reset() {
        let mut backoff = ExponentialBackoff::new();
        
        backoff.next();
        backoff.next();
        assert_eq!(backoff.attempts(), 2);
        
        backoff.reset();
        assert_eq!(backoff.attempts(), 0);
    }
    
    #[test]
    fn test_realistic_scenario() {
        let mut backoff = ExponentialBackoff::new();
        
        // Simulate 5 failures
        let waits: Vec<Duration> = (0..5).map(|_| backoff.next()).collect();
        
        // Check progression: ~1s, ~2s, ~4s, ~8s, ~16s
        println!("Backoff progression:");
        for (i, wait) in waits.iter().enumerate() {
            println!("  Attempt {}: wait {:?}", i + 1, wait);
        }
        
        // Verify roughly exponential growth
        assert!(waits[1] > waits[0]);
        assert!(waits[2] > waits[1]);
        assert!(waits[3] > waits[2]);
        assert!(waits[4] > waits[3]);
        
        // After success, reset
        backoff.reset();
        let first_after_reset = backoff.next();
        
        // Should be back to ~1s
        assert!(first_after_reset.as_secs_f64() >= 0.75);
        assert!(first_after_reset.as_secs_f64() <= 1.25);
    }
}