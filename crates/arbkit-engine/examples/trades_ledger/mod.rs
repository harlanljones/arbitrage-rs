//! Per-trade accuracy ledger for the pipeline example.
//!
//! Lives beside `pipeline.rs` (included via `#[path]`) rather than in any
//! crate's `src/`, because ledger capture is a post-consumption, example-level
//! concern: it pairs signals already collected off the SPSC ring with their
//! simulated execution reports and serializes the pairs to JSONL. The hot path
//! never sees any of this.
//!
//! The on-disk contract (`schemaVersion` 1, `kind: "arbkit-trades"`, header
//! line plus one record per trade) is frozen in `ROADMAP-TRADE-LEDGER.md`
//! §2.2; the dashboard validates against a zod mirror of exactly these shapes.
//! Money stays integer cents and rates stay integer bps/ppm end to end — no
//! float ever touches a value that decides or reports anything.

use arbkit_core::{MarketId, OutcomeId, VenueId};
use arbkit_engine::SignalEvent;
use arbkit_sim::{
    ArbExecutionClassification, ExecutionReport, LegFillStatus, PartialFillReason, PhantomReason,
    UnfilledReason,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Schema version of the trades JSONL format. Bump on any shape change.
pub const TRADES_SCHEMA_VERSION: u32 = 1;
/// Discriminator letting consumers tell this file from other JSONL artifacts.
pub const TRADES_KIND: &str = "arbkit-trades";

/// Line 1 of the trades file. `tradeCount` must equal the number of record
/// lines that follow — the recorder refuses to publish on mismatch, so a
/// truncated write can never reach the dashboard looking complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradesHeader {
    pub schema_version: u32,
    pub kind: &'static str,
    pub run_id: String,
    pub trade_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_at_epoch_ms: Option<u128>,
}

/// One detected-and-simulated arbitrage, pessimistic numbers intact.
///
/// Every money field is integer cents and every rate field is integer
/// bps/ppm exactly as computed upstream; serialization is the only
/// transformation applied between [`ExecutionReport`] and these fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeRecord {
    /// Dense from 0 within one run.
    pub seq: u64,
    /// Detection timestamp of the triggering signal, in nanoseconds.
    pub detection_timestamp_ns: u64,
    /// Engine-measured service latency for the signal, in nanoseconds.
    pub latency_ns: u64,
    /// Human-readable market name, resolved via the match registries.
    pub market_label: String,
    /// `Signal::profit_bps`.
    pub edge_bps: u32,
    /// `Signal::overround_ppm`.
    pub overround_ppm: u32,
    /// `Signal::total_stake`.
    pub requested_stake_cents: i64,
    /// Fee-adjusted detection view of profit (`ExecutionPnl::expected_profit`).
    pub expected_profit_cents: i64,
    /// `Signal::worst_case_profit`.
    pub worst_case_profit_cents: i64,
    /// Simulated outcome (`ExecutionPnl::realized_profit`, may be negative).
    pub realized_profit_cents: i64,
    /// `ExecutionPnl::slippage` (`expected - realized`).
    pub slippage_cents: i64,
    /// `ExecutionPnl::total_fees`.
    pub fees_paid_cents: i64,
    /// `ExecutionPnl::fill_ratio_bps`.
    pub fill_ratio_bps: u32,
    /// `"clean" | "proportional" | "phantom" | "brokenLeg"`.
    pub classification: String,
    /// Whether at least one leg filled by chasing past its detected quote.
    pub chased: bool,
    /// Per-leg audit trail. Empty legs are omitted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub legs: Vec<TradeLeg>,
}

/// One leg of a [`TradeRecord`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeLeg {
    pub venue_label: String,
    pub outcome_label: String,
    pub status: LegStatusWire,
    pub requested_stake_cents: i64,
    pub filled_stake_cents: i64,
    pub net_payout_cents: i64,
}

/// Wire form of a leg's fill status, matching the dashboard's discriminated
/// union: the string `"filled"`, an object under `partiallyFilled`, or an
/// object under `unfilled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, rename_all = "camelCase")]
pub enum LegStatusWire {
    Filled(String),
    #[serde(rename_all = "camelCase")]
    PartiallyFilled {
        partially_filled: PartialFillWire,
    },
    #[serde(rename_all = "camelCase")]
    Unfilled {
        unfilled: String,
    },
}

/// Detail payload of a partially filled leg.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialFillWire {
    pub filled_cents: i64,
    pub unfilled_cents: i64,
    pub reason: String,
}

