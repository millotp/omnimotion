# Omnimotion firmware

Motion control for the Omnimotion board: an ESP32-S3-WROOM-1-N16R8 driving three
TMC2209 axes (M0-M2) over a shared single-wire UART bus. Endstops, DIAG, e-stop
and the driver enable are all on direct GPIOs — there is no port expander and no
external ADC.

The e-stop is a 2-pole button: one pole tells firmware, the other sits in the
TMC2209 ENN return so the drivers release in copper regardless of what firmware
is doing. This is a hobby-grade stop, not a certified safety function.

The board this drives — schematic, bill of materials, and the reasoning behind
both — is in [`../hardware/`](../hardware/README.md).

## Quick start

```rust
#[embassy_executor::task]
async fn board_task(pins: BoardPins) {
    let mut board = Board::start(pins, Config::default())
        .await
        .expect("board startup");
    println!("{}", board.inventory);

    board.motion.jog(0, 20.0).await.unwrap();
}
```

`Board::start` surveys what is fitted, checks the e-stop, configures every
driver that answered, zeroes the axes and enables the outputs. `Config::default()`
is a 200-step motor on 16 microsteps driving a 40 mm/rev belt with no endstops
fitted, so override only what differs:

```rust
Config {
    axes: [AxisConfig {
        steps_per_unit: steps_per_mm(200, 16, 40.0).unwrap(),
        max_velocity: 60.0,
        max_acceleration: 600.0,
        min_velocity: 2.0,
    }; AXES],
    driver: DriverConfig {
        run_current_ma: 700.0,
        ..DriverConfig::default()
    },
    ..Config::default()
}
```

Use `Board::new` instead if you want the survey without the checks, the driver
configuration or the enable. `board.enable(&config)` does the rest later and can
be retried, which `start` cannot: on error `start` drops the board, and the step
ISR can only be installed once. So a machine that may boot on USB alone, or with
the e-stop open, should call `new` and retry `enable` until it succeeds.
`board.configure(&config)` is `enable` without turning the outputs on, for a
machine that must not hold or move its motors at boot; `faults.enable_drivers()`
turns them on when a command needs them.

## Two workspaces

| Path | Toolchain | Contents |
| --- | --- | --- |
| `software/` | host stable | `omnimotion` — planner, step engine, units, endstop debounce, TMC2209 driver |
| `software/firmware/` | `esp` (Xtensa) | `omnimotion-board` and the examples |

The split exists so the motion logic can be tested without installing a 1 GB
cross toolchain. Nothing in `software/` depends on `esp-hal`; the planner and the
step engine are plain integer arithmetic and run natively:

```bash
cd software && cargo test
```

That covers the parts worth testing: the step engine is driven tick by tick to
assert exact arrival and that no axis ever drifts more than one step from its
share of the move, the TMC2209 driver is exercised against a fake device that
echoes on the wire like the real half-duplex bus, and the current calculation is
pinned to the fitted 0.11 Ω shunts. Hardware is rated for at most 1.4 A RMS
per axis continuously, subject to thermal validation in the finished enclosure.

### Module layout

| `omnimotion` | `omnimotion-board` | Concern |
| --- | --- | --- |
| `motion/` planner, profile, engine, units, rotary, abort | `motion/` `Motion`, stepper, homing, supervisor | Moving the axes and stopping them |
| `machine/` lifecycle, control, persistence | `control`, `storage/` NVS, persistence, web assets, flash lock | Job state, commands and what survives a reboot |
| `devices/` tmc2209, endstop, power, check | `diag/` inventory, check; `pins` | The parts on the board |
| `update/` ota, web | `net/` WiFi and the update endpoint, OTA slots | Getting new firmware and UI onto the board |

`omnimotion-board`'s root holds the entry points: `board` (`Board`, `Config`),
`boot` and `pins` (`BoardPins`, `split!`).

## Building the firmware

The ESP32-S3 is Xtensa, so it needs Espressif's Rust fork. No nightly required.

```bash
cargo install espup --locked
espup install                 # installs the "esp" toolchain
. ~/export-esp.sh             # add to your shell profile
cargo install espflash --locked
```

Then:

```bash
cd software/firmware
cargo build --release
cargo run --release --example jog    # flashes and opens a monitor
```

