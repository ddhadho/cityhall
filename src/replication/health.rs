//! Replica Health Monitoring
//!
//! Tracks replica health state and transitions based on sync success/failure

use std::time::{SystemTime, Duration};

/// Health state of a replica
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    /// All syncs successful, no recent failures
    Healthy,
    
    /// Some recent failures, but still operational
    Degraded,
    
    /// Sustained failures, replica is unhealthy
    Unhealthy,
}

impl HealthState {
    /// Check if state indicates operational status
    pub fn is_operational(&self) -> bool {
        !matches!(self, HealthState::Unhealthy)
    }
}

/// Detailed health tracking for a replica
#[derive(Debug)]
pub struct ReplicaHealth {
    /// Current health state
    state: HealthState,
    
    /// Last successful sync timestamp
    last_success: Option<SystemTime>,
    
    /// Number of consecutive failures
    consecutive_failures: u32,
    
    /// Total number of failures (lifetime)
    total_failures: u64,
    
    /// Total number of successes (lifetime)
    total_successes: u64,
    
    /// Last error message (if any)
    last_error: Option<String>,
}

impl ReplicaHealth {
    /// Create new health tracker
    pub fn new() -> Self {
        Self {
            state: HealthState::Healthy,
            last_success: None,
            consecutive_failures: 0,
            total_failures: 0,
            total_successes: 0,
            last_error: None,
        }
    }
    
    /// Record a successful sync
    pub fn record_success(&mut self) {
        self.last_success = Some(SystemTime::now());
        self.consecutive_failures = 0;
        self.total_successes += 1;
        self.last_error = None;
        self.state = HealthState::Healthy;
    }
    
    /// Record a failed sync
    pub fn record_failure(&mut self, error: &crate::StorageError) {
        self.consecutive_failures += 1;
        self.total_failures += 1;
        self.last_error = Some(error.to_string());
        
        // Update state based on consecutive failures
        self.state = match self.consecutive_failures {
            0 => HealthState::Healthy,
            1..=2 => HealthState::Degraded,
            _ => HealthState::Unhealthy,
        };
    }
    
    /// Get current health state
    pub fn state(&self) -> HealthState {
        self.state
    }
    
    /// Check if replica is healthy
    pub fn is_healthy(&self) -> bool {
        self.state == HealthState::Healthy
    }
    
    /// Check if replica is operational (not unhealthy)
    pub fn is_operational(&self) -> bool {
        self.state.is_operational()
    }
    
    /// Get number of consecutive failures
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }
    
    /// Get total failures (lifetime)
    pub fn total_failures(&self) -> u64 {
        self.total_failures
    }
    
    /// Get total successes (lifetime)
    pub fn total_successes(&self) -> u64 {
        self.total_successes
    }
    
    /// Get success rate (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        let total = self.total_successes + self.total_failures;
        if total == 0 {
            return 1.0; // No attempts yet, assume healthy
        }
        self.total_successes as f64 / total as f64
    }
    
    /// Get time since last successful sync
    pub fn time_since_success(&self) -> Option<Duration> {
        self.last_success
            .and_then(|t| SystemTime::now().duration_since(t).ok())
    }
    
    /// Get last error message
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    
    /// Check if replica is stale (no success in given duration)
    pub fn is_stale(&self, threshold: Duration) -> bool {
        match self.time_since_success() {
            Some(duration) => duration > threshold,
            None => true, // Never succeeded
        }
    }
    
    /// Get a human-readable status report
    pub fn status_report(&self) -> String {
        let state_str = match self.state {
            HealthState::Healthy => "✓ Healthy",
            HealthState::Degraded => "⚠ Degraded",
            HealthState::Unhealthy => "❌ Unhealthy",
        };
        
        let time_since = match self.time_since_success() {
            Some(d) => format!("{:.1}s ago", d.as_secs_f64()),
            None => "never".to_string(),
        };
        
        let success_rate = (self.success_rate() * 100.0).round() as u32;
        
        format!(
            "{}\n  Last success: {}\n  Consecutive failures: {}\n  Success rate: {}%\n  Total: {} successes, {} failures",
            state_str,
            time_since,
            self.consecutive_failures,
            success_rate,
            self.total_successes,
            self.total_failures
        )
    }
}

impl Default for ReplicaHealth {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StorageError;
    
    #[test]
    fn test_initial_state() {
        let health = ReplicaHealth::new();
        
        assert_eq!(health.state(), HealthState::Healthy);
        assert!(health.is_healthy());
        assert!(health.is_operational());
        assert_eq!(health.consecutive_failures(), 0);
        assert_eq!(health.total_failures(), 0);
        assert_eq!(health.total_successes(), 0);
        assert_eq!(health.success_rate(), 1.0); // No attempts = 100%
        assert!(health.last_error().is_none());
    }
    
