//! Real-time data feed integration for JobCard.
//!
//! Provides types and utilities for ingesting, validating, and monitoring
//! real-time data streams from IoT sensors, GPS devices, and HTTP endpoints.
//!
//! # Example
//!
//! ```rust
//! use jobcard_core::realtime::{
//!     FeedConfig, FeedSourceType, ValidationConfig, ValueRange, FeedHealth,
//!     validate_record, FeedMetrics, example_gps_record,
//! };
//! use std::collections::HashMap;
//!
//! let mut ranges = HashMap::new();
//! ranges.insert("latitude".to_string(), ValueRange { min: -90.0, max: 90.0 });
//! ranges.insert("longitude".to_string(), ValueRange { min: -180.0, max: 180.0 });
//!
//! let config = FeedConfig {
//!     id: "gps-fleet-01".to_string(),
//!     source_type: FeedSourceType::Gps,
//!     endpoint: "udp://0.0.0.0:5005".to_string(),
//!     poll_interval_secs: 10,
//!     validation: ValidationConfig {
//!         required_fields: vec!["latitude".to_string(), "longitude".to_string()],
//!         max_staleness_secs: 60,
//!         value_ranges: ranges,
//!     },
//! };
//!
//! let record = example_gps_record("gps-fleet-01", 37.7749, -122.4194, 60.0);
//! let result = validate_record(&record, &config);
//! assert!(result.valid);
//!
//! let mut metrics = FeedMetrics::new("gps-fleet-01".to_string());
//! metrics.record_received(result.valid);
//! assert_eq!(metrics.health, FeedHealth::Healthy);
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a real-time data feed source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedConfig {
    /// Unique identifier for this feed.
    pub id: String,
    /// The type of data source.
    pub source_type: FeedSourceType,
    /// Connection endpoint (URL, socket address, or file path).
    pub endpoint: String,
    /// How often to poll the source, in seconds.
    pub poll_interval_secs: u64,
    /// Validation rules applied to incoming records.
    pub validation: ValidationConfig,
}

/// Supported real-time data source types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedSourceType {
    /// GPS or GNSS position feed.
    Gps,
    /// Generic IoT sensor feed.
    Iot,
    /// HTTP/HTTPS polling endpoint.
    Http,
    /// Local file updated by an external process.
    File,
}

/// Validation rules for incoming feed records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// Fields that must be present in every record.
    pub required_fields: Vec<String>,
    /// Maximum allowed age of a record in seconds before it is considered stale.
    pub max_staleness_secs: u64,
    /// Optional numeric range constraints keyed by field name.
    pub value_ranges: HashMap<String, ValueRange>,
}

/// Inclusive numeric range `[min, max]` for a field value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueRange {
    /// Minimum acceptable value (inclusive).
    pub min: f64,
    /// Maximum acceptable value (inclusive).
    pub max: f64,
}

/// A single record received from a data feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedRecord {
    /// The feed this record belongs to.
    pub feed_id: String,
    /// When the record was produced by the source.
    pub timestamp: DateTime<Utc>,
    /// Payload key-value pairs.
    pub fields: HashMap<String, serde_json::Value>,
}

/// Outcome of validating a [`FeedRecord`] against a [`FeedConfig`].
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the record passed all validation rules.
    pub valid: bool,
    /// Human-readable descriptions of every rule that failed.
    pub errors: Vec<String>,
}

impl ValidationResult {
    /// Creates a successful result with no errors.
    pub fn ok() -> Self {
        Self {
            valid: true,
            errors: vec![],
        }
    }

    /// Creates a failed result with the given error messages.
    pub fn err(errors: Vec<String>) -> Self {
        Self {
            valid: false,
            errors,
        }
    }
}