Examples are `jog` (single axis, start here on a new board), `home` (both
homing modes), `coordinated` (all three axes), `cross_core_abort`,
`control_plane`, `persistence`, `ota` (WiFi and over-the-air updates), and
`check` (wiring self-check, for bring-up).

`rust-toolchain.toml` selects the `esp` channel and `.cargo/config.toml` sets the
`xtensa-esp32s3-none-elf` target, so no `+esp` is needed once espup has run. That
config also enables `build-std`, because the esp toolchain ships `rust-src`
rather than a prebuilt `core` for Xtensa.

## Flash layout and stored data

`partitions.csv` describes the 16 MB flash. The runner passes it to espflash,
because espflash's built-in default gives `nvs` only 24 kB.

| Partition | Offset | Size | Holds |
| --- | --- | --- | --- |
| `nvs` | `0x9000` | 128 kB | Settings and checkpoints |
| `web` | `0x29000` | 128 kB | Gzipped web assets, written over USB or WiFi |
| `phy_init` | `0x49000` | 4 kB | Radio calibration |
| `otadata` | `0x4A000` | 8 kB | Which app slot boots, and whether it is on trial |
| `ota_0` | `0x50000` | 4 MB | The application, when flashed over USB |
| `ota_1` | `0x450000` | 4 MB | The other slot for over-the-air updates |

There is no factory slot. The runner flashes `ota_0` and erases `otadata`, so a
USB flash always boots what it just wrote, even on a board that an update had
moved to `ota_1`. A board flashed with the older single-`factory` table keeps
its `nvs` and `web` contents across the change, since neither moved.

`storage::Store` finds `nvs` by label when it walks that table, so nothing
hardcodes an offset. Its raw blob API takes names of at most 15 bytes, which is
all NVS allows. Settings and checkpoints should go through
`storage::persistence::TransactionalStore` rather than raw blobs.

Transactional records have a fixed header containing `OMNI`, record kind,
schema version, a monotonically increasing `u64` generation, payload length,
and a CRC-32 over the header and payload. Each logical record alternates between
two NVS keys. Loading chooses the complete slot with the greatest generation,
so an interrupted update leaves the previous slot recoverable. A complete
record with an unsupported schema, a corrupt pair, and a missing pair are
reported separately; no case silently becomes application defaults. Generation
exhaustion is also an error rather than wrapping. Postcard migration and
payload validation remain application-owned: load with the old schema version,
convert, and save with the new one. A save may replace an older stored schema,
but never a newer one, so old firmware cannot clobber a migrated record.

Web assets are packed into their own `web` partition, since they change with
the UI rather than the firmware, and flashing an image replaces the whole
partition it lands in:

```bash
cargo install esp-nvs-partition-tool --locked
python3 tools/pack_web.py path/to/web    # gzips into build/web.bin
espflash write-bin 0x29000 build/web.bin # the script prints this command
```

The script reads the `web` offset and size from `partitions.csv`. Everything is
gzipped, so the firmware serves every asset with a fixed
`Content-Encoding: gzip` and the browser does the decompressing. A UI of 44 kB
compresses to under 10 kB, which is 8% of the partition. Assets are stored as
blobs rather than strings because NVS caps a string at 4000 bytes, while a blob
spans pages and can reach roughly 500 kB.

The same image can arrive over WiFi, with `POST /web` on the update port. The
body must be exactly the size of the `web` partition, and every page is checked
as NVS before its sector is erased. A rejected page stops the upload with the
earlier pages already written, so the UI must then be uploaded again. The
application and `nvs` are untouched, and the board reboots once the upload is
complete. Upload with:

```bash
python3 tools/ota.py --web build/web.bin 192.168.1.42
```

Writing to `nvs` at runtime suspends the flash cache, so `Store` parks the other
core for the duration of each write. The step ISR lives in IRAM, but the
planner, supervisor, and application can stall for the erase.
`TransactionalStore::save` therefore requires `&Motion` and `&Lifecycle` and
refuses the write unless the segment queue is drained, no abort is pending, and
the lifecycle is `Idle` or `Paused` at an application-declared safe boundary.
Host or HTTP tasks must enqueue an update request; they must never write flash
directly. The one exception is the board's own update endpoint, described
below, which keeps motion out by other means.

