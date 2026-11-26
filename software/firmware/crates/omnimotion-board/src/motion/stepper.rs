use core::future::poll_fn;
use core::sync::atomic::{fence, AtomicBool, AtomicU32, Ordering};
use core::task::Poll;

use embassy_sync::waitqueue::AtomicWaker;
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::gpio::{AnyPin, Level, Output, OutputConfig, Pin};
use esp_hal::interrupt::Priority;
use esp_hal::peripherals::{GPIO, TIMG0};
use esp_hal::time::Duration;
use esp_hal::timer::timg::{TimerGroup, Wdt};
use esp_hal::timer::Timer as _;
use esp_hal::{handler, ram};
use heapless::spsc::{Consumer, Producer, Queue};
use omnimotion::{AbortGenerations, AbortState, AbortTicket, AxisMask, Segment, StepEngine, AXES};
use static_cell::StaticCell;

pub const DEFAULT_TICK_HZ: u32 = 40_000;
pub(crate) type Watchdog = Wdt<TIMG0<'static>>;

const QUEUE_DEPTH: usize = 16;

/// Half the queue, so the feeder refills in batches rather than once per segment.
const REFILL_AT: usize = QUEUE_DEPTH / 2;

#[derive(Clone, Copy)]
struct QueuedSegment {
    segment: Segment,
    generation: AbortTicket,
}

struct AtomicPosition {
    low: AtomicU32,
    high: AtomicU32,
}

impl AtomicPosition {
    const fn new() -> Self {
        Self {
            low: AtomicU32::new(0),
            high: AtomicU32::new(0),
        }
    }

    fn store(&self, value: i64) {
        let bits = value as u64;
        self.low.store(bits as u32, Ordering::Relaxed);
        self.high.store((bits >> 32) as u32, Ordering::Relaxed);
    }

    fn load(&self) -> i64 {
        ((self.high.load(Ordering::Relaxed) as u64) << 32 | self.low.load(Ordering::Relaxed) as u64)
            as i64
    }
}

static POSITION: [AtomicPosition; AXES] = [const { AtomicPosition::new() }; AXES];
static POSITION_SEQUENCE: AtomicU32 = AtomicU32::new(0);
static IDLE: AtomicBool = AtomicBool::new(true);
static ABORTS: AbortGenerations = AbortGenerations::new();
static STOPS: AtomicU32 = AtomicU32::new(0);
static UNDERRUN: AtomicBool = AtomicBool::new(false);
static ENGINE_FAULT: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static FEEDER: AtomicWaker = AtomicWaker::new();

