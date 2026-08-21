//! Comprehensive integration and unit tests for paper trading, latency modeling,
//! phantom arbitrage measurement, and PnL accounting.

use arbkit_core::arb::frictionless_leg;
use arbkit_core::{detect, Fee, Leg, Level, OutcomeBook, Prob};
use arbkit_sim::{
    ArbExecutionClassification, ChasePolicy, LatencyModel, LatencyProfile, PhantomReason,
    SimConfig, Simulator,
};

fn make_book(levels: &[(u32, i64)], seq: u64) -> OutcomeBook {
    let mut book = OutcomeBook::new();
    let lvl_vec: Vec<Level> = levels
        .iter()
        .map(|(cents, size)| Level {
            price: Prob::from_cents(*cents).unwrap(),
            size: *size,
        })
        .collect();
    book.apply_snapshot(&lvl_vec, seq);
    book
}

#[test]
fn test_clean_fill_arbitrage_execution() {
    // 48c on venue 0, 50c on venue 1 -> 98c to buy $1.00
    let leg0 = frictionless_leg(0, 0, Prob::from_cents(48).unwrap());
    let leg1 = frictionless_leg(1, 1, Prob::from_cents(50).unwrap());
    let legs = [leg0, leg1];

    let budget = 100_000; // $1,000 budget
    let signal = detect(&legs, budget)
        .unwrap()
        .expect("valid arbitrage signal");

    // Both venue books have plenty of resting depth at quoted prices
    let book0 = make_book(&[(48, 200_000)], 1);
    let book1 = make_book(&[(50, 200_000)], 1);
    let arrival_books = [book0, book1];

    let latency = LatencyModel::new(LatencyProfile::colocated());
    let mut sim = Simulator::new(latency);

    let report = sim
        .simulate_signal(1_000_000, &signal, &legs, &arrival_books)
        .expect("simulation succeeds");

    assert!(report.is_clean());
    assert!(!report.is_phantom());
    assert_eq!(report.classification, ArbExecutionClassification::CleanFill);

    assert_eq!(report.pnl.filled_stake, signal.total_stake);
    assert_eq!(report.pnl.realized_profit, signal.worst_case_profit);
    assert_eq!(report.pnl.slippage, 0);
    assert_eq!(report.pnl.fill_ratio_bps, 10_000); // 100% fill

    // Verify cumulative simulation stats
    let stats = sim.stats();
    assert_eq!(stats.total_signals, 1);
    assert_eq!(stats.clean_fills, 1);
    assert_eq!(stats.total_phantoms, 0);
    assert_eq!(stats.phantom_rate_bps(), 0);
    assert_eq!(stats.total_realized_profit_cents, signal.worst_case_profit);
}

#[test]
fn test_phantom_due_to_price_movement_decay() {
    // 48c on venue 0, 50c on venue 1 detected
    let leg0 = frictionless_leg(0, 0, Prob::from_cents(48).unwrap());
    let leg1 = frictionless_leg(1, 1, Prob::from_cents(50).unwrap());
    let legs = [leg0, leg1];

    let signal = detect(&legs, 100_000).unwrap().unwrap();

    // Venue 0 quote worsened from 48c to 52c before arrival
    let book0 = make_book(&[(52, 200_000)], 2);
    // Venue 1 remained 50c
    let book1 = make_book(&[(50, 200_000)], 1);
    let arrival_books = [book0, book1];

    let latency = LatencyModel::new(LatencyProfile::regional_cloud());
    let mut sim = Simulator::new(latency);

    let report = sim
        .simulate_signal(1_000_000, &signal, &legs, &arrival_books)
        .unwrap();

    assert!(report.is_phantom());
    // Venue 0 was unfilled due to price moving, Venue 1 filled -> broken leg!
    assert!(report.is_broken_leg());
    assert_eq!(
        report.classification,
        ArbExecutionClassification::Phantom(PhantomReason::BrokenLeg {
            filled_venue: 1,
            failed_venue: 0,
        })
    );

    // Worst-case payout is 0 because leg 0 failed and if outcome 0 occurs payout is 0
    assert_eq!(report.pnl.worst_case_payout, 0);
    // Realized profit is negative (loss of stake on filled leg 1)
    assert!(report.pnl.realized_profit < 0);

    let pstats = sim.phantom_stats();
    assert_eq!(pstats.total_detected, 1);
    assert_eq!(pstats.total_phantoms, 1);
    assert_eq!(pstats.phantoms_broken_leg, 1);
    assert_eq!(pstats.phantom_rate_bps(), 10_000); // 100% phantom
}