Two things are expected and harmless: the release build is much faster than the
dev build and esp-hal will nag if you forget `--release`, and the linker warns
that the image "has a LOAD segment with RWX permissions", which is simply how
ESP32 IRAM works.

`cargo clippy --all-targets` does not work in the firmware workspace: it tries to
build a test harness for a bare-metal target, which cannot link. Use
`--lib --examples`.

## WiFi and over-the-air updates

`net::start` joins WiFi, takes an address over DHCP, and serves firmware
updates on port 8080. The network name, its password and an update token are
compile-time environment variables, so the image you flash over USB joins the
network on its own and every later build can arrive over WiFi:

```bash
export OMNIMOTION_WIFI_SSID='network'
export OMNIMOTION_WIFI_PASSWORD='password'
export OMNIMOTION_OTA_TOKEN='change-me'

cargo run --release --example ota        # first time: over USB
cargo build --release --example ota      # every time after that
python3 tools/ota.py target/xtensa-esp32s3-none-elf/release/examples/ota 192.168.1.42
```

`start` panics if any of the three is unset, so a build that could never be
updated again is caught on the bench. The script's host argument is the
board's IP address, which the `ota` example prints when it gets one.

Call `start` from the first core's executor, after `boot_on_second_core`.
esp-radio needs the scheduler running, and the WiFi interrupts belong on the
core that is not generating steps. It returns the `embassy_net::Stack` so the
application can serve its own sockets on it: `net::SOCKETS` leaves four
free.

The update port also serves `POST /web` for the web partition, described
above. Both take the token in `X-API-Key`.

The endpoint is `POST /ota` with the app image as the body, its exact
`Content-Length`, and the token in `X-API-Key`. The image is an
`espflash save-image` output, not the ELF; the script does that conversion. The
board refuses a body that is not an ESP32-S3 app image before erasing anything,
writes the rest to the inactive slot one sector at a time, selects it, answers
`200`, and reboots.

### A bad update rolls back by itself

espflash's bundled bootloader has no rollback support, but it does skip a slot
whose otadata entry reads `Invalid`, so the firmware runs the trial itself:

1. The update is selected as `New`.
2. On its first boot, `net::start` marks it `PendingVerify` before doing
   anything else.
3. Once it has an IP address, which proves it can take the next update, it
   marks itself `Valid`.
4. If it is still waiting after two minutes, it marks itself `Invalid` and
   resets. If it resets or loses power first, the next boot finds
   `PendingVerify` and does the same. Either way the bootloader falls back to
   the previous slot. esp-backtrace halts on a panic rather than resetting,
   so an image that panics during its trial waits for a power cycle, then
   rolls back.

So an image built without the WiFi variables, or with the wrong ones, costs a
reboot rather than a USB cable. An image that never calls `net::start` is
never confirmed or rejected and keeps booting; only USB recovers it. Do not
pair this with a bootloader built with `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`:
that bootloader sets `PendingVerify` itself, so every update would look failed.

### Motion and the flash

Every flash erase stalls both cores for tens of milliseconds. Interrupts are
masked on the core doing the erase, and the other core is parked. A stall in
the middle of a move underruns the step queue. So for as long as an upload
runs, it holds a lease that makes `Motion::move_by_with` (and every move built
on it, homing included) fail with `MotionError::Busy`. An upload that arrives
while a move is running gets `409` and nothing is erased. The single-sector
otadata updates of the trial take a brief lease instead: a move that starts
meanwhile waits a few milliseconds rather than failing. The drivers keep their
state throughout, so a machine holding position stays held. The e-stop's ENN
pole works in copper and is unaffected.

`Store` and the update path share the chip through one lock, so an NVS write
and an image write never overlap.

There is no TLS. The token stops casual mistakes on a home network; it is not
protection against anyone who can watch the traffic.

## Version pinning

`esp-hal` 1.1.2 is the newest stable release, and it constrains everything that
links against it. `esp-backtrace` 0.19, `esp-println` 0.17 and
`esp-bootloader-esp-idf` 0.5 are the last versions on that line; their 0.20 /
0.18 / 0.6 releases require the `esp-hal` 1.2.0-rc prerelease. Upgrade them as a
set or not at all.

## Dependencies