/// Validate `record` against the rules in `config`.
///
/// Checks performed:
/// 1. All [`ValidationConfig::required_fields`] are present.
/// 2. All numeric fields listed in [`ValidationConfig::value_ranges`] fall within
///    their declared ranges.
/// 3. The record's [`FeedRecord::timestamp`] is not older than
///    [`ValidationConfig::max_staleness_secs`].
pub fn validate_record(record: &FeedRecord, config: &FeedConfig) -> ValidationResult {
    let mut errors = Vec::new();

    // 1. Required fields
    for field in &config.validation.required_fields {
        if !record.fields.contains_key(field) {
            errors.push(format!("missing required field: {field}"));
        }
    }

    // 2. Value ranges
    for (field, range) in &config.validation.value_ranges {
        if let Some(value) = record.fields.get(field) {
            if let Some(n) = value.as_f64() {
                if n < range.min || n > range.max {
                    errors.push(format!(
                        "field '{field}' value {n} out of range [{}, {}]",
                        range.min, range.max
                    ));
                }
            }
        }
    }

    // 3. Staleness
    let age_secs = (Utc::now() - record.timestamp).num_seconds().unsigned_abs();
    if age_secs > config.validation.max_staleness_secs {
        errors.push(format!(
            "record is stale: {age_secs}s old (max {}s allowed)",
            config.validation.max_staleness_secs
        ));
    }

    if errors.is_empty() {
        ValidationResult::ok()
    } else {
        ValidationResult::err(errors)
    }
}

/// Overall health state of a feed, derived from its error rate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedHealth {
    /// Error rate < 5 %.
    Healthy,
    /// Error rate between 5 % and 25 %.
    Degraded,
    /// No data received yet, or error rate ≥ 25 %.
    Down,
}

/// Rolling metrics for a single feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedMetrics {
    /// Feed identifier.
    pub feed_id: String,
    /// Total records received since the metrics object was created.
    pub records_received: u64,
    /// Records that passed validation.
    pub records_valid: u64,
    /// Records that failed validation.
    pub records_invalid: u64,
    /// Timestamp of the most recent record, if any.
    pub last_received: Option<DateTime<Utc>>,
    /// Derived health status.
    pub health: FeedHealth,
}

impl FeedMetrics {
    /// Creates a fresh metrics object for the given feed.
    pub fn new(feed_id: String) -> Self {
        Self {
            feed_id,
            records_received: 0,
            records_valid: 0,
            records_invalid: 0,
            last_received: None,
            health: FeedHealth::Down,
        }
    }

    /// Records that one record was received and updates health.
    pub fn record_received(&mut self, valid: bool) {
        self.records_received += 1;
        self.last_received = Some(Utc::now());
        if valid {
            self.records_valid += 1;
        } else {
            self.records_invalid += 1;
        }
        self.update_health();
    }

    /// Fraction of records that passed validation (`0.0`–`1.0`).
    pub fn success_rate(&self) -> f64 {
        if self.records_received == 0 {
            0.0
        } else {
            self.records_valid as f64 / self.records_received as f64
        }
    }

    fn update_health(&mut self) {
        let error_rate = if self.records_received == 0 {
            1.0
        } else {
            self.records_invalid as f64 / self.records_received as f64
        };

        self.health = if error_rate < 0.05 {
            FeedHealth::Healthy
        } else if error_rate < 0.25 {
            FeedHealth::Degraded
        } else {
            FeedHealth::Down
        };
    }
}

/// Severity level for a monitoring alert.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    /// Informational notice.
    Info,
    /// Degraded performance or elevated error rate.
    Warning,
    /// Feed is down or completely failing.
    Critical,
}

/// An alert produced by the monitoring system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    /// Feed that triggered the alert.
    pub feed_id: String,
    /// Severity classification.
    pub severity: AlertSeverity,
    /// Human-readable description.
    pub message: String,
    /// When the alert was generated.
    pub timestamp: DateTime<Utc>,
}