    #[test]
    fn test_record_success() {
        let mut health = ReplicaHealth::new();
        
        health.record_success();
        
        assert_eq!(health.state(), HealthState::Healthy);
        assert_eq!(health.consecutive_failures(), 0);
        assert_eq!(health.total_successes(), 1);
        assert!(health.time_since_success().is_some());
        
        // Multiple successes
        health.record_success();
        health.record_success();
        
        assert_eq!(health.total_successes(), 3);
        assert_eq!(health.success_rate(), 1.0);
    }
    
    #[test]
    fn test_state_transitions() {
        let mut health = ReplicaHealth::new();
        let error = StorageError::ConnectionFailed("test".to_string());
        
        // Initially healthy
        assert_eq!(health.state(), HealthState::Healthy);
        
        // First failure -> Degraded
        health.record_failure(&error);
        assert_eq!(health.state(), HealthState::Degraded);
        assert_eq!(health.consecutive_failures(), 1);
        
        // Second failure -> Still Degraded
        health.record_failure(&error);
        assert_eq!(health.state(), HealthState::Degraded);
        assert_eq!(health.consecutive_failures(), 2);
        
        // Third failure -> Unhealthy
        health.record_failure(&error);
        assert_eq!(health.state(), HealthState::Unhealthy);
        assert_eq!(health.consecutive_failures(), 3);
        assert!(!health.is_healthy());
        assert!(!health.is_operational());
        
        // Success resets to Healthy
        health.record_success();
        assert_eq!(health.state(), HealthState::Healthy);
        assert_eq!(health.consecutive_failures(), 0);
    }
    
    #[test]
    fn test_success_rate() {
        let mut health = ReplicaHealth::new();
        let error = StorageError::ConnectionFailed("test".to_string());
        
        // 3 successes, 1 failure = 75%
        health.record_success();
        health.record_success();
        health.record_success();
        health.record_failure(&error);
        
        assert_eq!(health.total_successes(), 3);
        assert_eq!(health.total_failures(), 1);
        assert_eq!(health.success_rate(), 0.75);
    }
    
    #[test]
    fn test_error_tracking() {
        let mut health = ReplicaHealth::new();
        let error = StorageError::Timeout("Connection timeout".to_string());
        
        assert!(health.last_error().is_none());
        
        health.record_failure(&error);
        
        assert!(health.last_error().is_some());
        assert!(health.last_error().unwrap().contains("timeout"));
        
        // Success clears error
        health.record_success();
        assert!(health.last_error().is_none());
    }
    
    #[test]
    fn test_time_since_success() {
        let mut health = ReplicaHealth::new();
        
        // No success yet
        assert!(health.time_since_success().is_none());
        
        // Record success
        health.record_success();
        
        // Should have a recent timestamp
        let elapsed = health.time_since_success().unwrap();
        assert!(elapsed.as_secs() < 1);
    }
    
    #[test]
    fn test_is_stale() {
        let mut health = ReplicaHealth::new();
        
        // Never succeeded -> stale
        assert!(health.is_stale(Duration::from_secs(60)));
        
        // Just succeeded -> not stale
        health.record_success();
        assert!(!health.is_stale(Duration::from_secs(60)));
        
        // Simulate old success
        health.last_success = Some(
            SystemTime::now() - Duration::from_secs(120)
        );
        
        // Now stale (120s > 60s threshold)
        assert!(health.is_stale(Duration::from_secs(60)));
    }
    
    #[test]
    fn test_status_report() {
        let mut health = ReplicaHealth::new();
        let error = StorageError::ConnectionFailed("test".to_string());
        
        // Initial report
        let report = health.status_report();
        assert!(report.contains("Healthy"));
        assert!(report.contains("never")); // No success yet
        
        // After success
        health.record_success();
        let report = health.status_report();
        assert!(report.contains("Healthy"));
        assert!(report.contains("ago"));
        
        // After failures
        health.record_failure(&error);
        health.record_failure(&error);
        health.record_failure(&error);
        
        let report = health.status_report();
        assert!(report.contains("Unhealthy"));
        assert!(report.contains("Consecutive failures: 3"));
    }
    
    #[test]
    fn test_mixed_success_failure_pattern() {
        let mut health = ReplicaHealth::new();
        let error = StorageError::ConnectionFailed("test".to_string());
        
        // Pattern: S, S, F, S, F, F, S
        health.record_success();  // 1 success
        health.record_success();  // 2 successes
        health.record_failure(&error);  // 1 failure, degraded
        health.record_success();  // 3 successes, back to healthy
        health.record_failure(&error);  // 2 failures, degraded
        health.record_failure(&error);  // 3 failures, degraded
        health.record_success();  // 4 successes, back to healthy
        
        assert_eq!(health.total_successes(), 4);
        assert_eq!(health.total_failures(), 3);
        assert_eq!(health.state(), HealthState::Healthy);
        assert_eq!(health.consecutive_failures(), 0);
        
        // Success rate: 4/7 ≈ 0.571
        assert!((health.success_rate() - 0.571).abs() < 0.01);
    }
    
    #[test]
    fn test_health_state_is_operational() {
        assert!(HealthState::Healthy.is_operational());
        assert!(HealthState::Degraded.is_operational());
        assert!(!HealthState::Unhealthy.is_operational());
    }
}