Used as-is:

| Crate | Why |
| --- | --- |
| `esp-hal` 1.1.2 | Bare-metal HAL. The `unstable` feature is required for `timer`. |
| `esp-rtos` 0.3 | Embassy time driver and executor integration. Supersedes `esp-hal-embassy`, which stopped at esp-hal `1.0.0-rc` and cannot resolve against 1.1.2. |
| `tmc2209` 0.3 | Zero-dependency `no_std` register map, CRC, datagrams, and `rms_current_to_vsense_cs`. |
| `heapless` | SPSC segment queue between the planner task and the step ISR. |
| `libm` | `sqrtf` for the trapezoid, since `no_std` has no float math. |
| `crc` | `no_std` CRC-32 for versioned settings and checkpoint envelopes. |
| `embassy-sync` | `AtomicWaker`, so the step ISR can wake the feeder task directly. |
| `embassy-executor` 0.10 | `boot` hands out a `Spawner`, so this is public API, not just an example dependency. The version must match the one `esp-rtos` resolves, or the two `Spawner` types do not unify. |
| `embassy-futures` | Executor-agnostic `select`, used to bound TMC reads without pulling a time driver into a host-testable crate. |
| `embedded-hal-async`, `embedded-io-async` | Driver trait boundaries, so the TMC2209 driver stays HAL-agnostic and host-testable. The OLED probe is generic over `I2c` too, which is why `omnimotion-board` depends on `embedded-hal-async` directly. |
| `esp-nvs` | Reads and writes the ESP-IDF NVS format in pure Rust, so settings stay compatible with Espressif's tools without linking ESP-IDF. |
| `esp-storage` | The flash driver underneath `esp-nvs`, including parking the other core while a write runs. |
| `esp-bootloader-esp-idf` | Partition-table parsing, so `Store` finds `nvs` by label instead of hardcoding an offset; the otadata reader and writer behind updates; also supplies the app descriptor the bootloader requires. Its `OtaUpdater` is not used: with blank otadata it can pick the running slot as the target, so the target is taken from the slot the MMU says is booted. |
| `esp-alloc` | `esp-nvs` allocates its page cache, and `init_heap` installs a heap for applications that have none, plus 64 kB the bootloader hands back, which esp-radio's WiFi buffers need. |
| `esp-radio` 0.18 | The WiFi driver. 0.18 is the line that resolves against `esp-hal` 1.1.2 and `esp-rtos` 0.3. |
| `embassy-net` | TCP/IP and DHCP over esp-radio, and the stack applications open their own sockets on. |
| `embedded-storage` | The `NorFlash` trait, so image sectors are erased and written explicitly instead of through the read-modify-write `Storage` path. |
| `serde`, `postcard` | Compact, `no_std` payload encoding for `TransactionalStore`. Not self-describing, which is why records carry a schema version. |
| `static_cell` | `'static` storage for the executors, the segment queue and the flash handle without `static mut`. |

Considered and rejected:

| Crate | Reason |
| --- | --- |
| `ramp-maker` | Unmaintained, and its fixed-point generics fight the segment representation. The trapezoid is ~100 lines of integer math we can unit test. |
| `stepper` 0.6 | `embedded-hal` 0.2 era, stale. |

## How motion works

A fixed-rate timer tick (default 40 kHz) on TIMG0 runs the step engine, with the
handler in IRAM so flash cache misses cannot stall it. Embassy's time driver sits
on SYSTIMER so the two never contend.

Each tick advances a 32-bit NCO by the current segment's rate. An overflow is one
master step, which Bresenham distributes across the three axes. Positions are
counted as steps happen, so they stay exact even if a move is aborted mid-flight
by an endstop or a stall.

Moves are sliced into **segments** of roughly 1 ms, each carrying a constant rate
and an exact master step count. A task fills a 16-deep SPSC queue that the ISR
drains, so memory does not depend on move length. The ISR wakes the feeder as the
queue empties rather than the feeder polling for room.

The tick is written to touch as little hardware as possible, because it runs
40 000 times a second and its cost is what caps the step rate. Step and direction
flags are carried as one bit per axis, which the ISR turns into a single write to
the GPIO `w1ts`/`w1tc` bank register, so all three step edges land on the same
cycle instead of being staggered. Direction is only written when it changes, the
previous pulse is only retired when there was one, and the position counters are
only stored on a tick that actually stepped. A tick that emits no step touches
the hardware exactly twice, both times to re-arm the timer.