/// Inspect `metrics` and return any active alerts.
///
/// Raises a `Critical` alert when the feed is [`FeedHealth::Down`], a
/// `Warning` when it is [`FeedHealth::Degraded`], and a `Warning` when no
/// data has arrived within the last 5 minutes.
pub fn check_alerts(metrics: &FeedMetrics) -> Vec<Alert> {
    let mut alerts = Vec::new();
    let now = Utc::now();

    match metrics.health {
        FeedHealth::Down => alerts.push(Alert {
            feed_id: metrics.feed_id.clone(),
            severity: AlertSeverity::Critical,
            message: format!("Feed '{}' is down", metrics.feed_id),
            timestamp: now,
        }),
        FeedHealth::Degraded => alerts.push(Alert {
            feed_id: metrics.feed_id.clone(),
            severity: AlertSeverity::Warning,
            message: format!(
                "Feed '{}' is degraded: {:.1}% success rate",
                metrics.feed_id,
                metrics.success_rate() * 100.0
            ),
            timestamp: now,
        }),
        FeedHealth::Healthy => {}
    }

    // Stale feed check (5 minutes)
    if let Some(last) = metrics.last_received {
        let age_secs = (now - last).num_seconds();
        if age_secs > 300 {
            alerts.push(Alert {
                feed_id: metrics.feed_id.clone(),
                severity: AlertSeverity::Warning,
                message: format!(
                    "Feed '{}' has not received data for {age_secs}s",
                    metrics.feed_id
                ),
                timestamp: now,
            });
        }
    }

    alerts
}

/// Aggregated validation results for a job's output records.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationSummary {
    /// Total records scanned.
    pub total: u64,
    /// Records that passed validation.
    pub valid: u64,
    /// Records that failed validation.
    pub invalid: u64,
    /// Total alerts generated.
    pub alert_count: u64,
    /// Number of critical-severity alerts.
    pub critical_alerts: u64,
    /// Derived feed health based on error rate.
    pub health: FeedHealth,
}

impl ValidationSummary {
    /// Returns the single-character badge for terminal display.
    ///
    /// - `✓` Healthy
    /// - `⚠` Degraded
    /// - `✗` Down
    pub fn badge(&self) -> &'static str {
        match self.health {
            FeedHealth::Healthy => "✓",
            FeedHealth::Degraded => "⚠",
            FeedHealth::Down => "✗",
        }
    }
}

// ---------------------------------------------------------------------------
// Example data generators
// ---------------------------------------------------------------------------

/// Build a sample GPS record suitable for testing or documentation.
pub fn example_gps_record(feed_id: &str, lat: f64, lon: f64, speed_kmh: f64) -> FeedRecord {
    let mut fields = HashMap::new();
    fields.insert("latitude".to_string(), serde_json::json!(lat));
    fields.insert("longitude".to_string(), serde_json::json!(lon));
    fields.insert("speed_kmh".to_string(), serde_json::json!(speed_kmh));
    fields.insert("heading_deg".to_string(), serde_json::json!(0.0_f64));

    FeedRecord {
        feed_id: feed_id.to_string(),
        timestamp: Utc::now(),
        fields,
    }
}

