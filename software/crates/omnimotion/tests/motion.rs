use omnimotion::{
    generation_is_acknowledged, group_limits, nco_increment, steps_per_mm, AbortGenerations,
    AbortTicket, AxisConfig, EngineError, Limits, LimitsError, MoveStart, PlanError, Planner,
    Profile, Segment, StepEngine, Trapezoid, UnitError, AXES, TICK_HZ,
};
use std::collections::VecDeque;
use std::sync::{Arc, Barrier};
use std::thread;

fn limits() -> Limits {
    Limits {
        max_rate: 8_000.0,
        min_rate: 200.0,
        accel: 20_000.0,
    }
}

fn rate_hz(increment: u32) -> f32 {
    increment as f32 / 4_294_967_296.0 * TICK_HZ as f32
}

/// Runs a move to completion, asserting the Bresenham invariant on every tick,
/// and returns the engine plus the rate of each segment in Hz.
fn run(delta: [i64; AXES], limits: Limits) -> (StepEngine, Vec<f32>) {
    let mut engine = StepEngine::new();
    let planner = Planner::new(delta, limits, TICK_HZ, Trapezoid).unwrap();
    let total = planner.total_steps() as i64;
    let dominant = delta
        .iter()
        .position(|axis| axis.unsigned_abs() as i64 == total)
        .expect("some axis covers the master distance");
    let mut rates = Vec::new();
    let mut ticks = 0u64;

    for segment in planner {
        let segment = segment.unwrap();
        rates.push(rate_hz(segment.rate));
        engine.load(segment).unwrap();

        while !engine.idle() {
            engine.tick().unwrap();
            ticks += 1;
            assert!(ticks < 10_000_000, "engine never finished");

            // Every axis must stay within one step of its exact share of the
            // distance covered so far. This is what "the axes cannot drift" means.
            let position = engine.position();
            let master = position[dominant].abs();

            for axis in 0..AXES {
                let expected = master * delta[axis];
                let actual = position[axis] * total;
                assert!(
                    (expected - actual).abs() <= total,
                    "axis {axis} drifted: {position:?} at master {master}"
                );
            }
        }
    }

    (engine, rates)
}

#[test]
fn arrives_exactly() {
    for delta in [
        [1, 0, 0],
        [1000, 0, 0],
        [1000, 400, 250],
        [-500, 0, 0],
        [-320, 640, -160],
        [7, 7, 7],
        [10_000, 1, 0],
        [0, 0, 900],
    ] {
        let (engine, _) = run(delta, limits());
        assert_eq!(engine.position(), delta, "move {delta:?} missed its target");
    }
}

#[test]
fn segment_steps_sum_to_the_master_total() {
    let delta = [1000, 400, 250];
    let planner = Planner::new(delta, limits(), TICK_HZ, Trapezoid).unwrap();
    let total = planner.total_steps();
    let summed: u64 = planner.map(|s| s.unwrap().steps as u64).sum();

    assert_eq!(total, 1000);
    assert_eq!(summed, total);
}

#[test]
fn a_zero_move_is_rejected() {
    assert_eq!(
        Planner::new([0, 0, 0], limits(), TICK_HZ, Trapezoid).err(),
        Some(PlanError::EmptyMove)
    );
}

#[test]
fn only_the_first_segment_carries_the_bresenham_setup() {
    let mut planner = Planner::new([5000, 100, 20], limits(), TICK_HZ, Trapezoid).unwrap();

    let first = planner
        .next()
        .unwrap()
        .unwrap()
        .start
        .expect("first segment sets up");
    assert_eq!(first.delta, [5000, 100, 20]);
    assert_eq!(first.dir, 0b111);
    assert!(planner.all(|segment| segment.unwrap().start.is_none()));
}

#[test]
fn direction_follows_the_sign_of_each_axis() {
    let start = Planner::new([-10, 10, -5], limits(), TICK_HZ, Trapezoid)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .start
        .unwrap();

    assert_eq!(start.delta, [10, 10, 5]);
    assert_eq!(start.dir, 0b010);
}