/// Resolves interned ids to human-readable labels.
///
/// Lookup misses fall back to `"market:<id>"`-style strings instead of
/// panicking: emitting the ledger must stay total even if the registries are
/// incomplete, because a failed lookup is a labeling gap, not a reason to
/// lose the run's proof of trading accuracy.
pub trait LabelResolver {
    fn market_label(&self, market_id: MarketId) -> String;
    fn venue_label(&self, venue_id: VenueId) -> String;
    fn outcome_label(&self, outcome_id: OutcomeId) -> String;
}

/// Maps an execution classification onto its frozen wire value. A phantom
/// with a broken-leg cause is its own class — unhedged directional risk is
/// the single worst outcome in the system and deserves its own badge.
pub fn classification_label(classification: &ArbExecutionClassification) -> &'static str {
    match classification {
        ArbExecutionClassification::CleanFill => "clean",
        ArbExecutionClassification::ProportionalPartialFill => "proportional",
        ArbExecutionClassification::Phantom(PhantomReason::BrokenLeg { .. }) => "brokenLeg",
        ArbExecutionClassification::Phantom(_) => "phantom",
    }
}

fn partial_reason_label(reason: PartialFillReason) -> &'static str {
    match reason {
        PartialFillReason::DepthDepleted => "depthDepleted",
        PartialFillReason::IncrementRounding => "incrementRounding",
    }
}

fn unfilled_reason_label(reason: UnfilledReason) -> &'static str {
    match reason {
        UnfilledReason::PriceMoved { .. } => "priceMoved",
        UnfilledReason::DepthExhausted { .. } => "depthExhausted",
        UnfilledReason::BookStale => "bookStale",
        UnfilledReason::IncrementConstraint => "incrementConstraint",
    }
}

fn leg_status_wire(status: LegFillStatus) -> LegStatusWire {
    match status {
        LegFillStatus::Filled => LegStatusWire::Filled(String::from("filled")),
        LegFillStatus::PartiallyFilled {
            filled_stake,
            unfilled_stake,
            reason,
        } => LegStatusWire::PartiallyFilled {
            partially_filled: PartialFillWire {
                filled_cents: filled_stake,
                unfilled_cents: unfilled_stake,
                reason: String::from(partial_reason_label(reason)),
            },
        },
        LegFillStatus::Unfilled(reason) => LegStatusWire::Unfilled {
            unfilled: String::from(unfilled_reason_label(reason)),
        },
    }
}

/// Pairs one collected signal event with its execution report into a
/// [`TradeRecord`]. Pure: every cents/bps field is copied verbatim from the
/// inputs, which is what the property test below pins down.
pub fn build_trade_record(
    seq: u64,
    signal_event: &SignalEvent,
    report: &ExecutionReport,
    labels: &impl LabelResolver,
) -> TradeRecord {
    let legs = report
        .leg_results()
        .iter()
        // Zero-stake legs carry no information; omit them per §2.2.
        .filter(|leg| leg.requested_stake != 0)
        .map(|leg| TradeLeg {
            venue_label: labels.venue_label(leg.venue),
            outcome_label: labels.outcome_label(leg.outcome),
            status: leg_status_wire(leg.status),
            requested_stake_cents: leg.requested_stake,
            filled_stake_cents: leg.filled_stake,
            net_payout_cents: leg.net_payout,
        })
        .collect();

    TradeRecord {
        seq,
        detection_timestamp_ns: signal_event.ingest_timestamp_ns,
        latency_ns: signal_event.latency_ns,
        market_label: labels.market_label(signal_event.market_id),
        edge_bps: signal_event.signal.profit_bps,
        overround_ppm: signal_event.signal.overround_ppm,
        requested_stake_cents: signal_event.signal.total_stake,
        expected_profit_cents: report.pnl.expected_profit,
        worst_case_profit_cents: signal_event.signal.worst_case_profit,
        realized_profit_cents: report.pnl.realized_profit,
        slippage_cents: report.pnl.slippage,
        fees_paid_cents: report.pnl.total_fees,
        fill_ratio_bps: report.pnl.fill_ratio_bps,
        classification: String::from(classification_label(&report.classification)),
        chased: report.chased,
        legs,
    }
}