Ceiling is one step per tick on the dominant axis: 40 k steps/s by default, about
750 rpm at 1/16 microstepping. The tick rate is configurable.

## Abort and cross-core control

`Motion::abort_handle()` returns a cloneable handle that does not borrow
`Motion`. `request` returns an `AbortTicket`; `wait` completes only after the ISR
has discarded queued segments, stopped the engine, and published its exact raw
position. Requests coalesce and old tickets become acknowledged when a newer
request is acknowledged. Wrap-aware ticket comparisons are valid while a
ticket is less than half the `u32` generation space old.

An abort stops the step train dead, which a heavy load cannot follow: the rotor
overruns its field and loses steps. `request_stop` instead ends the running
move along its own deceleration ramp, after the segments already queued (about
15 ms); the move returns `Outcome::Aborted` with exact counters. It only affects
a move already running when it is called.

`ControlPlane` provides fixed-capacity command and completion queues plus a
coherent copied `StatusSnapshot`. Split it once and keep the client on core 0
and the machine endpoint beside `Board` on core 1. Queue-full is a submission
error; busy, invalid-state, unavailable-axis, fault, and invalid-command errors
are asynchronous command completions. Completion sends apply backpressure
rather than discarding an accepted request. Application commands are generic,
and homing sensor configuration stays in the application dispatcher.

## Rotary motion and jobs

`RotaryPeriod` normalizes raw signed positions and calculates the shortest
delta to an absolute modulo target. Exact half turns choose the positive
direction. `fraction_target` computes every indexed target directly as a
rational fraction of the period, avoiding accumulated rounding error.
`Motion::move_to_modulo` is the board-level convenience; the ISR's raw `i64`
counters remain unwrapped and authoritative.

`Lifecycle` owns only generic job transitions and operation counts. Applications
call `begin_operation` and `complete_operation` around their own choreography.
An immediate pause or stop remains transitional until its abort is
acknowledged. A pause additionally requires the application to declare that it
has recovered to a safe operation boundary. Stop and explicit fault recovery
discard resumable progress; a fault can never be resumed as though it were a
pause.

## Faults

`FaultSupervisor` must be polled continuously — nothing else disables the
drivers. Run it against your motion future:

```rust
match select(run(&mut board.motion), board.faults.wait_for_fault()).await {
    Either::First(()) => println!("motion stopped"),
    Either::Second(fault) => println!("fault: {fault:?}"),
}
board.faults.disable_drivers();
```

This is also the only normal watchdog feed path. The watchdog is armed while the
outputs are enabled and disarmed with them, so whenever `DRV_EN` is high
something must keep polling the supervisor, or the MCU resets after five
seconds; the hardware `DRV_EN` pull-down keeps the drivers disabled through that
reset. With the outputs disabled nothing can move, so a board that is idle,
failed to start, or is waiting for commands with its drivers off does not need
the supervisor running.

`Fault` is `EStop`, `Endstop`, `Diag`, `Underrun`, `MotionEngine` or `Driver`.
Tripping any of them latches the fault, aborts the queue and drops `DRV_EN`.
`faults.clear(&mut board.motion)` clears the latch, but only while the outputs
are already disabled, the e-stop is closed, no DIAG is asserted and no
`Limit`-role endstop is triggered.

Because endstops are direct GPIOs the supervisor is a plain 2 ms poll with no
I²C in the path, so none of it returns `Result`.

## Choosing the core

`split` divides the chip between the scheduler and the board, then one of two
entry points decides where motion runs. Both start esp-rtos on the calling core,
which is the only core esp-rtos will start on.

```rust
#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(config);
    let (rtos, pins) = omnimotion_board::split!(peripherals);
    // Everything the board does not claim is still in `peripherals`: FLASH,
    // WIFI, the OUT1/OUT2 gates on GPIO42/GPIO2, AIN on GPIO3, RGB on GPIO48.
    omnimotion_board::boot(rtos, |spawner| {
        spawner.spawn(board_task(pins).expect("board task spawned once"))
    });
}
```