/// One GPIO bit per axis, so a tick drives every axis with a single register
/// write and all three step edges land on the same cycle.
struct Bits {
    step: [u32; AXES],
    dir: [u32; AXES],
    dir_all: u32,
    /// Held only to keep the pins as outputs; the ISR writes the registers.
    _step: [Output<'static>; AXES],
    _dir: [Output<'static>; AXES],
}

impl Bits {
    fn new(step: [AnyPin<'static>; AXES], dir: [AnyPin<'static>; AXES]) -> Self {
        let step_bits = core::array::from_fn(|axis| bit(&step[axis]));
        let dir_bits: [u32; AXES] = core::array::from_fn(|axis| bit(&dir[axis]));
        let config = OutputConfig::default();

        Self {
            step: step_bits,
            dir: dir_bits,
            dir_all: dir_bits.iter().fold(0, |all, bit| all | bit),
            _step: step.map(|pin| Output::new(pin, Level::Low, config)),
            _dir: dir.map(|pin| Output::new(pin, Level::Low, config)),
        }
    }

    fn mask(bits: &[u32; AXES], axes: AxisMask) -> u32 {
        let mut mask = 0;
        for (axis, bit) in bits.iter().enumerate() {
            if axes & (1 << axis) != 0 {
                mask |= bit;
            }
        }
        mask
    }
}

fn bit(pin: &AnyPin<'static>) -> u32 {
    let number = pin.number();
    // Every step and dir pin on this board is GPIO4-9, which keeps the ISR to
    // the low bank and therefore to one register.
    assert!(number < 32, "GPIO{number} is outside the low bank");
    1 << number
}

/// `w1ts`/`w1tc` only act on the bits written, so these never disturb a pin the
/// rest of the firmware owns.
#[inline(always)]
fn set(mask: u32) {
    if mask != 0 {
        GPIO::regs().out_w1ts().write(|w| unsafe { w.bits(mask) });
    }
}

#[inline(always)]
fn clear(mask: u32) {
    if mask != 0 {
        GPIO::regs().out_w1tc().write(|w| unsafe { w.bits(mask) });
    }
}

fn publish_position(position: [i64; AXES]) {
    POSITION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    fence(Ordering::Release);
    for (counter, value) in POSITION.iter().zip(position) {
        counter.store(value);
    }
    POSITION_SEQUENCE.fetch_add(1, Ordering::Release);
}

struct Interrupt {
    engine: StepEngine,
    segments: Consumer<'static, QueuedSegment>,
    bits: Bits,
    /// STEP bits currently high, retired on the next tick.
    pulsed: u32,
    dir: AxisMask,
    active_move: bool,
    move_ending: bool,
}

impl Interrupt {
    #[inline(always)]
    fn rearm(&self) {
        let regs = TIMG0::regs();
        regs.int_clr().write(|w| w.t(0).clear_bit_by_one());
        regs.t(0).config().modify(|_, w| w.alarm_en().set_bit());
    }

    #[inline(always)]
    fn discard(&mut self) {
        self.engine.abort();
        while self.segments.dequeue().is_some() {}
    }

    #[inline(always)]
    fn fault(&mut self) {
        ENGINE_FAULT.store(true, Ordering::Release);
        self.discard();
        self.go_idle();
    }

    /// Parks the timer until the feeder enqueues again. The re-check closes the
    /// race with an enqueue or abort that saw the timer still running.
    #[inline(always)]
    fn go_idle(&mut self) {
        self.active_move = false;
        self.move_ending = false;
        IDLE.store(true, Ordering::Release);
        FEEDER.wake();
        park_timer();
        if !self.segments.is_empty() || ABORTS.pending() {
            kick_timer();
        }
    }
}

#[inline(always)]
fn kick_timer() {
    if STARTED.load(Ordering::Acquire) {
        let regs = TIMG0::regs();
        let timer = regs.t(0);
        if timer.config().read().en().bit_is_clear() {
            timer.loadlo().write(|w| unsafe { w.load_lo().bits(0) });
            timer.loadhi().write(|w| unsafe { w.load_hi().bits(0) });
            timer.load().write(|w| unsafe { w.load().bits(1) });
            regs.int_clr().write(|w| w.t(0).clear_bit_by_one());
            timer
                .config()
                .modify(|_, w| w.en().set_bit().alarm_en().set_bit());
        }
    }
}

#[inline(always)]
fn park_timer() {
    TIMG0::regs()
        .t(0)
        .config()
        .modify(|_, w| w.en().clear_bit().alarm_en().clear_bit());
}

/// Written once by `start` before the timer interrupt is enabled, and read only
/// by `on_tick` afterwards. `STARTED` rejects a second `start` so that stays true.
static mut INTERRUPT: Option<Interrupt> = None;

/// Feeds segments to the step ISR.
pub(crate) struct Stepper {
    segments: Producer<'static, QueuedSegment>,
    tick_hz: u32,
    step_bits: [u32; AXES],
    dir_bits: [u32; AXES],
}

/// One axis's STEP and DIR, driven directly for a wiring check. Both go back to
/// their previous levels on drop, because the ISR assumes DIR still holds the
/// level it last wrote.
pub(crate) struct HeldLines<'a> {
    _stepper: &'a mut Stepper,
    step: u32,
    dir: u32,
    saved: u32,
}

impl HeldLines<'_> {
    pub(crate) fn drive(&mut self, step: bool, dir: bool) {
        let high = if step { self.step } else { 0 } | if dir { self.dir } else { 0 };
        set(high);
        clear((self.step | self.dir) & !high);
    }
}