#[test]
fn test_phantom_due_to_queue_front_running_depth_exhaustion() {
    let leg0 = Leg {
        capacity: 10_000, // $100 depth at detection
        ..frictionless_leg(0, 0, Prob::from_cents(48).unwrap())
    };
    let leg1 = Leg {
        capacity: 50_000,
        ..frictionless_leg(1, 1, Prob::from_cents(50).unwrap())
    };
    let legs = [leg0, leg1];

    let signal = detect(&legs, 100_000).unwrap().unwrap();

    // Upon arrival, venue 0 resting depth is only 5_000 cents ($50)
    let book0 = make_book(&[(48, 5_000)], 1);
    let book1 = make_book(&[(50, 100_000)], 1);
    let arrival_books = [book0, book1];

    // High front-run latency profile eats 60% of resting depth
    let profile = LatencyProfile::new(2_000_000, 1_000_000, 6_000);
    let latency = LatencyModel::new(profile);
    let mut sim = Simulator::new(latency);

    let report = sim
        .simulate_signal(1_000_000, &signal, &legs, &arrival_books)
        .unwrap();

    // 5_000 depth with 60% front-run leaves 2_000 cents, which is less than requested stake
    assert!(report.leg_results()[0].is_partially_filled());
    assert!(report.leg_results()[1].is_fully_filled());

    // Because fills are asymmetric and unbalanced, this is classified as a phantom
    assert!(report.is_phantom());
    let stats = sim.phantom_stats();
    assert_eq!(stats.total_phantoms, 1);
}

#[test]
fn test_phantom_due_to_stale_dropped_book() {
    let leg0 = frictionless_leg(0, 0, Prob::from_cents(48).unwrap());
    let leg1 = frictionless_leg(1, 1, Prob::from_cents(50).unwrap());
    let legs = [leg0, leg1];
    let signal = detect(&legs, 100_000).unwrap().unwrap();

    // Venue 0 book went stale on dropped packet / reconnect
    let mut book0 = make_book(&[(48, 100_000)], 1);
    book0.mark_stale();
    let mut book1 = make_book(&[(50, 100_000)], 1);
    book1.mark_stale();
    let arrival_books = [book0, book1];

    let mut sim = Simulator::new(LatencyModel::default());
    let report = sim
        .simulate_signal(1_000_000, &signal, &legs, &arrival_books)
        .unwrap();

    assert!(report.is_phantom());
    assert_eq!(
        report.classification,
        ArbExecutionClassification::Phantom(PhantomReason::BookStale { venue: 0 })
    );
    assert_eq!(report.pnl.filled_stake, 0);
    assert_eq!(report.pnl.realized_profit, 0);
}

#[test]
fn test_three_way_soccer_market_simulation() {
    // 3-way soccer market: Home 45c, Away 30c, Draw 23c -> 98c total
    let leg0 = frictionless_leg(0, 0, Prob::from_cents(45).unwrap());
    let leg1 = frictionless_leg(1, 1, Prob::from_cents(30).unwrap());
    let leg2 = frictionless_leg(2, 2, Prob::from_cents(23).unwrap());
    let legs = [leg0, leg1, leg2];

    let signal = detect(&legs, 100_000).unwrap().unwrap();
    assert_eq!(signal.allocations().len(), 3);

    let book0 = make_book(&[(45, 100_000)], 1);
    let book1 = make_book(&[(30, 100_000)], 1);
    let book2 = make_book(&[(23, 100_000)], 1);
    let arrival_books = [book0, book1, book2];

    let mut sim = Simulator::new(LatencyModel::new(LatencyProfile::colocated()));
    let report = sim
        .simulate_signal(5_000_000, &signal, &legs, &arrival_books)
        .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.leg_results().len(), 3);
    assert_eq!(report.pnl.filled_stake, signal.total_stake);
    assert_eq!(report.pnl.realized_profit, signal.worst_case_profit);
}