`split!` is a macro rather than a function because only a local binding can be
partially moved out of; a function taking `Peripherals` would have to drop
everything it does not return.

`boot` runs the board's executor on the calling core and never returns. Use it
when nothing else needs the chip.

`boot_on_second_core` runs the board's executor on the second core and *returns*,
leaving the first core to the application:

```rust
omnimotion_board::boot_on_second_core(rtos, |spawner| {
    spawner.spawn(board_task(pins).expect("board task spawned once"))
});

// Still on core 0, with the board running on core 1.
static EXECUTOR: StaticCell<Executor> = StaticCell::new();
EXECUTOR.init(Executor::new()).run(|spawner| { /* network, host link, UI */ })
```

The split is worth understanding: the tick ISR is installed by whichever core
calls `stepper::start`, which `Board::new` does. So building the `Board` inside
`init` is what pins step generation to that core — there is no separate knob for
it, and constructing the board on the wrong core silently puts the ISR there.
Queue refill lands on the same core too, since it is polled by that executor.

This matters because the tick has a 25 µs budget at the default 40 kHz and queue
underrun is a latching fault. Anything interrupt-heavy — a WiFi stack especially
— is much safer on the other core. That is also why `split` exists at all: the
scheduler has to come up on the first core, so the board's peripherals have to be
separable from the scheduler's in order to be handed elsewhere.

## Endstops

`Config::endstops` takes one `EndstopConfig` per connector, J11–J16 in
E0–E5 order, so a machine can mix sensor types across its inputs:

```rust
endstops: [
    EndstopConfig::HALL,             // E0
    EndstopConfig::UNUSED,           // E1
    EndstopConfig::SWITCH_REFERENCE, // E2: homed against, not guarded
    EndstopConfig::UNUSED,           // E3
    EndstopConfig::SWITCH,           // E4
    EndstopConfig::SWITCH,           // E5
],
```

The two sensor types are electrically identical — both idle high through their
pull-up and pull low — so the type only changes how long a level must hold before
the supervisor believes it. A contact needs three agreeing 2 ms samples because it
bounces; a hall output is solid state and the 100 µs input RC has already
settled it, so one sample is enough. That makes a hall endstop trip roughly 4 ms
sooner than a switch.

`UNUSED` ignores the pin entirely, which is the default. Declaring `SWITCH` on a
connector with nothing wired to it is also harmless, since the input idles high.

### Limits and references

A connector's `EndstopRole` decides what the supervisor does when it triggers,
independently of the sensor type:

| Role | Meaning | On trigger |
| --- | --- | --- |
| `Limit` (default) | The axis has travelled somewhere it should not have. | Latches `Fault::Endstop` and disables the drivers. |
| `Reference` | A datum the machine drives into on purpose. | Nothing. Readable, and usable as a homing trigger. |

Machines that home on a switch need `Reference`, because a limit would fault the
instant homing succeeded. So does any axis whose datum is crossed during normal
work — an index mark on a rotary axis passes under its sensor every revolution,
and as a `Limit` that would stop the board on every turn. `SWITCH_REFERENCE`
and `HALL_REFERENCE` are the shipped shorthands.

References also do not block fault recovery, so a machine parked on its datum can
still clear a latch, which a `Limit` deliberately prevents.

Set `active_low: false` for normally-closed wiring, where the input idles low
and a cut cable then trips. Note the asymmetry: a hall sensor that loses power
reads as released, so if you want a broken sensor cable to stop the machine,
mount the magnet present-in-the-safe-zone and invert that endstop.

Hall sensors must run from 3.3 V and hold V_OL under 0.40 V;
[`../hardware/DESIGN_NOTES.md`](../hardware/DESIGN_NOTES.md) works through why
and names a verified part.

## Homing

`motion::homing::home` runs one pass per call. `HomingSensor` chooses what ends it:

```rust
// Sensorless: push into the frame and let the driver notice the load.
HomingSensor::StallGuard { current_ma: 450.0, threshold: 90 }

// Or against a switch or hall sensor on one of J11–J16.
HomingSensor::Endstop { endstop: 2 }
```

Everything else in `HomingConfig` — signed `travel` whose sign is the direction,
`rate`, `accel`, `backoff`, all in the axis's own units like every other move —
is shared, because the geometry of the pass does not depend on how the stop
announces itself. Either way the axis backs off the stop and zeroes at the
backoff point, not on the stop itself.