#[test]
fn the_ramp_is_symmetric() {
    let (total, limits) = (4_000u64, limits());

    for travelled in (0..=total).step_by(97) {
        let forward = Trapezoid.rate(travelled, total, &limits);
        let mirrored = Trapezoid.rate(total - travelled, total, &limits);
        assert!(
            (forward - mirrored).abs() < 0.01,
            "asymmetric at {travelled}: {forward} vs {mirrored}"
        );
    }
}

#[test]
fn a_long_move_reaches_the_rate_ceiling() {
    let limits = limits();
    let (_, rates) = run([8_000, 0, 0], limits);
    let peak = rates.iter().cloned().fold(0.0f32, f32::max);

    assert!(
        (peak - limits.max_rate).abs() / limits.max_rate < 0.01,
        "expected to cruise at {}, peaked at {peak}",
        limits.max_rate
    );
}

#[test]
fn a_short_move_never_reaches_the_rate_ceiling() {
    let limits = limits();
    let (_, rates) = run([600, 0, 0], limits);
    let peak = rates.iter().cloned().fold(0.0f32, f32::max);

    // 600 steps at 20 000 steps/s² tops out well under 8 000 steps/s.
    assert!(peak < limits.max_rate * 0.8, "peaked at {peak}");
    assert!(
        peak > limits.min_rate,
        "never accelerated, peaked at {peak}"
    );
}

#[test]
fn the_ramp_rises_from_the_minimum_and_falls_back_to_it() {
    let limits = limits();
    let (_, rates) = run([4_000, 0, 0], limits);
    let last = *rates.last().unwrap();

    assert!(
        (rates[0] - limits.min_rate).abs() < 0.5,
        "starts at {}",
        rates[0]
    );

    // The tail cannot land exactly on min_rate: a segment's rate is sampled at
    // its start, and the final segment still has one step left to decelerate over.
    let one_step_out = (limits.min_rate.powi(2) + 2.0 * limits.accel).sqrt();
    assert!(
        last <= one_step_out + 0.5,
        "ended at {last}, expected <= {one_step_out}"
    );

    let peak = rates.iter().cloned().fold(0.0f32, f32::max);
    assert!(
        last < peak / 10.0,
        "did not decelerate: peak {peak}, ended {last}"
    );
}

#[test]
fn the_ramp_only_changes_direction_once() {
    let (_, rates) = run([4_000, 0, 0], limits());
    let falling = rates.windows(2).position(|w| w[1] < w[0]).unwrap();

    assert!(
        rates[..falling].windows(2).all(|w| w[1] >= w[0]),
        "accel not monotonic"
    );
    assert!(
        rates[falling..].windows(2).all(|w| w[1] <= w[0]),
        "decel not monotonic"
    );
}

#[test]
fn aborting_leaves_the_position_exact() {
    let delta = [5_000, 2_500, 1_250];
    let mut engine = StepEngine::new();
    let mut emitted = [0i64; AXES];
    let mut planner = Planner::new(delta, limits(), TICK_HZ, Trapezoid).unwrap();

    'outer: for segment in planner.by_ref() {
        engine.load(segment.unwrap()).unwrap();
        while !engine.idle() {
            let tick = engine.tick().unwrap();
            for (axis, count) in emitted.iter_mut().enumerate() {
                if tick.step & (1 << axis) != 0 {
                    *count += if tick.dir & (1 << axis) != 0 { 1 } else { -1 };
                }
            }
            if emitted[0] >= 1_000 {
                engine.abort();
                break 'outer;
            }
        }
    }

    assert_eq!(engine.position(), emitted);
    assert!(engine.idle());
    assert_eq!(engine.tick().unwrap().step, 0);
}

#[test]
fn abort_generations_coalesce_without_losing_a_request() {
    let aborts = AbortGenerations::new();
    let first = aborts.request();
    let second = aborts.request();

    assert!(aborts.pending());
    assert!(!aborts.is_acknowledged(first));
    aborts.acknowledge(second);
    assert!(!aborts.pending());
    assert!(aborts.is_acknowledged(first));
    assert!(aborts.is_acknowledged(second));
}

#[test]
fn abort_acknowledgement_comparison_handles_wrap() {
    assert!(generation_is_acknowledged(0, u32::MAX));
    assert!(!generation_is_acknowledged(u32::MAX, 0));
}