#[test]
fn test_fee_impact_on_simulated_fills() {
    // 47c and 50c with Kalshi-style stake fees (350 bps on leg 0)
    let leg0 = Leg {
        fee: Fee::StakeFeeBps(350),
        ..frictionless_leg(0, 0, Prob::from_cents(47).unwrap())
    };
    let leg1 = Leg {
        fee: Fee::CommissionBps(200),
        ..frictionless_leg(1, 1, Prob::from_cents(50).unwrap())
    };
    let legs = [leg0, leg1];

    let signal = detect(&legs, 100_000).unwrap().unwrap();

    let book0 = make_book(&[(47, 100_000)], 1);
    let book1 = make_book(&[(50, 100_000)], 1);

    let mut sim = Simulator::new(LatencyModel::new(LatencyProfile::ZERO));
    let report = sim
        .simulate_signal(1_000, &signal, &legs, &[book0, book1])
        .unwrap();

    assert!(report.is_clean());
    assert!(report.pnl.total_fees > 0);
    assert_eq!(report.pnl.realized_profit, signal.worst_case_profit);
}

#[test]
fn test_summary_statistics_aggregation_and_reset() {
    let mut sim = Simulator::new(LatencyModel::default());

    let leg0 = frictionless_leg(0, 0, Prob::from_cents(48).unwrap());
    let leg1 = frictionless_leg(1, 1, Prob::from_cents(50).unwrap());
    let legs = [leg0, leg1];
    let signal = detect(&legs, 100_000).unwrap().unwrap();

    // 1st signal: Clean fill
    let good_books = [
        make_book(&[(48, 100_000)], 1),
        make_book(&[(50, 100_000)], 1),
    ];
    sim.simulate_signal(1, &signal, &legs, &good_books).unwrap();

    // 2nd signal: Phantom price moved (leg 0 moved from 48c to 55c)
    let moved_books = [
        make_book(&[(55, 100_000)], 2),
        make_book(&[(50, 100_000)], 1),
    ];
    sim.simulate_signal(2, &signal, &legs, &moved_books)
        .unwrap();

    // 3rd signal: Clean fill
    sim.simulate_signal(3, &signal, &legs, &good_books).unwrap();

    let stats = sim.stats();
    assert_eq!(stats.total_signals, 3);
    assert_eq!(stats.clean_fills, 2);
    assert_eq!(stats.total_phantoms, 1);
    assert_eq!(stats.phantom_rate_bps(), 3_333); // 33.33%

    assert!(stats.filled_stake_dollars_f64() > 0.0);

    // Reset and check zeroed state
    sim.reset();
    assert_eq!(sim.stats().total_signals, 0);
    assert_eq!(sim.phantom_stats().total_detected, 0);
}

#[test]
fn test_simulate_with_quotes_direct() {
    let leg0 = frictionless_leg(0, 0, Prob::from_cents(48).unwrap());
    let leg1 = frictionless_leg(1, 1, Prob::from_cents(50).unwrap());
    let legs = [leg0, leg1];
    let signal = detect(&legs, 100_000).unwrap().unwrap();

    let mut sim = Simulator::new(LatencyModel::new(LatencyProfile::ZERO));

    // Direct quotes: Venue 0 at 48c with $1000 depth, Venue 1 at 50c with $1000 depth
    let prices = [
        Some(Prob::from_cents(48).unwrap()),
        Some(Prob::from_cents(50).unwrap()),
    ];
    let depths = [100_000, 100_000];

    let report = sim
        .simulate_with_quotes(1_000_000, &signal, &legs, &prices, &depths)
        .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.pnl.filled_stake, signal.total_stake);
    assert_eq!(report.pnl.realized_profit, signal.worst_case_profit);
}

#[test]
fn test_asymmetric_venue_latencies() {
    let mut latency = LatencyModel::new(LatencyProfile::colocated());
    // Venue 0 is colocated (~40 µs), Venue 1 is cross-region (~40 ms)
    latency.set_venue_profile(0, LatencyProfile::colocated());
    latency.set_venue_profile(1, LatencyProfile::cross_region());

    let leg0 = frictionless_leg(0, 0, Prob::from_cents(48).unwrap());
    let leg1 = frictionless_leg(1, 1, Prob::from_cents(50).unwrap());
    let legs = [leg0, leg1];
    let signal = detect(&legs, 100_000).unwrap().unwrap();

    let mut sim = Simulator::new(latency);

    let book0 = make_book(&[(48, 100_000)], 1);
    let book1 = make_book(&[(50, 100_000)], 1);

    let report = sim
        .simulate_signal(100_000, &signal, &legs, &[book0, book1])
        .unwrap();

    assert_eq!(
        report.leg_results()[0].arrival_timestamp_ns,
        100_000 + LatencyProfile::colocated().total_latency_ns()
    );
    assert_eq!(
        report.leg_results()[1].arrival_timestamp_ns,
        100_000 + LatencyProfile::cross_region().total_latency_ns()
    );
}