impl Drop for HeldLines<'_> {
    fn drop(&mut self) {
        let lines = self.step | self.dir;
        set(self.saved & lines);
        clear(lines & !self.saved);
    }
}

/// Installs the tick ISR and starts TIMG0. Panics if called twice.
pub(crate) fn start(
    timg0: TIMG0<'static>,
    step: [AnyPin<'static>; AXES],
    dir: [AnyPin<'static>; AXES],
    tick_hz: u32,
) -> (Stepper, Watchdog) {
    assert!(
        !STARTED.swap(true, Ordering::SeqCst),
        "the step engine is already running"
    );

    static QUEUE: StaticCell<Queue<QueuedSegment, QUEUE_DEPTH>> = StaticCell::new();
    let (producer, consumer) = QUEUE.init(Queue::new()).split();

    assert!((1..=1_000_000).contains(&tick_hz), "unsupported tick rate");
    let period_us = 1_000_000 / tick_hz;
    let realized_tick_hz = 1_000_000 / period_us;

    let timers = TimerGroup::new(timg0);
    let timer = timers.timer0;
    let watchdog = timers.wdt;
    timer.set_interrupt_handler(on_tick);
    timer
        .load_value(Duration::from_micros(period_us as u64))
        .expect("tick rate out of range for TIMG0");
    timer.enable_auto_reload(true);

    let bits = Bits::new(step, dir);
    let (step_bits, dir_bits) = (bits.step, bits.dir);

    // Safety: the ISR stays disabled until INTERRUPT is initialized once.
    unsafe {
        INTERRUPT = Some(Interrupt {
            engine: StepEngine::new(),
            segments: consumer,
            bits,
            pulsed: 0,
            dir: AxisMask::MAX,
            active_move: false,
            move_ending: false,
        });
    }
    timer.enable_interrupt(true);
    timer.start();

    (
        Stepper {
            segments: producer,
            tick_hz: realized_tick_hz,
            step_bits,
            dir_bits,
        },
        watchdog,
    )
}

pub(crate) fn abort() -> AbortTicket {
    let ticket = ABORTS.request();
    kick_timer();
    FEEDER.wake();
    ticket
}

/// Asks the running move to decelerate to a stop; see `AbortHandle::request_stop`.
pub(crate) fn request_stop() {
    STOPS.fetch_add(1, Ordering::AcqRel);
    FEEDER.wake();
}

pub(crate) fn stop_generation() -> u32 {
    STOPS.load(Ordering::Acquire)
}

pub(crate) fn aborting() -> bool {
    ABORTS.pending()
}

pub(crate) fn abort_generation() -> AbortTicket {
    ABORTS.requested()
}

pub(crate) fn abort_state() -> AbortState {
    ABORTS.state()
}

pub(crate) async fn await_abort(ticket: AbortTicket) {
    while !ABORTS.is_acknowledged(ticket) {
        Timer::after(EmbassyDuration::from_millis(1)).await;
    }
}

pub(crate) fn position() -> [i64; AXES] {
    loop {
        let before = POSITION_SEQUENCE.load(Ordering::Acquire);
        if before & 1 != 0 {
            continue;
        }
        let position = core::array::from_fn(|axis| POSITION[axis].load());
        fence(Ordering::Acquire);
        let after = POSITION_SEQUENCE.load(Ordering::Relaxed);
        if before == after {
            return position;
        }
    }
}

pub(crate) fn underrun() -> bool {
    UNDERRUN.load(Ordering::Acquire)
}

pub(crate) fn engine_fault() -> bool {
    ENGINE_FAULT.load(Ordering::Acquire)
}

impl Stepper {
    pub(crate) fn tick_hz(&self) -> u32 {
        self.tick_hz
    }

    pub(crate) fn clear_fault(&mut self) -> bool {
        if !self.drained() || aborting() {
            return false;
        }
        UNDERRUN.store(false, Ordering::Release);
        ENGINE_FAULT.store(false, Ordering::Release);
        true
    }

    pub(crate) fn enqueue(
        &mut self,
        segment: Segment,
        generation: AbortTicket,
    ) -> Result<(), Segment> {
        let result = self
            .segments
            .enqueue(QueuedSegment {
                segment,
                generation,
            })
            .map_err(|queued| queued.segment);
        if result.is_ok() {
            kick_timer();
        }
        result
    }

    /// `None` unless the queue is drained and no abort is pending, so the ISR
    /// never writes these pins while they are held.
    pub(crate) fn hold_lines(&mut self, axis: usize) -> Option<HeldLines<'_>> {
        if axis >= AXES || !self.drained() || aborting() {
            return None;
        }
        let (step, dir) = (self.step_bits[axis], self.dir_bits[axis]);
        let saved = GPIO::regs().out().read().bits() & (step | dir);
        Some(HeldLines {
            _stepper: self,
            step,
            dir,
            saved,
        })
    }

    /// True once the queue has drained and the engine has finished its segment.
    pub(crate) fn drained(&self) -> bool {
        self.segments.is_empty() && IDLE.load(Ordering::Acquire)
    }

    /// Resolves once the ISR has freed room in the queue, finished the move, or
    /// an abort is pending.
    pub(crate) async fn room(&self) {
        self.wake_when(true).await
    }

    /// Resolves once the ISR has finished the move or an abort is pending. Waiting
    /// for room instead would return at once and never yield to the executor.
    pub(crate) async fn finish(&self) {
        self.wake_when(false).await
    }

    async fn wake_when(&self, room: bool) {
        poll_fn(|context| {
            FEEDER.register(context.waker());

            if (room && self.segments.ready()) || self.drained() || aborting() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await
    }
}

#[handler(priority = Priority::Priority3)]
#[ram]
fn on_tick() {
    // Safety: see INTERRUPT. This is the only reader.
    let Some(state) = (unsafe { (*core::ptr::addr_of_mut!(INTERRUPT)).as_mut() }) else {
        return;
    };
    state.rearm();

    // The TMC2209 needs at least 100 ns of STEP high and sets no upper bound, so
    // holding the pulse until the next tick is legal.
    clear(core::mem::take(&mut state.pulsed));

    let abort_request = ABORTS.requested();
    if ABORTS.pending() {
        state.discard();
        ABORTS.acknowledge(abort_request);
        state.go_idle();
        return;
    }

    while state.engine.idle() {
        match state.segments.dequeue() {
            Some(queued) if queued.generation != ABORTS.requested() => {
                FEEDER.wake();
                continue;
            }
            Some(queued) => {
                let segment = queued.segment;
                IDLE.store(false, Ordering::Release);
                if segment.start.is_some() {
                    state.active_move = true;
                }
                state.move_ending = segment.last;
                if state.engine.load(segment).is_err() {
                    state.fault();
                    return;
                }
                if state.segments.len() <= REFILL_AT {
                    FEEDER.wake();
                }
                break;
            }
            None => {
                if state.active_move && !state.move_ending {
                    UNDERRUN.store(true, Ordering::Release);
                    state.engine.abort();
                }
                state.go_idle();
                return;
            }
        }
    }

    let Ok(tick) = state.engine.tick() else {
        state.fault();
        return;
    };

    // Direction changes only at the start of a move, so this stays off the hot path.
    if tick.dir != state.dir {
        let forward = Bits::mask(&state.bits.dir, tick.dir);
        set(forward);
        clear(state.bits.dir_all & !forward);
        state.dir = tick.dir;
    }

    // Most ticks step nothing, so the mask computation stays off that path.
    if tick.step != 0 {
        let pulse = Bits::mask(&state.bits.step, tick.step);
        set(pulse);
        state.pulsed = pulse;

        publish_position(state.engine.position());
    }
}