/// Build a sample IoT sensor record suitable for testing or documentation.
pub fn example_iot_record(
    feed_id: &str,
    sensor_id: &str,
    temperature_c: f64,
    humidity_pct: f64,
) -> FeedRecord {
    let mut fields = HashMap::new();
    fields.insert("sensor_id".to_string(), serde_json::json!(sensor_id));
    fields.insert(
        "temperature_c".to_string(),
        serde_json::json!(temperature_c),
    );
    fields.insert("humidity_pct".to_string(), serde_json::json!(humidity_pct));

    FeedRecord {
        feed_id: feed_id.to_string(),
        timestamp: Utc::now(),
        fields,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn gps_config() -> FeedConfig {
        let mut ranges = HashMap::new();
        ranges.insert(
            "latitude".to_string(),
            ValueRange {
                min: -90.0,
                max: 90.0,
            },
        );
        ranges.insert(
            "longitude".to_string(),
            ValueRange {
                min: -180.0,
                max: 180.0,
            },
        );
        ranges.insert(
            "speed_kmh".to_string(),
            ValueRange {
                min: 0.0,
                max: 300.0,
            },
        );

        FeedConfig {
            id: "gps-test".to_string(),
            source_type: FeedSourceType::Gps,
            endpoint: "udp://0.0.0.0:5005".to_string(),
            poll_interval_secs: 10,
            validation: ValidationConfig {
                required_fields: vec![
                    "latitude".to_string(),
                    "longitude".to_string(),
                    "speed_kmh".to_string(),
                ],
                max_staleness_secs: 60,
                value_ranges: ranges,
            },
        }
    }

    fn iot_config() -> FeedConfig {
        let mut ranges = HashMap::new();
        ranges.insert(
            "temperature_c".to_string(),
            ValueRange {
                min: -50.0,
                max: 100.0,
            },
        );
        ranges.insert(
            "humidity_pct".to_string(),
            ValueRange {
                min: 0.0,
                max: 100.0,
            },
        );

        FeedConfig {
            id: "iot-test".to_string(),
            source_type: FeedSourceType::Iot,
            endpoint: "mqtt://broker:1883/sensors".to_string(),
            poll_interval_secs: 30,
            validation: ValidationConfig {
                required_fields: vec![
                    "sensor_id".to_string(),
                    "temperature_c".to_string(),
                    "humidity_pct".to_string(),
                ],
                max_staleness_secs: 120,
                value_ranges: ranges,
            },
        }
    }

    // -- FeedConfig & types --------------------------------------------------

    #[test]
    fn feed_config_roundtrips_json() {
        let cfg = gps_config();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: FeedConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, cfg.id);
        assert_eq!(back.source_type, FeedSourceType::Gps);
        assert_eq!(back.poll_interval_secs, 10);
    }

    #[test]
    fn feed_source_type_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&FeedSourceType::Iot).unwrap(),
            "\"iot\""
        );
        assert_eq!(
            serde_json::to_string(&FeedSourceType::Http).unwrap(),
            "\"http\""
        );
    }

    // -- validate_record -----------------------------------------------------

    #[test]
    fn valid_gps_record_passes() {
        let cfg = gps_config();
        let rec = example_gps_record("gps-test", 37.7749, -122.4194, 60.0);
        let result = validate_record(&rec, &cfg);
        assert!(result.valid, "errors: {:?}", result.errors);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn valid_iot_record_passes() {
        let cfg = iot_config();
        let rec = example_iot_record("iot-test", "sensor-1", 22.5, 55.0);
        let result = validate_record(&rec, &cfg);
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn missing_required_field_fails() {
        let cfg = gps_config();
        // Build a record without 'speed_kmh'
        let mut rec = example_gps_record("gps-test", 37.7749, -122.4194, 0.0);
        rec.fields.remove("speed_kmh");
        let result = validate_record(&rec, &cfg);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("speed_kmh")));
    }

    #[test]
    fn out_of_range_latitude_fails() {
        let cfg = gps_config();
        let mut rec = example_gps_record("gps-test", 95.0, -122.4194, 60.0); // lat > 90
        rec.fields
            .insert("latitude".to_string(), serde_json::json!(95.0));
        let result = validate_record(&rec, &cfg);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("latitude")));
    }

    #[test]
    fn stale_record_fails() {
        let cfg = gps_config();
        let mut rec = example_gps_record("gps-test", 37.7749, -122.4194, 60.0);
        // Make the record 2 hours old
        rec.timestamp = Utc::now() - chrono::Duration::hours(2);
        let result = validate_record(&rec, &cfg);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("stale")));
    }

    #[test]
    fn multiple_errors_are_collected() {
        let cfg = gps_config();
        let mut rec = example_gps_record("gps-test", 200.0, 200.0, 500.0); // lat, lon, speed all bad
        rec.fields
            .insert("latitude".to_string(), serde_json::json!(200.0));
        rec.fields
            .insert("longitude".to_string(), serde_json::json!(200.0));
        rec.fields
            .insert("speed_kmh".to_string(), serde_json::json!(500.0));
        let result = validate_record(&rec, &cfg);
        assert!(!result.valid);
        assert!(
            result.errors.len() >= 3,
            "expected ≥3 errors, got: {:?}",
            result.errors
        );
    }

    // -- FeedMetrics ---------------------------------------------------------

    #[test]
    fn new_metrics_start_as_down() {
        let m = FeedMetrics::new("feed-1".to_string());
        assert_eq!(m.health, FeedHealth::Down);
        assert_eq!(m.success_rate(), 0.0);
    }

    #[test]
    fn healthy_after_valid_records() {
        let mut m = FeedMetrics::new("feed-1".to_string());
        for _ in 0..20 {
            m.record_received(true);
        }
        assert_eq!(m.health, FeedHealth::Healthy);
        assert!((m.success_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn degraded_at_moderate_error_rate() {
        let mut m = FeedMetrics::new("feed-1".to_string());
        // 10% error rate → Degraded
        for _ in 0..9 {
            m.record_received(true);
        }
        m.record_received(false);
        assert_eq!(m.health, FeedHealth::Degraded);
    }

    #[test]
    fn down_at_high_error_rate() {
        let mut m = FeedMetrics::new("feed-1".to_string());
        // 50% error rate → Down
        for _ in 0..5 {
            m.record_received(true);
            m.record_received(false);
        }
        assert_eq!(m.health, FeedHealth::Down);
    }

    #[test]
    fn metrics_counts_are_accurate() {
        let mut m = FeedMetrics::new("feed-x".to_string());
        m.record_received(true);
        m.record_received(true);
        m.record_received(false);
        assert_eq!(m.records_received, 3);
        assert_eq!(m.records_valid, 2);
        assert_eq!(m.records_invalid, 1);
        assert!(m.last_received.is_some());
    }

    // -- check_alerts --------------------------------------------------------

    #[test]
    fn no_alerts_for_healthy_feed() {
        let mut m = FeedMetrics::new("feed-ok".to_string());
        for _ in 0..10 {
            m.record_received(true);
        }
        let alerts = check_alerts(&m);
        assert!(
            alerts.is_empty(),
            "unexpected alerts: {:?}",
            alerts.iter().map(|a| &a.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn critical_alert_for_down_feed() {
        let m = FeedMetrics::new("feed-down".to_string());
        let alerts = check_alerts(&m);
        assert!(alerts.iter().any(|a| a.severity == AlertSeverity::Critical));
    }

    #[test]
    fn warning_alert_for_degraded_feed() {
        let mut m = FeedMetrics::new("feed-deg".to_string());
        for _ in 0..9 {
            m.record_received(true);
        }
        m.record_received(false);
        let alerts = check_alerts(&m);
        assert!(alerts.iter().any(|a| a.severity == AlertSeverity::Warning));
    }

    #[test]
    fn alert_severity_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&AlertSeverity::Warning).unwrap(),
            "\"warning\""
        );
        assert_eq!(
            serde_json::to_string(&AlertSeverity::Critical).unwrap(),
            "\"critical\""
        );
    }

    // -- Example generators --------------------------------------------------

    #[test]
    fn example_gps_record_has_expected_fields() {
        let rec = example_gps_record("g1", 51.5, -0.1, 80.0);
        assert_eq!(rec.feed_id, "g1");
        assert!(rec.fields.contains_key("latitude"));
        assert!(rec.fields.contains_key("longitude"));
        assert!(rec.fields.contains_key("speed_kmh"));
        assert!(rec.fields.contains_key("heading_deg"));
        assert_eq!(rec.fields["latitude"].as_f64().unwrap(), 51.5);
    }

    #[test]
    fn example_iot_record_has_expected_fields() {
        let rec = example_iot_record("i1", "s-42", 23.0, 60.0);
        assert_eq!(rec.feed_id, "i1");
        assert!(rec.fields.contains_key("sensor_id"));
        assert!(rec.fields.contains_key("temperature_c"));
        assert!(rec.fields.contains_key("humidity_pct"));
        assert_eq!(rec.fields["sensor_id"].as_str().unwrap(), "s-42");
    }

    #[test]
    fn feed_record_roundtrips_json() {
        let rec = example_gps_record("g1", 10.0, 20.0, 50.0);
        let json = serde_json::to_string(&rec).unwrap();
        let back: FeedRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.feed_id, rec.feed_id);
        assert_eq!(
            back.fields["latitude"].as_f64().unwrap(),
            rec.fields["latitude"].as_f64().unwrap()
        );
    }

    // -- ValidationResult ----------------------------------------------------

    #[test]
    fn validation_result_ok_is_valid() {
        let r = ValidationResult::ok();
        assert!(r.valid);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn validation_result_err_is_invalid() {
        let r = ValidationResult::err(vec!["oops".to_string()]);
        assert!(!r.valid);
        assert_eq!(r.errors.len(), 1);
    }
}