#[test]
fn test_error_on_invalid_leg_count() {
    let mut sim = Simulator::new(LatencyModel::default());
    let leg0 = frictionless_leg(0, 0, Prob::from_cents(48).unwrap());
    let leg1 = frictionless_leg(1, 1, Prob::from_cents(50).unwrap());
    let legs = [leg0, leg1];
    let signal = detect(&legs, 100_000).unwrap().unwrap();

    // Mismatched arrival books length
    let single_book = [make_book(&[(48, 100_000)], 1)];
    assert!(sim
        .simulate_signal(1, &signal, &legs, &single_book)
        .is_err());
}

/// Shared fixture for the chase-policy tests below: two frictionless 40c
/// legs, which is a fat 25% edge (`worst_case_profit == 25_000`,
/// `profit_bps == 2_500`) so there is plenty of room for arrival prices to
/// move against us and still leave something behind.
fn chase_fixture_legs_and_signal() -> ([Leg; 2], arbkit_core::Signal) {
    let leg0 = frictionless_leg(0, 0, Prob::from_cents(40).unwrap());
    let leg1 = frictionless_leg(1, 1, Prob::from_cents(40).unwrap());
    let legs = [leg0, leg1];
    let signal = detect(&legs, 100_000).unwrap().unwrap();
    assert_eq!(signal.total_stake, 100_000);
    assert_eq!(signal.worst_case_profit, 25_000);
    (legs, signal)
}

#[test]
fn chased_fill_is_profitable_but_never_invents_profit_beyond_the_arrival_edge() {
    let (legs, signal) = chase_fixture_legs_and_signal();

    // Venue 0's 40c quote decays to 44c by arrival - a 1,000 bps adverse
    // move (40,000 ppm over a 400,000 ppm quote). Venue 1 is unchanged.
    // Ample depth on both sides so the chase fills in full.
    let arrival_prices = [
        Some(Prob::from_cents(44).unwrap()),
        Some(Prob::from_cents(40).unwrap()),
    ];
    let arrival_depths = [200_000, 200_000];

    let config = SimConfig {
        chase: ChasePolicy {
            enabled: true,
            max_chase_bps: 1_000,
        },
        signal_ttl_ns: 0,
    };
    let mut sim = Simulator::with_config(LatencyModel::new(LatencyProfile::ZERO), config);

    let report = sim
        .simulate_with_quotes(1_000_000, &signal, &legs, &arrival_prices, &arrival_depths)
        .unwrap();

    assert!(report.chased);
    assert!(report.is_clean());
    assert_eq!(report.classification, ArbExecutionClassification::CleanFill);

    // Hand-computed from the same pessimistic-floor mechanics
    // `LegFillResult::compute_fill` uses: at 44c, $500 staked returns
    // floor(50_000 * 1_000_000 / 440_000) = 113_636; at 40c it returns
    // 125_000. The worse leg governs, so realized profit is
    // 113_636 - 100_000 = 13_636 - not the 25_000 the stale detected quote
    // promised.
    assert_eq!(report.pnl.filled_stake, 100_000);
    assert_eq!(report.pnl.realized_profit, 13_636);
    assert!(report.pnl.realized_profit > 0);
    assert!(report.pnl.realized_profit < signal.worst_case_profit);

    let stats = sim.stats();
    assert_eq!(stats.chased_count, 1);
    assert_eq!(stats.chased_profit_cents, 13_636);
}

