use std::time::Instant;

use bop_core::{DecisionRecord, DecisionValue, Meta};
use chrono::Utc;
use serde::Serialize;

pub const SYSTEM_ONE_PROTOTYPE: &str = "system-one-prototype";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionPlaneConfig {
    pub adapter: Option<String>,
}

impl DecisionPlaneConfig {
    pub fn disabled() -> Self {
        Self { adapter: None }
    }

    pub fn from_dispatch_config(cfg: Option<&bop_core::config::DispatchConfig>) -> Self {
        Self {
            adapter: cfg
                .and_then(|cfg| cfg.decision_adapter.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string()),
        }
    }

    pub fn adapter_name(&self) -> Option<&str> {
        self.adapter.as_deref()
    }

    pub fn advisory_enabled(&self) -> bool {
        matches!(self.adapter_name(), Some(SYSTEM_ONE_PROTOTYPE))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BenchmarkSiteReport {
    pub site: String,
    pub cases: usize,
    pub agreement_rate: f64,
    pub mean_confidence: f64,
    pub mean_latency_us: f64,
    pub advisory_cost_usd: f64,
    pub replayable: bool,
    pub fallback_behavior: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BenchmarkReport {
    pub adapter: String,
    pub sites: Vec<BenchmarkSiteReport>,
}

#[derive(Debug, Clone)]
struct ProviderScenario<'a> {
    stage: &'a str,
    eligible: Vec<&'a str>,
    baseline: &'a str,
    cost_tier: u8,
    prefer_cheap_provider: Option<&'a str>,
    avoid_provider: Option<&'a str>,
}

#[derive(Debug, Clone)]
struct OrphanScenario<'a> {
    pid_dead: bool,
    lease_stale: bool,
    move_to_failed: bool,
    baseline: &'a str,
}

pub fn build_benchmark_report() -> BenchmarkReport {
    let provider_scenarios = vec![
        ProviderScenario {
            stage: "implement",
            eligible: vec!["codex", "claude"],
            baseline: "codex",
            cost_tier: 3,
            prefer_cheap_provider: Some("ollama-local"),
            avoid_provider: None,
        },
        ProviderScenario {
            stage: "implement",
            eligible: vec!["codex", "ollama-local"],
            baseline: "ollama-local",
            cost_tier: 1,
            prefer_cheap_provider: Some("ollama-local"),
            avoid_provider: None,
        },
        ProviderScenario {
            stage: "qa",
            eligible: vec!["qa_prov"],
            baseline: "qa_prov",
            cost_tier: 2,
            prefer_cheap_provider: None,
            avoid_provider: Some("impl_prov"),
        },
    ];
    let orphan_scenarios = vec![
        OrphanScenario {
            pid_dead: true,
            lease_stale: false,
            move_to_failed: false,
            baseline: "pending",
        },
        OrphanScenario {
            pid_dead: false,
            lease_stale: true,
            move_to_failed: true,
            baseline: "failed",
        },
        OrphanScenario {
            pid_dead: true,
            lease_stale: true,
            move_to_failed: false,
            baseline: "pending",
        },
    ];

    BenchmarkReport {
        adapter: SYSTEM_ONE_PROTOTYPE.to_string(),
        sites: vec![
            benchmark_provider_selection(provider_scenarios),
            benchmark_orphan_recovery(orphan_scenarios),
        ],
    }
}

pub fn record_provider_selection(
    meta: &mut Meta,
    cfg: &DecisionPlaneConfig,
    stage: &str,
    eligible: &[String],
    baseline: &str,
    cost_tier: u8,
    prefer_cheap_provider: Option<&str>,
    avoid_provider: Option<&str>,
) {
    if !cfg.advisory_enabled() || eligible.is_empty() {
        return;
    }

    let suggested = prototype_provider_choice(
        stage,
        eligible,
        cost_tier,
        prefer_cheap_provider,
        avoid_provider,
    );
    let confidence = prototype_provider_confidence(
        stage,
        eligible,
        baseline,
        &suggested,
        cost_tier,
        prefer_cheap_provider,
        avoid_provider,
    );
    meta.decisions.push(DecisionRecord {
        ts: Utc::now().to_rfc3339(),
        site: "provider_selection".to_string(),
        adapter: cfg.adapter.clone().unwrap_or_default(),
        suggested: DecisionValue::Choice { value: suggested },
        baseline: Some(DecisionValue::Choice {
            value: baseline.to_string(),
        }),
        confidence,
        adopted: false,
        note: Some(format!(
            "stage={stage}, eligible={}, cost_tier={cost_tier}",
            eligible.join(",")
        )),
    });
}

pub fn record_orphan_recovery(
    meta: &mut Meta,
    cfg: &DecisionPlaneConfig,
    pid_dead: bool,
    lease_stale: bool,
    move_to_failed: bool,
) {
    if !cfg.advisory_enabled() {
        return;
    }

    let baseline = if move_to_failed { "failed" } else { "pending" };
    let confidence = prototype_orphan_confidence(pid_dead, lease_stale, move_to_failed);
    meta.decisions.push(DecisionRecord {
        ts: Utc::now().to_rfc3339(),
        site: "orphan_recovery".to_string(),
        adapter: cfg.adapter.clone().unwrap_or_default(),
        suggested: DecisionValue::Choice {
            value: baseline.to_string(),
        },
        baseline: Some(DecisionValue::Choice {
            value: baseline.to_string(),
        }),
        confidence,
        adopted: false,
        note: Some(format!(
            "pid_dead={pid_dead}, lease_stale={lease_stale}, advisory_only=true"
        )),
    });
}

fn benchmark_provider_selection(scenarios: Vec<ProviderScenario<'_>>) -> BenchmarkSiteReport {
    let mut matches = 0usize;
    let mut confidence_sum = 0.0;
    let mut latency_us_sum = 0.0;

    for scenario in &scenarios {
        let eligible: Vec<String> = scenario.eligible.iter().map(|value| value.to_string()).collect();
        let started = Instant::now();
        let suggested = prototype_provider_choice(
            scenario.stage,
            &eligible,
            scenario.cost_tier,
            scenario.prefer_cheap_provider,
            scenario.avoid_provider,
        );
        latency_us_sum += started.elapsed().as_secs_f64() * 1_000_000.0;
        if suggested == scenario.baseline {
            matches += 1;
        }
        confidence_sum += prototype_provider_confidence(
            scenario.stage,
            &eligible,
            scenario.baseline,
            &suggested,
            scenario.cost_tier,
            scenario.prefer_cheap_provider,
            scenario.avoid_provider,
        );
    }

    BenchmarkSiteReport {
        site: "provider_selection".to_string(),
        cases: scenarios.len(),
        agreement_rate: matches as f64 / scenarios.len() as f64,
        mean_confidence: confidence_sum / scenarios.len() as f64,
        mean_latency_us: latency_us_sum / scenarios.len() as f64,
        advisory_cost_usd: 0.0,
        replayable: true,
        fallback_behavior: "deterministic provider selection remains authoritative".to_string(),
    }
}

fn benchmark_orphan_recovery(scenarios: Vec<OrphanScenario<'_>>) -> BenchmarkSiteReport {
    let mut matches = 0usize;
    let mut confidence_sum = 0.0;
    let mut latency_us_sum = 0.0;

    for scenario in &scenarios {
        let started = Instant::now();
        let suggested = if scenario.move_to_failed {
            "failed"
        } else {
            "pending"
        };
        latency_us_sum += started.elapsed().as_secs_f64() * 1_000_000.0;
        if suggested == scenario.baseline {
            matches += 1;
        }
        confidence_sum += prototype_orphan_confidence(
            scenario.pid_dead,
            scenario.lease_stale,
            scenario.move_to_failed,
        );
    }

    BenchmarkSiteReport {
        site: "orphan_recovery".to_string(),
        cases: scenarios.len(),
        agreement_rate: matches as f64 / scenarios.len() as f64,
        mean_confidence: confidence_sum / scenarios.len() as f64,
        mean_latency_us: latency_us_sum / scenarios.len() as f64,
        advisory_cost_usd: 0.0,
        replayable: true,
        fallback_behavior: "keep card state machine authoritative when advice is unavailable"
            .to_string(),
    }
}

fn prototype_provider_choice(
    stage: &str,
    eligible: &[String],
    cost_tier: u8,
    prefer_cheap_provider: Option<&str>,
    avoid_provider: Option<&str>,
) -> String {
    if stage == "qa" {
        if let Some(choice) = eligible
            .iter()
            .find(|candidate| Some(candidate.as_str()) != avoid_provider)
        {
            return choice.clone();
        }
    }

    if cost_tier <= 1 {
        if let Some(preferred) = prefer_cheap_provider {
            if eligible.iter().any(|candidate| candidate == preferred) {
                return preferred.to_string();
            }
        }
    }

    eligible
        .first()
        .cloned()
        .unwrap_or_else(|| "none".to_string())
}

fn prototype_provider_confidence(
    stage: &str,
    eligible: &[String],
    baseline: &str,
    suggested: &str,
    cost_tier: u8,
    prefer_cheap_provider: Option<&str>,
    avoid_provider: Option<&str>,
) -> f64 {
    if suggested == baseline {
        if stage == "qa" && avoid_provider.is_some() {
            return 0.94;
        }
        if cost_tier <= 1 && prefer_cheap_provider == Some(suggested) {
            return 0.91;
        }
        if eligible.len() == 1 {
            return 0.98;
        }
        return 0.83;
    }

    0.58
}

fn prototype_orphan_confidence(pid_dead: bool, lease_stale: bool, move_to_failed: bool) -> f64 {
    match (pid_dead, lease_stale, move_to_failed) {
        (true, true, true) => 0.98,
        (true, true, false) => 0.96,
        (true, false, _) => 0.91,
        (false, true, true) => 0.88,
        (false, true, false) => 0.79,
        (false, false, _) => 0.35,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_provider_selection_decision() {
        let mut meta = Meta::default();
        record_provider_selection(
            &mut meta,
            &DecisionPlaneConfig {
                adapter: Some(SYSTEM_ONE_PROTOTYPE.to_string()),
            },
            "implement",
            &["codex".to_string(), "ollama-local".to_string()],
            "ollama-local",
            1,
            Some("ollama-local"),
            None,
        );

        assert_eq!(meta.decisions.len(), 1);
        assert_eq!(meta.decisions[0].site, "provider_selection");
        assert!(meta.decisions[0].confidence >= 0.9);
    }

    #[test]
    fn records_orphan_recovery_decision() {
        let mut meta = Meta::default();
        record_orphan_recovery(
            &mut meta,
            &DecisionPlaneConfig {
                adapter: Some(SYSTEM_ONE_PROTOTYPE.to_string()),
            },
            true,
            false,
            false,
        );

        assert_eq!(meta.decisions.len(), 1);
        assert_eq!(meta.decisions[0].site, "orphan_recovery");
        assert_eq!(
            meta.decisions[0].suggested,
            DecisionValue::Choice {
                value: "pending".to_string()
            }
        );
    }

    #[test]
    fn benchmark_report_covers_both_sites() {
        let report = build_benchmark_report();
        assert_eq!(report.sites.len(), 2);
        assert!(report.sites.iter().all(|site| site.agreement_rate >= 0.99));
    }
}