/// Writes the header line followed by one compact JSON record per trade,
/// atomically via a temporary file. Header `trade_count` is asserted equal
/// to `records.len()` here so drift inside this process is impossible.
pub fn write_trades_file(
    path: &Path,
    header: &TradesHeader,
    records: &[TradeRecord],
) -> Result<(), String> {
    assert_eq!(
        header.trade_count,
        records.len(),
        "header trade count must match record count"
    );

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }

    let mut out = String::new();
    out.push_str(
        &serde_json::to_string(header)
            .map_err(|error| format!("could not serialize trades header: {error}"))?,
    );
    out.push('\n');
    for record in records {
        out.push_str(
            &serde_json::to_string(record)
                .map_err(|error| format!("could not serialize trade {}: {error}", record.seq))?,
        );
        out.push('\n');
    }

    let temporary = path.with_extension("jsonl.tmp");
    fs::write(&temporary, out)
        .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("could not publish {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbkit_core::{Allocation, Fee, Leg, Prob, Signal};
    use arbkit_sim::LatencyModel;
    use std::collections::HashMap;

    struct TestLabels(HashMap<u32, String>);

    impl LabelResolver for TestLabels {
        fn market_label(&self, _market_id: MarketId) -> String {
            "BOS @ LAL moneyline".to_string()
        }
        fn venue_label(&self, venue_id: VenueId) -> String {
            self.0
                .get(&(u32::from(venue_id)))
                .cloned()
                .unwrap_or(format!("venue:{venue_id}"))
        }
        fn outcome_label(&self, outcome_id: OutcomeId) -> String {
            self.0
                .get(&outcome_id)
                .cloned()
                .unwrap_or(format!("outcome:{outcome_id}"))
        }
    }

    fn sample_signal() -> Signal {
        let allocations = [
            Allocation {
                leg: 0,
                stake: 48_000,
                payout: 100_000,
            },
            Allocation {
                leg: 1,
                stake: 51_000,
                payout: 100_000,
            },
            Allocation {
                leg: 0,
                stake: 0,
                payout: 0,
            },
            Allocation {
                leg: 0,
                stake: 0,
                payout: 0,
            },
        ];
        Signal::from_raw_parts(allocations, 2, 960_000, 99_000, 1_000, 101)
    }

    /// A latency model with no queue front-running, so arrival depths are
    /// exactly the depths the fixture passes in and fills stay predictable.
    fn zero_latency() -> LatencyModel {
        LatencyModel::new(arbkit_sim::LatencyProfile {
            wire_delay_ns: 0,
            venue_processing_ns: 0,
            queue_front_run_bps: 0,
        })
    }

    /// Produces a real `ExecutionReport` through the public simulator API —
    /// the struct's fill slots are private outside `arbkit-sim`, and going
    /// through `simulate_with_quotes` keeps the fixture honest.
    fn simulate_report() -> ExecutionReport {
        let mut sim = arbkit_sim::Simulator::new(zero_latency());
        let legs = [
            Leg {
                venue: 1,
                outcome: 10,
                quoted: Prob::from_cents(48).unwrap(),
                fee: Fee::None,
                capacity: 80_000,
                increment: 1,
            },
            Leg {
                venue: 0,
                outcome: 11,
                quoted: Prob::from_cents(46).unwrap(),
                fee: Fee::StakeFeeBps(350),
                capacity: 50_000,
                increment: 100,
            },
        ];
        // Arrival prices improve on both quotes (smaller Prob is better), so
        // every simulated execution here fills fully and cleanly.
        let arrival_prices = [
            Some(Prob::from_cents(46).unwrap()),
            Some(Prob::from_cents(44).unwrap()),
        ];
        let arrival_depths = [80_000, 60_000];
        sim.simulate_with_quotes(
            1_000,
            &sample_signal(),
            &legs,
            &arrival_prices,
            &arrival_depths,
        )
        .expect("simulation succeeds")
    }

    fn sample_signal_event() -> SignalEvent {
        SignalEvent {
            market_id: 0,
            signal: sample_signal(),
            ingest_timestamp_ns: 5_000,
            signal_timestamp_ns: 5_250,
            latency_ns: 250,
        }
    }

    #[test]
    fn emitted_fields_equal_inputs_exactly() {
        let report = simulate_report();
        let labels = TestLabels(HashMap::from([
            (10u32, "Los Angeles Lakers".to_string()),
            (11u32, "Boston Celtics".to_string()),
        ]));
        let record = build_trade_record(7, &sample_signal_event(), &report, &labels);

        assert_eq!(record.seq, 7);
        assert_eq!(record.detection_timestamp_ns, 5_000);
        assert_eq!(record.latency_ns, 250);
        assert_eq!(record.edge_bps, 101);
        assert_eq!(record.overround_ppm, 960_000);
        assert_eq!(record.requested_stake_cents, 99_000);
        assert_eq!(record.worst_case_profit_cents, 1_000);
        assert_eq!(record.expected_profit_cents, report.pnl.expected_profit);
        assert_eq!(record.realized_profit_cents, report.pnl.realized_profit);
        assert_eq!(record.slippage_cents, report.pnl.slippage);
        assert_eq!(record.fees_paid_cents, report.pnl.total_fees);
        assert_eq!(record.fill_ratio_bps, report.pnl.fill_ratio_bps);
        assert_eq!(record.classification, "clean");
        assert!(!record.chased);
        assert_eq!(record.legs.len(), 2);
        assert_eq!(record.legs[0].venue_label, "venue:1");
        assert_eq!(record.legs[0].outcome_label, "Los Angeles Lakers");
        assert_eq!(
            record.legs[0].status,
            LegStatusWire::Filled(String::from("filled"))
        );
        assert_eq!(record.legs[0].requested_stake_cents, 48_000);
    }

    #[test]
    fn label_lookup_misses_fall_back_without_panicking() {
        let report = simulate_report();
        let record = build_trade_record(
            0,
            &sample_signal_event(),
            &report,
            &TestLabels(HashMap::new()),
        );
        assert_eq!(record.legs[0].outcome_label, "outcome:10");
        assert_eq!(record.legs[0].venue_label, "venue:1");
    }

    #[test]
    fn jsonl_round_trip_preserves_every_field() {
        let report = simulate_report();
        let record = build_trade_record(
            3,
            &sample_signal_event(),
            &report,
            &TestLabels(HashMap::from([
                (10u32, "Los Angeles Lakers".to_string()),
                (11u32, "Boston Celtics".to_string()),
            ])),
        );

        let line = serde_json::to_string(&record).expect("serialize");
        let parsed: TradeRecord = serde_json::from_str(&line).expect("parse");
        assert_eq!(parsed, record);

        let value: serde_json::Value = serde_json::from_str(&line).expect("json value");
        let obj = value.as_object().expect("object");
        assert_eq!(obj["seq"], serde_json::json!(3));
        assert_eq!(obj["edgeBps"], serde_json::json!(101));
        assert_eq!(obj["classification"], serde_json::json!("clean"));
        let legs = obj["legs"].as_array().expect("legs array");
        assert_eq!(legs[0]["status"], serde_json::json!("filled"));
    }

    #[test]
    fn partial_fill_serializes_its_variant() {
        // Arrival depth below the requested stake produces a proportional
        // partial fill on both legs — the hedge-preserving middle case.
        let mut sim = arbkit_sim::Simulator::new(zero_latency());
        let legs = [
            Leg {
                venue: 1,
                outcome: 10,
                quoted: Prob::from_cents(48).unwrap(),
                fee: Fee::None,
                capacity: 80_000,
                increment: 1,
            },
            Leg {
                venue: 0,
                outcome: 11,
                quoted: Prob::from_cents(46).unwrap(),
                fee: Fee::StakeFeeBps(350),
                capacity: 50_000,
                increment: 100,
            },
        ];
        let arrival_prices = [
            Some(Prob::from_cents(46).unwrap()),
            Some(Prob::from_cents(44).unwrap()),
        ];
        let arrival_depths = [47_000, 50_400];
        let report = sim
            .simulate_with_quotes(
                1_000,
                &sample_signal(),
                &legs,
                &arrival_prices,
                &arrival_depths,
            )
            .expect("simulation succeeds");

        let record = build_trade_record(
            0,
            &sample_signal_event(),
            &report,
            &TestLabels(HashMap::new()),
        );
        let line = serde_json::to_string(&record).expect("serialize");
        let parsed: TradeRecord = serde_json::from_str(&line).expect("parse");
        assert_eq!(parsed, record);

        let value: serde_json::Value = serde_json::from_str(&line).expect("json value");
        for leg in value["legs"].as_array().expect("legs array") {
            let partial = &leg["status"]["partiallyFilled"];
            assert!(
                partial.is_object(),
                "expected partiallyFilled status, got {leg}"
            );
            assert!(partial["reason"].is_string());
        }
        assert_eq!(record.classification, "proportional");
    }

    #[test]
    fn written_header_count_matches_record_lines() {
        let report = simulate_report();
        let records: Vec<TradeRecord> = (0..4)
            .map(|seq| {
                build_trade_record(
                    seq,
                    &sample_signal_event(),
                    &report,
                    &TestLabels(HashMap::new()),
                )
            })
            .collect();
        let header = TradesHeader {
            schema_version: TRADES_SCHEMA_VERSION,
            kind: TRADES_KIND,
            run_id: "test-run".to_string(),
            trade_count: records.len(),
            recorded_at_epoch_ms: Some(1_700_000_000_000),
        };

        let dir = std::env::temp_dir().join(format!("arbkit-trades-test-{}", std::process::id()));
        let path = dir.join("run.trades.jsonl");
        write_trades_file(&path, &header, &records).expect("write succeeds");

        let contents = fs::read_to_string(&path).expect("read back");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), records.len() + 1);

        let parsed_header: serde_json::Value = serde_json::from_str(lines[0]).expect("header json");
        assert_eq!(parsed_header["kind"], serde_json::json!("arbkit-trades"));
        assert_eq!(parsed_header["tradeCount"], serde_json::json!(4));
        for line in &lines[1..] {
            let _: TradeRecord = serde_json::from_str(line).expect("record parses");
        }

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }
}