#[test]
fn concurrent_abort_requests_are_unique_and_coalesce() {
    const REQUESTERS: usize = 8;
    let aborts = Arc::new(AbortGenerations::new());
    let barrier = Arc::new(Barrier::new(REQUESTERS));
    let mut threads = Vec::new();

    for _ in 0..REQUESTERS {
        let aborts = Arc::clone(&aborts);
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            aborts.request()
        }));
    }

    let mut tickets: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    tickets.sort_by_key(|ticket| ticket.generation());
    assert_eq!(
        tickets
            .iter()
            .map(|ticket| ticket.generation())
            .collect::<Vec<_>>(),
        (1..=REQUESTERS as u32).collect::<Vec<_>>()
    );

    aborts.acknowledge(*tickets.last().unwrap());
    assert!(!aborts.state().pending());
    assert!(tickets
        .into_iter()
        .all(|ticket| aborts.state().is_acknowledged(ticket)));
}

#[test]
fn stale_and_repeated_abort_tickets_remain_acknowledged() {
    let aborts = AbortGenerations::new();
    let stale = aborts.request();
    let repeated = aborts.request();
    aborts.acknowledge(repeated);

    assert!(aborts.is_acknowledged(stale));
    assert!(aborts.is_acknowledged(repeated));

    let current = aborts.request();
    assert!(aborts.is_acknowledged(stale));
    assert!(!aborts.is_acknowledged(current));
    aborts.acknowledge(current);
    assert!(aborts.is_acknowledged(current));
}

#[test]
fn acknowledgement_stops_a_generation_and_preserves_exact_counters() {
    let aborts = AbortGenerations::new();
    let move_generation = aborts.request();
    aborts.acknowledge(move_generation);
    let mut queue: VecDeque<(AbortTicket, Segment)> =
        Planner::new([20_000, -10_000, 5_000], limits(), TICK_HZ, Trapezoid)
            .unwrap()
            .map(|segment| (move_generation, segment.unwrap()))
            .collect();
    let mut engine = StepEngine::new();
    let mut emitted = [0i64; AXES];

    while emitted[0] < 1_000 {
        if engine.idle() {
            let (generation, segment) = queue.pop_front().unwrap();
            assert_eq!(generation, aborts.requested());
            engine.load(segment).unwrap();
        }
        let tick = engine.tick().unwrap();
        for (axis, count) in emitted.iter_mut().enumerate() {
            if tick.step & (1 << axis) != 0 {
                *count += if tick.dir & (1 << axis) != 0 { 1 } else { -1 };
            }
        }
    }

    let ticket = aborts.request();
    engine.abort();
    queue.clear();
    aborts.acknowledge(ticket);
    let acknowledged_position = engine.position();

    // Model the producer/ISR race: a segment from the stopped move arrives
    // after acknowledgement. The ISR rejects its stale generation.
    let stale_segment = Planner::new([100, 0, 0], limits(), TICK_HZ, Trapezoid)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    queue.push_back((move_generation, stale_segment));
    if let Some((generation, segment)) = queue.pop_front() {
        if generation == aborts.requested() {
            engine.load(segment).unwrap();
        }
    }

    for _ in 0..10_000 {
        assert_eq!(engine.tick().unwrap().step, 0);
    }
    assert!(aborts.is_acknowledged(ticket));
    assert_eq!(engine.position(), acknowledged_position);
    assert_eq!(engine.position(), emitted);
    assert_eq!(emitted, [1_000, -500, 250]);
}

#[test]
fn a_loaded_engine_resumes_from_its_current_position() {
    let mut engine = StepEngine::new();
    engine.set_position([100, -50, 25]);

    for segment in Planner::new([10, 0, 0], limits(), TICK_HZ, Trapezoid).unwrap() {
        engine.load(segment.unwrap()).unwrap();
        while !engine.idle() {
            engine.tick().unwrap();
        }
    }

    assert_eq!(engine.position(), [110, -50, 25]);
}

#[test]
fn nco_increments_map_rate_onto_ticks() {
    assert_eq!(nco_increment(TICK_HZ as f32, TICK_HZ), Ok(u32::MAX));
    assert!(nco_increment(0.0, TICK_HZ).is_err());

    let half = nco_increment(TICK_HZ as f32 / 2.0, TICK_HZ).unwrap();
    assert!((rate_hz(half) - TICK_HZ as f32 / 2.0).abs() < 1.0);
}

