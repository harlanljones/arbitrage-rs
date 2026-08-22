//! End-to-end pipeline benchmark and analysis runner.
//!
//! Simulates high-throughput market feeds across Kalshi, Polymarket, and Pinnacle,
//! routes them through the zero-allocation hot loop engine, measures sub-microsecond
//! latency distributions, and evaluates order executions through the paper trading simulator.

use arbkit_core::{Fee, Leg, Level, MarketKind, Prob};
use arbkit_engine::{spsc_ring, Engine, FeedEventSlot, MarketConfig, SignalEvent, SignalEventSlot};
use arbkit_feed::{FeedEvent, TradeSide};
use arbkit_match::team::{parse_matchup, Sport};
use arbkit_match::{CanonicalRegistry, VenueRegistry};
use arbkit_sim::{ExecutionReport, LatencyModel, LatencyProfile, Simulator};
use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "trades_ledger/mod.rs"]
mod trades_ledger;

use trades_ledger::{
    build_trade_record, write_trades_file, LabelResolver, TradeRecord, TradesHeader, TRADES_KIND,
    TRADES_SCHEMA_VERSION,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PipelineReport {
    schema_version: u32,
    run: RunMetadata,
    environment: EnvironmentMetadata,
    workload: WorkloadMetadata,
    performance: PerformanceMetrics,
    detection: DetectionMetrics,
    simulation: SimulationMetrics,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunMetadata {
    id: String,
    recorded_at_epoch_ms: u128,
    source: &'static str,
    project_version: &'static str,
    git_commit: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvironmentMetadata {
    label: String,
    os: &'static str,
    arch: &'static str,
    rustc: Option<String>,
    build_profile: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkloadMetadata {
    synthetic: bool,
    paper_trading: bool,
    feed_events: usize,
    event: &'static str,
    market: &'static str,
    venues: [&'static str; 3],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PerformanceMetrics {
    elapsed_ms: f64,
    throughput_per_second: f64,
    target_p99_ns: u64,
    latency_ns: LatencyMetrics,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LatencyMetrics {
    count: u64,
    min: u64,
    mean: u64,
    p50: u64,
    p90: u64,
    p99: u64,
    p999: u64,
    max: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DetectionMetrics {
    events_processed: u64,
    signals_emitted: u64,
    collected_signals: usize,
    sample: Option<SampleSignal>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SampleSignal {
    profit_bps: u32,
    total_stake_cents: i64,
    guaranteed_profit_cents: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SimulationMetrics {
    total_signals: u64,
    clean_fills: u64,
    proportional_fills: u64,
    phantoms: u64,
    phantom_rate_bps: u32,
    filled_stake_cents: i64,
    fees_paid_cents: i64,
    realized_profit_cents: i64,
    realized_roi_bps: i64,
}

struct PipelineArgs {
    json: Option<PathBuf>,
    trades: Option<PathBuf>,
    ticks: usize,
}

fn parse_args() -> PipelineArgs {
    const DEFAULT_TICKS: usize = 2_000_000;
    let mut args = env::args().skip(1);
    let mut output = None;
    let mut trades = None;
    let mut ticks = DEFAULT_TICKS;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => {
                let path = args.next().unwrap_or_else(|| {
                    eprintln!("error: --json requires an output path");
                    std::process::exit(2);
                });
                output = Some(PathBuf::from(path));
            }
            "--trades" => {
                let path = args.next().unwrap_or_else(|| {
                    eprintln!("error: --trades requires an output path");
                    std::process::exit(2);
                });
                trades = Some(PathBuf::from(path));
            }
            "--ticks" => {
                let raw = args.next().unwrap_or_else(|| {
                    eprintln!("error: --ticks requires a count");
                    std::process::exit(2);
                });
                ticks = match raw.parse::<usize>() {
                    Ok(count) if count > 0 => count,
                    Ok(_) => {
                        eprintln!("error: --ticks must be greater than zero");
                        std::process::exit(2);
                    }
                    Err(error) => {
                        eprintln!("error: --ticks is not a valid count ({error})");
                        std::process::exit(2);
                    }
                };
            }
            "--help" | "-h" => {
                println!("Usage: pipeline [--ticks <n>] [--json <path>] [--trades <path>]");
                println!("Streams {DEFAULT_TICKS} synthetic feed events through the pipeline by default;");
                println!(
                    "--ticks overrides the count. Optionally writes a schema-versioned JSON report"
                );
                println!("and a per-trade accuracy ledger (JSONL). The trades path defaults to a");
                println!(
                    "sibling of the --json path with a .trades.jsonl suffix, else trades.jsonl."
                );
                std::process::exit(0);
            }
            _ => {
                eprintln!("error: unknown argument {arg:?}");
                std::process::exit(2);
            }
        }
    }
    PipelineArgs {
        json: output,
        trades,
        ticks,
    }
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    Some(value.trim().to_owned()).filter(|value| !value.is_empty())
}

/// Resolves interned engine ids to human-readable labels through the match
/// registries built at startup. Lookup misses fall back to `"market:<id>"`-
/// style strings rather than panicking — ledger emission must stay total.
struct PipelineLabels<'a> {
    registry: &'a CanonicalRegistry,
    venues: &'a VenueRegistry,
}

impl LabelResolver for PipelineLabels<'_> {
    fn market_label(&self, market_id: u32) -> String {
        let Some(market) = self.registry.get_market(market_id) else {
            return format!("market:{market_id}");
        };
        let kind = match market.kind {
            MarketKind::Moneyline => "moneyline".to_string(),
            MarketKind::Spread(line) => format!("spread {line:?}"),
            MarketKind::Total(line) => format!("total {line:?}"),
        };
        match self.registry.get_event(market.event_id) {
            Some(event) => format!("{} · {}", event.name, kind),
            None => kind,
        }
    }

    fn venue_label(&self, venue_id: u16) -> String {
        self.venues
            .name_of(venue_id)
            .map(str::to_string)
            .unwrap_or_else(|| format!("venue:{venue_id}"))
    }

    fn outcome_label(&self, outcome_id: u32) -> String {
        self.registry
            .get_outcome(outcome_id)
            .map(|outcome| outcome.name.clone())
            .unwrap_or_else(|| format!("outcome:{outcome_id}"))
    }
}

fn write_report(path: &Path, report: &PipelineReport) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("could not serialize report: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, format!("{json}\n"))
        .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("could not publish {}: {error}", path.display()))
}

fn main() {
    let PipelineArgs {
        json: json_path,
        trades: trades_flag,
        ticks: num_ticks,
    } = parse_args();
    println!("========================================================================");
    println!("  arbkit: End-to-End Pipeline Execution & Analysis");
    println!("========================================================================");
    println!();

    // 1. Setup Canonical Registry
    let mut registry = CanonicalRegistry::new();
    let matchup = parse_matchup("BOS @ LAL", Some(Sport::Nba)).expect("parse matchup");
    let event_id = registry.create_event(
        "Boston Celtics @ Los Angeles Lakers",
        Sport::Nba,
        matchup.home,
        matchup.away,
        Some("26OCT25"),
    );

    let (moneyline_market, outcome_lal, outcome_bos) = registry
        .create_moneyline_market(event_id)
        .expect("create moneyline market");

    println!("✓ Canonical Registry initialized:");
    println!(
        "  - Event: {} @ {}",
        matchup.away.full_name, matchup.home.full_name
    );
    println!("  - Market: Moneyline (ID: {moneyline_market})");
    println!("  - Outcomes: BOS (ID: {outcome_bos}), LAL (ID: {outcome_lal})");
    println!(
        "  - Venues: Kalshi ({}), Polymarket ({}), Pinnacle ({})",
        VenueRegistry::KALSHI,
        VenueRegistry::POLYMARKET,
        VenueRegistry::PINNACLE
    );
    println!();

    // 2. Setup Engine & Market Configurations
    let mut engine = Engine::new(16);
    let mut config = MarketConfig {
        outcome_count: 2,
        active: true,
        ..Default::default()
    };

    // Venue 0: Kalshi (continuous stake fee, ~$3.50/100c at even money = 350 bps)
    config.venue_fees[VenueRegistry::KALSHI as usize] = Fee::StakeFeeBps(350);
    config.venue_increments[VenueRegistry::KALSHI as usize] = 100;

    // Venue 1: Polymarket (0 bps maker/taker fee in CLOB)
    config.venue_fees[VenueRegistry::POLYMARKET as usize] = Fee::None;
    config.venue_increments[VenueRegistry::POLYMARKET as usize] = 1;

    // Venue 2: Pinnacle (100 bps amortized fee)
    config.venue_fees[VenueRegistry::PINNACLE as usize] = Fee::StakeFeeBps(100);
    config.venue_increments[VenueRegistry::PINNACLE as usize] = 100;

    engine
        .register_market(moneyline_market, config)
        .expect("register market");

    // 3. Setup SPSC Rings
    const RING_CAPACITY: usize = 8192;
    let (feed_prod, mut feed_cons) = spsc_ring::<FeedEventSlot>(RING_CAPACITY);
    let (mut signal_prod, mut signal_cons) = spsc_ring::<SignalEventSlot>(RING_CAPACITY);

    let engine_running = Arc::new(AtomicBool::new(true));
    let engine_running_flag = engine_running.clone();

    let start_instant = Instant::now();
    let engine_start_instant = start_instant;

    let mut service_histogram = arbkit_engine::LatencyHistogram::new();
    let (service_tx, service_rx) = std::sync::mpsc::channel();

    // 4. Spawn Engine Hot Loop on a dedicated thread
    //
    // `service_histogram` measures pure hot-loop *compute* time (pop-to-done,
    // this thread's own clock) for every processed event, signal or not.
    // This is distinct from `engine.histogram()`, which times ingest-to-emit
    // (the event's own embedded timestamp to processing time) and so also
    // captures ring-queue dwell time under burst load — a real but different
    // metric from the p99-compute-time budget this section reports against.
    let engine_thread = thread::Builder::new()
        .name("hot-engine-loop".into())
        .spawn(move || {
            while engine_running_flag.load(Ordering::Relaxed) {
                if let Some(event) = feed_cons.try_pop() {
                    let start_ns = engine_start_instant.elapsed().as_nanos() as u64;
                    let signal = engine.process_event(&event, start_ns);
                    let end_ns = engine_start_instant.elapsed().as_nanos() as u64;
                    service_histogram.record(end_ns.saturating_sub(start_ns));
                    if let Some(signal_event) = signal {
                        let _ = signal_prod.try_push(signal_event);
                    }
                }
            }

            // Drain remaining ingress queue
            while let Some(event) = feed_cons.try_pop() {
                let start_ns = engine_start_instant.elapsed().as_nanos() as u64;
                let signal = engine.process_event(&event, start_ns);
                let end_ns = engine_start_instant.elapsed().as_nanos() as u64;
                service_histogram.record(end_ns.saturating_sub(start_ns));
                if let Some(signal_event) = signal {
                    let _ = signal_prod.try_push(signal_event);
                }
            }

            let service_summary = service_histogram.summary();
            let stats = *engine.stats();
            service_tx.send(service_summary).unwrap();
            stats
        })
        .expect("spawn engine thread");

    // 5. Setup Paper Trading Simulator on downstream
    let default_profile = LatencyProfile {
        wire_delay_ns: 10_000_000,
        venue_processing_ns: 2_000_000,
        queue_front_run_bps: 500,
    };
    let mut latency_model = LatencyModel::new(default_profile);
    latency_model.set_venue_profile(
        VenueRegistry::KALSHI,
        LatencyProfile {
            wire_delay_ns: 8_000_000,       // 8ms
            venue_processing_ns: 2_000_000, // 2ms
            queue_front_run_bps: 500,       // 5% queue front-running
        },
    );
    latency_model.set_venue_profile(
        VenueRegistry::POLYMARKET,
        LatencyProfile {
            wire_delay_ns: 12_000_000,      // 12ms
            venue_processing_ns: 3_000_000, // 3ms
            queue_front_run_bps: 1000,      // 10% queue front-running
        },
    );

    let mut simulator = Simulator::new(latency_model);

    // 6. Generate Synthetic Market Stream
    println!("Streaming {num_ticks} market feed events through the pipeline...");
    let start_time = Instant::now();

    let mut signal_collector = Vec::with_capacity(5000);
    let mut seq_kalshi_bos = 1u64;
    let mut seq_kalshi_lal = 1u64;
    let mut seq_poly_bos = 1u64;
    let mut seq_poly_lal = 1u64;

    let mut feed_producer = feed_prod;

    let clock_now = move || start_instant.elapsed().as_nanos() as u64;

    // Helper macro to push feed events
    let make_snap = |venue_id, outcome_id, price_cents, size, seq| FeedEvent::Snapshot {
        market_id: moneyline_market,
        outcome_id,
        venue_id,
        levels: [
            Level {
                price: Prob::from_cents(price_cents).unwrap(),
                size,
            },
            Level {
                price: Prob::from_cents((price_cents + 1).min(99)).unwrap(),
                size: size * 2,
            },
            Level {
                price: Prob::CERTAIN,
                size: 0,
            },
            Level {
                price: Prob::CERTAIN,
                size: 0,
            },
            Level {
                price: Prob::CERTAIN,
                size: 0,
            },
            Level {
                price: Prob::CERTAIN,
                size: 0,
            },
            Level {
                price: Prob::CERTAIN,
                size: 0,
            },
            Level {
                price: Prob::CERTAIN,
                size: 0,
            },
        ],
        num_levels: 2,
        seq,
        timestamp_ns: clock_now(),
    };

    // Initial snapshots to initialize all 4 books
    while feed_producer
        .try_push(make_snap(
            VenueRegistry::KALSHI,
            outcome_bos,
            49,
            100_000,
            seq_kalshi_bos,
        ))
        .is_err()
    {
        thread::yield_now();
    }
    while feed_producer
        .try_push(make_snap(
            VenueRegistry::KALSHI,
            outcome_lal,
            53,
            100_000,
            seq_kalshi_lal,
        ))
        .is_err()
    {
        thread::yield_now();
    }
    while feed_producer
        .try_push(make_snap(
            VenueRegistry::POLYMARKET,
            outcome_bos,
            51,
            100_000,
            seq_poly_bos,
        ))
        .is_err()
    {
        thread::yield_now();
    }
    while feed_producer
        .try_push(make_snap(
            VenueRegistry::POLYMARKET,
            outcome_lal,
            49,
            100_000,
            seq_poly_lal,
        ))
        .is_err()
    {
        thread::yield_now();
    }

    // Stream deltas with realistic microsecond spacing
    for i in 0..num_ticks {
        let is_kalshi = (i % 2) == 0;
        let is_bos = ((i / 2) % 2) == 0;

        let (venue_id, outcome_id, seq) = match (is_kalshi, is_bos) {
            (true, true) => {
                seq_kalshi_bos += 1;
                (VenueRegistry::KALSHI, outcome_bos, seq_kalshi_bos)
            }
            (true, false) => {
                seq_kalshi_lal += 1;
                (VenueRegistry::KALSHI, outcome_lal, seq_kalshi_lal)
            }
            (false, true) => {
                seq_poly_bos += 1;
                (VenueRegistry::POLYMARKET, outcome_bos, seq_poly_bos)
            }
            (false, false) => {
                seq_poly_lal += 1;
                (VenueRegistry::POLYMARKET, outcome_lal, seq_poly_lal)
            }
        };

        // Price fluctuation simulation:
        // Periodic fleeting arbitrage window:
        // Kalshi BOS drops to 46c (effective = 47.6c) and Polymarket LAL is 48c -> sum = 95.6c < 1.0!
        let price_cents = match i % 200 {
            0..=20 => {
                if outcome_id == outcome_bos {
                    if venue_id == VenueRegistry::KALSHI {
                        46
                    } else {
                        53
                    }
                } else {
                    if venue_id == VenueRegistry::POLYMARKET {
                        48
                    } else {
                        55
                    }
                }
            } // Lucrative Arbitrage Window (~440 bp edge)
            21..=35 => {
                if outcome_id == outcome_bos {
                    if venue_id == VenueRegistry::KALSHI {
                        48
                    } else {
                        52
                    }
                } else {
                    if venue_id == VenueRegistry::POLYMARKET {
                        50
                    } else {
                        52
                    }
                }
            } // Marginal window (fees eat edge)
            _ => {
                let shift = (i % 4) as u32;
                if outcome_id == outcome_bos {
                    50 + shift
                } else {
                    52 + shift
                }
            } // Normal vig market
        };

        let ts_ns = clock_now();
        let event = if i % 5000 == 4999 {
            FeedEvent::Trade {
                market_id: moneyline_market,
                outcome_id,
                venue_id,
                price: Prob::from_cents(price_cents.min(99)).unwrap(),
                size: 20_000,
                side: TradeSide::Buy,
                seq,
                timestamp_ns: ts_ns,
            }
        } else {
            FeedEvent::Delta {
                market_id: moneyline_market,
                outcome_id,
                venue_id,
                level: Level {
                    price: Prob::from_cents(price_cents.min(99)).unwrap(),
                    size: 50_000 + ((i % 5) as i64) * 20_000,
                },
                is_delete: false,
                seq,
                timestamp_ns: ts_ns,
            }
        };

        while feed_producer.try_push(event).is_err() {
            // Ingress ring full, drain egress ring to maintain throughput
            if let Some(signal_event) = signal_cons.try_pop() {
                signal_collector.push(signal_event);
            }
            std::hint::spin_loop();
        }

        // Periodically drain signal consumer
        if i % 16 == 0 {
            while let Some(signal_event) = signal_cons.try_pop() {
                signal_collector.push(signal_event);
            }
        }
    }

    // Allow engine to process remaining queue
    thread::sleep(Duration::from_millis(20));
    while let Some(signal_event) = signal_cons.try_pop() {
        signal_collector.push(signal_event);
    }

    // Stop engine
    engine_running.store(false, Ordering::Relaxed);
    let stats = engine_thread.join().expect("join engine thread");
    let total_duration = start_time.elapsed();

    // 7. Run Paper Trading Simulation on emitted signals
    println!("Simulating order execution against market depth & latency...");
    let mut trade_pairs: Vec<(SignalEvent, ExecutionReport)> =
        Vec::with_capacity(signal_collector.len());
    for (idx, signal_event) in signal_collector.iter().enumerate() {
        let signal: SignalEvent = *signal_event;
        let (p_lal, p_bos) = if idx % 10 == 0 {
            // Simulated quote decay / phantom scenario (price moved during transit)
            (
                Some(Prob::from_cents(51).unwrap()),
                Some(Prob::from_cents(49).unwrap()),
            )
        } else {
            // Live resting prices match quote
            (
                Some(Prob::from_cents(48).unwrap()),
                Some(Prob::from_cents(46).unwrap()),
            )
        };

        let legs = [
            Leg {
                venue: VenueRegistry::POLYMARKET,
                outcome: outcome_lal,
                quoted: Prob::from_cents(48).unwrap(),
                fee: Fee::None,
                capacity: 80_000,
                increment: 1,
            },
            Leg {
                venue: VenueRegistry::KALSHI,
                outcome: outcome_bos,
                quoted: Prob::from_cents(46).unwrap(),
                fee: Fee::StakeFeeBps(350),
                capacity: 50_000,
                increment: 100,
            },
        ];
        let arrival_prices = [p_lal, p_bos];
        let arrival_depths = [80_000, 50_000];

        if let Ok(report) = simulator.simulate_with_quotes(
            signal.ingest_timestamp_ns,
            &signal.signal,
            &legs,
            &arrival_prices,
            &arrival_depths,
        ) {
            trade_pairs.push((*signal_event, report));
        }
    }

    let sim_stats = simulator.stats();

    let service_summary = service_rx.recv().unwrap();

    // Run identity is computed once so the JSON report and the trades header
    // always agree on `runId`.
    let recorded_at_epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_millis();
    let git_commit = command_output("git", &["rev-parse", "--short", "HEAD"]);
    let run_id = format!(
        "{}-{}-{}-{}",
        recorded_at_epoch_ms,
        env::consts::OS,
        env::consts::ARCH,
        git_commit.as_deref().unwrap_or("working-tree")
    );

    // 8. Per-trade accuracy ledger
    //
    // Pure post-consumption work: pairs already-collected signals with their
    // simulation reports and serializes them. Nothing here touches the hot
    // path, and a failed write is reported to stderr without aborting — a
    // lost ledger is a reporting gap, not a reason to discard a finished run.
    let venues = VenueRegistry::new();
    let labels = PipelineLabels {
        registry: &registry,
        venues: &venues,
    };
    let trade_records: Vec<TradeRecord> = trade_pairs
        .iter()
        .enumerate()
        .map(|(seq, (signal_event, report))| {
            build_trade_record(seq as u64, signal_event, report, &labels)
        })
        .collect();

    let trades_path = trades_flag.unwrap_or_else(|| {
        json_path
            .as_deref()
            .map(|json| json.with_extension("trades.jsonl"))
            .unwrap_or_else(|| PathBuf::from("trades.jsonl"))
    });
    let trades_header = TradesHeader {
        schema_version: TRADES_SCHEMA_VERSION,
        kind: TRADES_KIND,
        run_id: run_id.clone(),
        trade_count: trade_records.len(),
        recorded_at_epoch_ms: Some(recorded_at_epoch_ms),
    };
    match write_trades_file(&trades_path, &trades_header, &trade_records) {
        Ok(()) => {
            let hit_count = trade_records
                .iter()
                .filter(|record| record.realized_profit_cents > 0)
                .count();
            let realized_total: i64 = trade_records
                .iter()
                .map(|record| record.realized_profit_cents)
                .sum();
            println!(
                "Trade Ledger Written:         {:>12} trades to {}",
                trade_records.len(),
                trades_path.display()
            );
            println!(
                "  Profitable Trades:          {:>12} · realized PnL {} cents (${:.2})",
                hit_count,
                realized_total,
                realized_total as f64 / 100.0
            );
            // Reconciliation guard: the ledger must agree with the aggregate
            // simulation section, or the proof it carries is suspect.
            if realized_total != sim_stats.total_realized_profit_cents {
                eprintln!(
                    "warning: trade ledger realized PnL ({realized_total}) does not reconcile with simulation totals ({})",
                    sim_stats.total_realized_profit_cents
                );
            }
        }
        Err(error) => {
            eprintln!("warning: could not write trade ledger: {error}");
        }
    }

    // 9. Output Detailed Results
    println!();
    println!("========================================================================");
    println!("                        PIPELINE RESULTS & ANALYSIS                     ");
    println!("========================================================================");
    println!();
    println!("  1. THROUGHPUT & CAPACITY");
    println!("  ----------------------------------------------------------------------");
    println!("  Total Feed Events Ingested:   {:>12}", num_ticks);
    println!("  Elapsed Ingestion Time:       {:>12.2?}", total_duration);
    let events_per_sec = (num_ticks as f64) / total_duration.as_secs_f64();
    println!(
        "  Burst Ingestion Throughput:   {:>12.0} msg/sec",
        events_per_sec
    );
    println!();

    println!("  2. IN-PROCESS HOT LOOP SERVICE TIME, ALL EVENTS (Budget: p99 < 50 µs)");
    println!("  ----------------------------------------------------------------------");
    println!(
        "  Events Sampled:                {:>12}",
        service_summary.count
    );
    println!(
        "  Hot Loop Latency Min:         {:>12.3} µs",
        service_summary.min_ns as f64 / 1000.0
    );
    println!(
        "  Hot Loop Latency p50:         {:>12.3} µs",
        service_summary.p50_ns as f64 / 1000.0
    );
    println!(
        "  Hot Loop Latency p90:         {:>12.3} µs",
        service_summary.p90_ns as f64 / 1000.0
    );
    println!(
        "  Hot Loop Latency p99:         {:>12.3} µs",
        service_summary.p99_ns as f64 / 1000.0
    );
    println!(
        "  Hot Loop Latency p99.9:       {:>12.3} µs",
        service_summary.p999_ns as f64 / 1000.0
    );
    println!(
        "  Hot Loop Latency Max:         {:>12.3} µs",
        service_summary.max_ns as f64 / 1000.0
    );
    println!(
        "  Hot Loop Latency Mean:        {:>12.3} µs",
        service_summary.mean_ns as f64 / 1000.0
    );
    let service_p99_us = service_summary.p99_ns as f64 / 1000.0;
    if service_p99_us < 50.0 {
        println!(
            "  >>> [PASS] In-process hot loop p99 ({:.3} µs) is well within the 50 µs budget!",
            service_p99_us
        );
    } else {
        println!(
            "  >>> [FAIL] In-process hot loop p99 ({:.3} µs) exceeds 50 µs budget!",
            service_p99_us
        );
    }
    println!();

    println!("  3. ARBITRAGE DETECTION METRICS");
    println!("  ----------------------------------------------------------------------");
    println!(
        "  Total Feed Events Processed:  {:>12}",
        stats.events_processed
    );
    println!(
        "  Valid Signals Emitted:        {:>12}",
        stats.signals_emitted
    );
    println!(
        "  Collected Signal Events:      {:>12}",
        signal_collector.len()
    );
    if let Some(first_sig) = signal_collector.first() {
        println!(
            "  Sample Signal Edge (bps):     {:>12} bps",
            first_sig.signal.profit_bps
        );
        println!(
            "  Sample Total Stake:           {:>12} cents (${:.2})",
            first_sig.signal.total_stake,
            first_sig.signal.total_stake as f64 / 100.0
        );
        println!(
            "  Sample Guaranteed Worst PnL:  {:>12} cents (${:.2})",
            first_sig.signal.worst_case_profit,
            first_sig.signal.worst_case_profit as f64 / 100.0
        );
    }
    println!();

    println!("  4. SIMULATOR & EXECUTION ACCOUNTING");
    println!("  ----------------------------------------------------------------------");
    println!(
        "  Simulated Signals:            {:>12}",
        sim_stats.total_signals
    );
    println!(
        "  Clean Fills:                  {:>12}",
        sim_stats.clean_fills
    );
    println!(
        "  Proportional/Partial Fills:   {:>12}",
        sim_stats.proportional_fills
    );
    println!(
        "  Phantom Signals:              {:>12}",
        sim_stats.total_phantoms
    );
    println!(
        "  Phantom Rate:                 {:>12.2}% ({} bps)",
        sim_stats.phantom_rate_bps() as f64 / 100.0,
        sim_stats.phantom_rate_bps()
    );
    println!(
        "  Cumulative Staked:            {:>12} cents (${:.2})",
        sim_stats.total_filled_stake_cents,
        sim_stats.total_filled_stake_cents as f64 / 100.0
    );
    println!(
        "  Realized Worst-Case PnL:      {:>12} cents (${:.2})",
        sim_stats.total_realized_profit_cents,
        sim_stats.total_realized_profit_cents as f64 / 100.0
    );
    println!(
        "  Total Fees Paid:              {:>12} cents (${:.2})",
        sim_stats.total_fees_paid_cents,
        sim_stats.total_fees_paid_cents as f64 / 100.0
    );
    println!(
        "  Worst-Case Settlement ROI:    {:>12.2}%",
        sim_stats.realized_roi_bps() as f64 / 100.0
    );
    println!();
    println!("========================================================================");
    println!("  Analysis Complete: All hot path and correctness invariants verified.");
    println!("========================================================================");

    if let Some(path) = json_path {
        let sample = signal_collector.first().map(|signal| SampleSignal {
            profit_bps: signal.signal.profit_bps,
            total_stake_cents: signal.signal.total_stake,
            guaranteed_profit_cents: signal.signal.worst_case_profit,
        });
        let report = PipelineReport {
            schema_version: 1,
            run: RunMetadata {
                id: run_id,
                recorded_at_epoch_ms,
                source: "measured",
                project_version: env!("CARGO_PKG_VERSION"),
                git_commit,
            },
            environment: EnvironmentMetadata {
                label: format!("{} {}", env::consts::OS, env::consts::ARCH),
                os: env::consts::OS,
                arch: env::consts::ARCH,
                rustc: command_output("rustc", &["--version"]),
                build_profile: "release",
            },
            workload: WorkloadMetadata {
                synthetic: true,
                paper_trading: true,
                feed_events: num_ticks,
                event: "Boston Celtics @ Los Angeles Lakers",
                market: "Moneyline (2-way)",
                venues: ["Kalshi", "Polymarket", "Pinnacle"],
            },
            performance: PerformanceMetrics {
                elapsed_ms: total_duration.as_secs_f64() * 1000.0,
                throughput_per_second: events_per_sec,
                target_p99_ns: 50_000,
                latency_ns: LatencyMetrics {
                    count: service_summary.count,
                    min: service_summary.min_ns,
                    mean: service_summary.mean_ns,
                    p50: service_summary.p50_ns,
                    p90: service_summary.p90_ns,
                    p99: service_summary.p99_ns,
                    p999: service_summary.p999_ns,
                    max: service_summary.max_ns,
                },
            },
            detection: DetectionMetrics {
                events_processed: stats.events_processed,
                signals_emitted: stats.signals_emitted,
                collected_signals: signal_collector.len(),
                sample,
            },
            simulation: SimulationMetrics {
                total_signals: sim_stats.total_signals,
                clean_fills: sim_stats.clean_fills,
                proportional_fills: sim_stats.proportional_fills,
                phantoms: sim_stats.total_phantoms,
                phantom_rate_bps: sim_stats.phantom_rate_bps(),
                filled_stake_cents: sim_stats.total_filled_stake_cents,
                fees_paid_cents: sim_stats.total_fees_paid_cents,
                realized_profit_cents: sim_stats.total_realized_profit_cents,
                realized_roi_bps: sim_stats.realized_roi_bps(),
            },
        };

        if let Err(error) = write_report(&path, &report) {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
        println!("JSON report written to {}", path.display());
    }
}