#[test]
fn ttl_expiry_drops_the_trade_even_though_arrival_prices_would_pass_the_chase() {
    let (legs, signal) = chase_fixture_legs_and_signal();

    let arrival_prices = [
        Some(Prob::from_cents(44).unwrap()),
        Some(Prob::from_cents(40).unwrap()),
    ];
    let arrival_depths = [200_000, 200_000];

    // Both venues take 2,000 ns to reach, so the attempt happens 2,000 ns
    // after detection. A 1,000 ns TTL has already lapsed by then.
    let latency = LatencyModel::new(LatencyProfile::new(1_000, 1_000, 0));
    let config = SimConfig {
        chase: ChasePolicy {
            enabled: true,
            max_chase_bps: 1_000,
        },
        signal_ttl_ns: 1_000,
    };
    let mut sim = Simulator::with_config(latency, config);

    let report = sim
        .simulate_with_quotes(0, &signal, &legs, &arrival_prices, &arrival_depths)
        .unwrap();

    // Same historical drop as an unchased leg: venue 0 never fills, venue 1
    // does, which is an unhedged broken leg.
    assert!(!report.chased);
    assert!(report.is_phantom());
    assert!(report.is_broken_leg());
    assert_eq!(
        report.classification,
        ArbExecutionClassification::Phantom(PhantomReason::BrokenLeg {
            filled_venue: 1,
            failed_venue: 0,
        })
    );
    assert_eq!(sim.stats().chased_count, 0);
}

#[test]
fn max_chase_bps_is_a_hard_boundary() {
    let (legs, signal) = chase_fixture_legs_and_signal();

    // Exactly 1,000 bps of adverse movement on venue 0.
    let arrival_prices = [
        Some(Prob::from_cents(44).unwrap()),
        Some(Prob::from_cents(40).unwrap()),
    ];
    let arrival_depths = [200_000, 200_000];

    // Exactly at the threshold: passes.
    let passing_config = SimConfig {
        chase: ChasePolicy {
            enabled: true,
            max_chase_bps: 1_000,
        },
        signal_ttl_ns: 0,
    };
    let mut passing_sim =
        Simulator::with_config(LatencyModel::new(LatencyProfile::ZERO), passing_config);
    let passing_report = passing_sim
        .simulate_with_quotes(1_000_000, &signal, &legs, &arrival_prices, &arrival_depths)
        .unwrap();
    assert!(passing_report.chased);
    assert!(passing_report.is_clean());

    // One bps under the required threshold: the same move now fails the
    // joint gate, and the whole signal reverts to the historical drop.
    let failing_config = SimConfig {
        chase: ChasePolicy {
            enabled: true,
            max_chase_bps: 999,
        },
        signal_ttl_ns: 0,
    };
    let mut failing_sim =
        Simulator::with_config(LatencyModel::new(LatencyProfile::ZERO), failing_config);
    let failing_report = failing_sim
        .simulate_with_quotes(1_000_000, &signal, &legs, &arrival_prices, &arrival_depths)
        .unwrap();
    assert!(!failing_report.chased);
    assert!(failing_report.is_phantom());
    assert!(failing_report.is_broken_leg());
}

#[test]
fn disabled_chase_policy_is_byte_identical_to_the_historical_drop_behavior() {
    let (legs, signal) = chase_fixture_legs_and_signal();

    let arrival_prices = [
        Some(Prob::from_cents(44).unwrap()),
        Some(Prob::from_cents(40).unwrap()),
    ];
    let arrival_depths = [200_000, 200_000];

    // `Simulator::new` defaults to `ChasePolicy::DISABLED`.
    let mut default_sim = Simulator::new(LatencyModel::new(LatencyProfile::ZERO));
    let default_report = default_sim
        .simulate_with_quotes(1_000_000, &signal, &legs, &arrival_prices, &arrival_depths)
        .unwrap();

    // Explicitly-disabled config, to confirm the two paths agree.
    let explicit_config = SimConfig {
        chase: ChasePolicy {
            enabled: false,
            max_chase_bps: 1_000_000, // would trivially pass if enabled
        },
        signal_ttl_ns: 0,
    };
    let mut explicit_sim =
        Simulator::with_config(LatencyModel::new(LatencyProfile::ZERO), explicit_config);
    let explicit_report = explicit_sim
        .simulate_with_quotes(1_000_000, &signal, &legs, &arrival_prices, &arrival_depths)
        .unwrap();

    for report in [&default_report, &explicit_report] {
        assert!(!report.chased);
        assert!(report.is_phantom());
        assert!(report.is_broken_leg());
        assert_eq!(
            report.classification,
            ArbExecutionClassification::Phantom(PhantomReason::BrokenLeg {
                filled_venue: 1,
                failed_venue: 0,
            })
        );
        assert_eq!(report.pnl.filled_stake, 50_000);
        assert_eq!(report.pnl.worst_case_payout, 0);
    }
    assert_eq!(default_report.pnl, explicit_report.pnl);
    assert_eq!(default_sim.stats().chased_count, 0);
    assert_eq!(explicit_sim.stats().chased_count, 0);
}