#[test]
fn axis_config_converts_between_units_and_steps() {
    let axis = AxisConfig {
        steps_per_unit: steps_per_mm(200, 16, 40.0).unwrap(),
        max_velocity: 100.0,
        max_acceleration: 1_000.0,
        min_velocity: 2.5,
    };

    assert_eq!(axis.steps_per_unit, 80.0);
    assert_eq!(axis.steps(10.0), Ok(800));
    assert_eq!(axis.units(800), Ok(10.0));

    let limits = axis.limits().unwrap();
    assert_eq!(limits.max_rate, 8_000.0);
    assert_eq!(limits.accel, 80_000.0);
    assert_eq!(limits.min_rate, 200.0);
}

#[test]
fn group_limits_are_set_by_the_most_constrained_axis() {
    let fast = AxisConfig {
        steps_per_unit: 80.0,
        max_velocity: 100.0,
        max_acceleration: 1_000.0,
        min_velocity: 2.5,
    };
    let slow = AxisConfig {
        max_velocity: 25.0,
        ..fast
    };

    // Both axes travel the same distance, so the slow one caps the master rate.
    let limits = group_limits(&[fast, slow, fast], &[1_000, 1_000, 0]).unwrap();
    assert_eq!(limits.max_rate, 2_000.0);

    // The slow axis now covers a quarter of the distance, so it constrains the
    // master four times less and the two axes come out even.
    let limits = group_limits(&[fast, slow, fast], &[1_000, 250, 0]).unwrap();
    assert_eq!(limits.max_rate, 8_000.0);
    // min_rate is the rate every axis may jump to without a ramp, so the
    // lowest transformed allowance is the safe group start rate.
    assert_eq!(limits.min_rate, 200.0);

    // An axis that is not moving never constrains the group.
    let limits = group_limits(&[fast, fast, slow], &[1_000, 0, 0]).unwrap();
    assert_eq!(limits.max_rate, 8_000.0);
}

#[test]
fn invalid_planning_inputs_are_rejected() {
    for bad in [
        Limits {
            max_rate: 0.0,
            ..limits()
        },
        Limits {
            min_rate: -1.0,
            ..limits()
        },
        Limits {
            accel: f32::NAN,
            ..limits()
        },
        Limits {
            min_rate: 9_000.0,
            ..limits()
        },
    ] {
        assert!(Planner::new([1, 0, 0], bad, TICK_HZ, Trapezoid).is_err());
    }

    assert_eq!(
        Planner::new([1, 0, 0], limits(), 0, Trapezoid).err(),
        Some(PlanError::UnsupportedTickRate)
    );
    assert_eq!(
        nco_increment(TICK_HZ as f32 + 1.0, TICK_HZ),
        Err(PlanError::RateExceedsTickRate)
    );
    assert_eq!(
        Limits {
            max_rate: 1.0,
            min_rate: 2.0,
            accel: 1.0
        }
        .validate(),
        Err(LimitsError::MinimumExceedsMaximum)
    );
}

#[test]
fn invalid_unit_conversions_are_rejected() {
    assert_eq!(steps_per_mm(200, 0, 40.0), Err(UnitError::NonPositive));
    assert_eq!(
        steps_per_mm(u32::MAX, 2, 40.0),
        Err(UnitError::CoordinateOverflow)
    );

    let invalid = AxisConfig {
        steps_per_unit: f32::NAN,
        max_velocity: 1.0,
        max_acceleration: 1.0,
        min_velocity: 1.0,
    };
    assert_eq!(invalid.steps(1.0), Err(UnitError::NonFinite));
    assert_eq!(
        group_limits(&[invalid; AXES], &[0; AXES]),
        Err(UnitError::EmptyMove)
    );
}

#[test]
fn segment_rates_stay_inside_both_endpoint_envelopes() {
    let limits = limits();
    let planner = Planner::new([4_000, 0, 0], limits, TICK_HZ, Trapezoid).unwrap();
    let total = planner.total_steps();
    let mut travelled = 0;

    for segment in planner {
        let segment = segment.unwrap();
        let end = travelled + segment.steps as u64;
        let actual = rate_hz(segment.rate);
        let bound = Trapezoid
            .rate(travelled, total, &limits)
            .min(Trapezoid.rate(end, total, &limits));
        assert!(
            actual <= bound + 0.01,
            "segment {travelled}..{end}: {actual} exceeds {bound}"
        );
        travelled = end;
    }
    assert_eq!(travelled, total);
}