A stall aborts the pass at once, since the motor is pushing the frame. An
endstop trip records the position and ramps down at `accel`, so a heavy axis can
home fast without losing steps; the backoff is measured from the recorded trip,
not from where the ramp ended. The pass starts and ends at a quarter of `rate`,
so keep that within what the axis can start from rest.

The two paths differ in what they cost. A StallGuard pass has to reconfigure the
driver (reduced current, StealthChop forced at every velocity because StallGuard4
measures nothing in SpreadCycle, `SGTHRS`, a CoolStep velocity gate armed from
half the homing rate), wait out a 2 s settle so a stale stall flag cannot trip
it, then put all of that back and interrogate the driver afterwards to check the
load was the frame rather than a short or an overtemperature. The microstep
setting for the velocity gate comes from the `DriverConfig` passed to `home`. An
endstop pass touches the driver not at all: it homes at the axis's running
current and starts moving immediately.

Choose sensorless when there is a hard end of travel and you would rather not
fit a sensor. Choose an endstop when there is a switch already, when the datum
must be repeatable without tuning, or when the axis has no end of travel to crash
into at all — a continuously rotating axis gives StallGuard nothing to detect, so
an index sensor is the only option.

`home` returns `Homed::Found` or `Homed::NotFound`, the latter meaning the whole
travel budget ran out; it also disables the outputs, which
`faults.enable_drivers()` turns back on. An endstop pass that begins already
sitting on its datum returns `HomingError::PreassertedEndstop` without latching
anything, since nothing has moved and nothing is wrong: step off the datum and
call again. One that is still on its datum after backing off returns
`HomingError::BackoffTooShort` without zeroing. An abort request during the pass
returns `HomingError::Aborted` without latching a fault.

## Startup inventory

The board is modular: drivers, the OLED and the bulk supply are all optional.
`Board::new` surveys what is actually fitted and leaves the result in
`board.inventory`, a plain `Copy` struct that also implements `Display`:

```text
board inventory
  power      bulk 23.99 V
  e-stop     closed
  oled       0x3c responding
  drivers    M0 tmc2209, M1 tmc2209, M2 silent
```

`power` is the answer to "USB or bulk?", read from the VMOTOR monitor divider on
GPIO1 using the chip's own calibrated ADC. `PowerSource` distinguishes `UsbOnly`,
`Bulk`, and `BulkOutOfRange`. The supported input is 12–24 V; firmware uses an
11–26 V motor-ready window to allow supply and ADC tolerance.
`Inventory::can_run_motors` folds that together with the e-stop into one go/no-go.
`board.vmon.read()` re-reads it at any time.

The OLED probe is an address-only write, because an SSD1306 has no register worth
reading and writing to it would change the display. It reports just the address
that answered — modules strap either 0x3C or 0x3D. The I²C bus stays available as
`board.i2c` for your own display driver; nothing else is on it.

`drivers` is filled in during `Board::new`. VMOTOR is not switched on this
board, so with bulk power present a TMC2209 can answer over UART from boot; with
USB alone it cannot, which is why `enable` surveys again. `Responding` means the driver returned the 0x21 a TMC2209 reports in
`IOIN.version`; `Missing` cannot tell an empty socket from a dead motor rail,
which is why the power reading is worth reading first. `Board::enable`, which
`Board::start` calls, re-surveys the drivers once power is confirmed and
configures only the axes that answered. It refreshes VMON and the e-stop before
and after driver configuration, then enables the shared ENN only while the
supply is in the motor-ready window. USB-only and out-of-range power return
`StartError::PowerNotReady`; no replies on otherwise valid bulk power return
`StartError::NoDrivers`, and a non-0x21 reply returns
`StartError::DriverMismatch`. Motion commands that include a missing or
mismatched axis return `MotionError::AxisUnavailable`.
`board.survey_drivers()` re-probes later and can remove a newly silent axis from
the available set; it never makes a newly discovered, unconfigured axis runnable.

## Wiring self-check

The inventory says what is fitted; `board.check()` says whether it is wired
right. It never energises a motor, so it needs the outputs disabled, which is
how `Board::new` leaves them, and refuses with `CheckError::DriversEnabled`
otherwise. Flash the `check` example during bring-up and after any rewiring:

```text
board check
  power   ok   bulk 23.99 V
  e-stop  ok   closed
  oled    --   0x3c responding
  E0      ok   switch, clear
  E1      warn hall, triggered at rest; parked on it, or a cut NC cable
  E2      --   unused, idle
  ...
  M0      ok   step, dir, diag, enable, straps, charge pump, temperature
  M1      FAIL DIR not seen by driver
  M2      --   silent, not fitted?
result: 1 failure(s), 1 warning(s)
```

The driver checks work because a TMC2209 reports the level on its own STEP,
DIR, ENN, DIAG and MS1/MS2 pins in `IOIN`. The check drives STEP and DIR low,
reads them back over UART, drives them high, reads again, and puts both back as
they were. So for each driver that answers:

- STEP and DIR must follow, which proves both traces end to end.
- ENN must read inactive while firmware commands the outputs off. If it reads
  active, Q1 or the ENN net is holding the drivers on, which is the failure
  that matters most.
- DIAG must read the same on both sides. Both idle low, so this catches a
  shorted or stuck line but not an open one.
- MS1/MS2 must match the address the driver answered on.
- `GSTAT` must show no charge-pump undervoltage, and `DRV_STATUS` no
  overtemperature warning or shutdown.

Each STEP line gets one rising edge, which moves nothing with ENN inactive.

Endstops are judged against `Config::endstops`, so fill that in first. An
unused connector held low fails, because something is plugged in that the
firmware does not know about. A fitted endstop that reads triggered at rest is
a warning: the machine may be parked on its datum, or a normally-closed cable
may be cut. A normally-open switch with a cut cable reads clear, so that fault
is invisible to any check. Power out of range, an open e-stop, a driver that
answers with the wrong version, and one that stops answering part way through
are failures. A silent driver socket is only reported, because the board is
meant to run partly populated. On USB power the drivers cannot answer, so they
are reported unchecked and the result carries a warning.

Some things are outside the check's reach. The 3V3 and 5V rails, the USB
handover flag at TP15, the OUT1/OUT2 loads, the RGB LED and the motor coils
have no feedback path the firmware can read. They stay on the bring-up list on
schematic sheet 9. Open and shorted coils would need the motors energised,
which the check deliberately never does.

The judgement and the report text live in `omnimotion::devices::check` and are tested
on the host. `board.check()` only gathers the readings, and it changes neither
`board.inventory` nor the set of available axes.

## Board-level operating limits

- J3 is a 2-pole normally-closed e-stop. Pole B is in the ENN return, so opening
  it releases the drivers with no firmware involvement. Linking J3 bypasses it
  entirely, and without the B link the drivers never enable.
- The motor rail is **not** switched. An e-stop stops the motors at the driver;
  it does not de-energise the 24 V output. If a solenoid or brake must drop
  mechanically, use a 3-pole button or an external contactor.
- Reverse polarity blows F1 rather than being blocked. F1 is a replaceable
  cartridge, so the cost is a fuse; feed the board 12–24 V and nothing else.
- There is no active overvoltage protection. The TMC2209's V_VS absolute
  maximum is 32 V and the input TVS clamps well above that under full surge, so
  supply choice is the protection: a bench supply or a good 24 V brick on a
  short lead. A long inductive feed, or a supply shared with contactors or
  motor brakes, needs surge suppression upstream of J1/J2.
- The fault supervisor feeds a five-second ESP watchdog, armed only while the
  outputs are enabled. Watchdog and panic recovery still depend on rebooting.
- DIAG inputs are pulled down by R62–R64 (100 kΩ) and by the GPIO's internal
  pull, so an unfitted driver channel reads healthy rather than floating into
  `Fault::Diag`.
- Each endstop is declared at setup as a dry contact, a 3.3 V open-drain hall
  sensor, or unused, with polarity and role per connector.
- USB-only power has a 500 mA total design budget; its PTC is not a precision
  limiter. Do not run motors or external loads without the 12–24 V input.
- The 0.11 Ω shunts are 2512, 1 W parts; configure no more than 1.4 A RMS per
  axis continuously and validate driver temperatures under worst-case load.