#[test]
fn only_the_final_segment_is_marked_last() {
    let segments: Vec<_> = Planner::new([4_000, 0, 0], limits(), TICK_HZ, Trapezoid)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(segments.last().unwrap().last);
    assert!(segments[..segments.len() - 1]
        .iter()
        .all(|segment| !segment.last));
}

/// Plans `delta`, calls `stop` once `stop_after` master steps are planned,
/// and runs every segment through the engine. Returns the engine, the rate of
/// each segment in Hz and the steps planned when the stop came.
fn run_with_stop(delta: [i64; AXES], stop_after: u64) -> (StepEngine, Vec<f32>, u64) {
    let mut planner = Planner::new(delta, limits(), TICK_HZ, Trapezoid).unwrap();
    let mut engine = StepEngine::new();
    let mut rates = Vec::new();
    let mut planned = 0u64;
    let mut stopped_at = None;
    while let Some(segment) = planner.next() {
        let segment = segment.unwrap();
        planned += u64::from(segment.steps);
        rates.push(rate_hz(segment.rate));
        engine.load(segment).unwrap();
        while !engine.idle() {
            engine.tick().unwrap();
        }
        if stopped_at.is_none() && planned >= stop_after {
            planner.stop();
            stopped_at = Some(planned);
        }
    }
    (engine, rates, stopped_at.unwrap())
}

#[test]
fn a_stop_mid_cruise_decelerates_over_the_shortest_ramp() {
    let limits = limits();
    let (engine, rates, stopped_at) = run_with_stop([100_000, 50_000, 0], 20_000);
    let travelled = engine.position()[0] as u64;
    let ramp = (limits.max_rate.powi(2) - limits.min_rate.powi(2)) / (2.0 * limits.accel);

    assert!(travelled < 100_000);
    assert!(
        (travelled - stopped_at) as f32 <= ramp + 1.0,
        "took {} steps to stop, ramp is {ramp}",
        travelled - stopped_at
    );
    assert!((engine.position()[1] - engine.position()[0] / 2).abs() <= 1);
    let after = &rates[rates.len() - 10..];
    assert!(after.windows(2).all(|pair| pair[1] <= pair[0]));
    let one_step_out = (limits.min_rate.powi(2) + 2.0 * limits.accel).sqrt();
    assert!(*rates.last().unwrap() <= one_step_out + 0.5);
}

#[test]
fn a_stop_already_inside_the_final_ramp_changes_nothing() {
    let (engine, _, _) = run_with_stop([4_000, 0, 0], 3_990);
    assert_eq!(engine.position()[0], 4_000);
}

#[test]
fn only_a_final_segment_may_end_a_move_early() {
    let start = Some(MoveStart {
        delta: [100, 0, 0],
        dir: 0b111,
    });
    let mut engine = StepEngine::new();
    let early = Segment {
        rate: u32::MAX,
        steps: 10,
        start,
        last: true,
    };
    engine.load(early).unwrap();
    while !engine.idle() {
        engine.tick().unwrap();
    }
    assert_eq!(engine.position()[0], 10);

    let whole_but_not_last = Segment {
        rate: u32::MAX,
        steps: 100,
        start,
        last: false,
    };
    assert_eq!(
        StepEngine::new().load(whole_but_not_last),
        Err(EngineError::InvalidSegment)
    );
}

#[test]
fn engine_reports_coordinate_overflow_without_wrapping() {
    let mut engine = StepEngine::new();
    engine.set_position([i64::MAX, 0, 0]);
    engine
        .load(Segment {
            rate: u32::MAX,
            steps: 1,
            start: Some(MoveStart {
                delta: [1, 0, 0],
                dir: 0b111,
            }),
            last: true,
        })
        .unwrap();

    assert!(engine.tick().is_ok());
    assert_eq!(engine.tick(), Err(EngineError::CoordinateOverflow));
    assert_eq!(engine.position()[0], i64::MAX);
}
