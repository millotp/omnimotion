# Omnimotion — Universal ESP32-S3 Motion Controller, Rev A.1

Design notes for the 3-axis motion-control motherboard: one reusable board for
the self-playing chessboard, the zen garden, the line-art drawing machine and
whatever comes after them.

This is the deep dive. [`README.md`](README.md) has the board overview,
specifications, connector list and build instructions.

The board is built around a deliberate simplicity constraint: a hobbyist has to
be able to hand-solder it, source it from one distributor order, and reason
about it without a block diagram. Five IC part numbers, one FET part number,
one Schottky part number. ["Known risks and open items"](#known-risks-and-open-items) is the
honest accounting of what that constraint costs.

## Deliverables in this folder

| File | Contents |
| --- | --- |
| `generated/omnimotion_schematic.svg` | 9-sheet schematic, 1840 × 1400 px per sheet |
| `generated/omnimotion_BOM.csv` | 48 BOM lines (one table), 174 placed parts plus 15 bare test pads, 0603 minimum, home-fab kit |
| `scripts/gen_schematic.py` | Generator for the schematic (source of truth) |
| `scripts/gen_bom.py` | Harvests the BOM from the generator, so the two cannot drift apart |
| `scripts/schlib.py` | Minimal SVG schematic-symbol library |

Regenerate both deliverables with:

```bash
python3 scripts/gen_schematic.py && python3 scripts/gen_bom.py
```

Add `--check` to either script to fail on a stale committed file instead of
rewriting it. The generator refuses an ambiguous drawing (unconnected terminals
or wire ends, wires through symbols, T-junctions without dots), and `gen_bom.py`
fails loudly on a duplicate reference designator or a component value that has
no BOM entry, so the BOM is always a complete reflection of the drawing.

## Sheet index

| Sheet | Contents |
| --- | --- |
| 1 | Architecture, power tree, absolute-maximum budget, layout rules, key decisions |
| 2 | DC input protection: fuse, TVS, 2-pole e-stop, VMOTOR monitor |
| 3 | LMR33630 5 V buck, TPS2116 priority mux, AMS1117 3.3 V LDO |
| 4 | ESP32-S3-WROOM-1-N16R8, USB-C, boot/reset, pin-allocation rules |
| 5 | Axis M0 TMC2209 channel + shared UART bus + ENN fail-safe chain + DIAG pull-downs |
| 6 | Axis M1 channel + per-channel BOM and thermal budget |
| 7 | Axis M2 channel + motor wiring and stall-detection notes |
| 8 | Six direct-GPIO endstops, J17 analogue input, OLED and I²C headers |
| 9 | Two low-side outputs, WS2812, rail LEDs, test points, bring-up order |

## The whole board in five IC part numbers

That is the headline of the whole design: seven IC packages from five part
numbers, plus the USBLC6-2 ESD array, which the BOM files as a diode (D5).

| Ref | Part | Job |
| --- | --- | --- |
| U1 | LMR33630ADDA | 5 V / 3 A buck off VMOTOR |
| U2 | TPS2116 | 2.5 A priority mux: buck wins, USB is the fallback |
| U3 | AMS1117-3.3 | 3.3 V linear regulator |
| U4 | ESP32-S3-WROOM-1-N16R8 | MCU |
| U5 U6 U7 | TMC2209-LA-T | The three axes |

Everything else is discretes: one TVS, the USBLC6-2, three SS36 Schottkys,
three AO3400A FETs, four LEDs, and passives. There is exactly one FET part
number and one Schottky part number on the whole board, which is the point.

## Datasheets used, and what each one changed

Every value in the schematic traces back to one of these documents. The
"verified" column lists what was actually checked, not just what was read.
The datasheets are the manufacturers' own and are linked rather than copied
into the repository. Note that littelfuse.com refuses non-browser downloads,
so the SMCJ26A has to be opened in a browser.

| Part | Datasheet | Verified |
| --- | --- | --- |
| ESP32-S3-WROOM-1 | <https://www.espressif.com/sites/default/files/documentation/esp32-s3-wroom-1_wroom-1u_datasheet_en.pdf> | Module pinout, which GPIOs the N16R8 octal PSRAM consumes, strapping-pin defaults, V_IL / V_IH, ADC1 channel map, antenna keepout |
| TMC2209 | <https://www.analog.com/media/en/technical-documentation/data-sheets/TMC2209_datasheet.pdf> | V_VS limits, standard application circuit (section 3.1), charge pump, 5VOUT loading, UART addressing, sense-resistor sizing, ENN polarity |
| LMR33630 | <https://www.ti.com/lit/ds/symlink/lmr33630.pdf> | Recommended component table, feedback divider, C_BOOT, EN/UVLO divider, input HF bypass, PGOOD, layout |
| TPS2116 | <https://www.ti.com/lit/ds/symlink/tps2116.pdf> | 6 V absolute maximum, 2.5 A, MODE pin for automatic priority, PR1 threshold against the 1.0 V internal reference, ST open-drain polarity |
| AMS1117 | <http://www.advanced-monolithic.com/pdf/ds1117.pdf> | Input/output capacitor requirements, tantalum recommendation, dropout, SOT-223 tab = V_OUT |
| USBLC6-2SC6 | <https://www.st.com/resource/en/datasheet/usblc6-2.pdf> | Pinout, working voltage, placement guidance |
| SMCJ26A | <https://www.littelfuse.com/assetdocs/tvs-diodes-smcj-datasheet?assetguid=37388813-0D6D-4329-969B-1AA8B7614AC1> | V_R 26 V, V_BR 28.9–31.9 V, and the 42.1 V clamp at I_PP 35.7 A that the driver margin has to live with |
| SS36 | <https://www.vishay.com/docs/88751/ss32.pdf> | 60 V / 3 A, V_F vs. I_F curve for the WS2812 feed and the output flyback energy |
| AO3400A | <http://www.aosmd.com/pdfs/datasheet/AO3400A.pdf> | 30 V V_DS, V_GS(th) 1.45 V max, R_DS(on) 31 mΩ at 4.5 V, SOT-23 dissipation |

## Where the datasheets changed the plan

1. **TMC2209 V_VS absolute maximum is 32 V**, not "somewhere near 40 V". At
   24 V nominal that is 8 V of headroom, and a 26 V-standoff TVS clamps at
   42.1 V during a surge. The board carries no ideal-diode controller and no
   active overvoltage cut, and accepts that exposure instead. See
   ["Known risks"](#known-risks-and-open-items) — this is the single biggest
   thing the simplicity constraint costs. Run the board at 12–24 V from a
   bench supply or a decent brick, not off a long inductive feed.
2. **TMC2209 single-wire UART needs TX *and* RX.** The datasheet's arrangement
   is a 1 kΩ series resistor in the TX leg with RX tapped directly off the bus.
   That costs two GPIOs, not one: GPIO16 and GPIO47.
3. **GPIO35/36/37 are unusable on an N16R8** (in-package octal PSRAM) and
   GPIO33/34 are not bonded out on WROOM-1. None appear in the pin map.
4. **Three TMC2209 on one UART is explicitly supported** through MS1_AD0 /
   MS2_AD1 address straps, so no multiplexer is needed — but each device needs
   its own 10 k straps, which the original plan did not budget.
5. **VREF must be defined at reset.** `I_scale_analog` resets to 1, meaning the
   driver uses the analogue reference until firmware clears the bit. A
   33 k / 10 k divider off 5VOUT gives a safe ~0.82 A_RMS default instead of
   whatever a floating pin picks up.
6. **WS2812 input thresholds depend on the exact revision.** Current V6 parts
   accept 3.3 V data at 5 V, while older variants can require up to
   0.7 × V_DD. D6 drops the strip supply by one Schottky drop, but that drop is
   current-dependent and is not a guaranteed level shifter. The board is
   specified for a 3.3 V-compatible strip; fit an external AHCT buffer for
   older or unknown pixels.
7. **A 2-pole e-stop needs no active circuitry.** Pole A pulls GPIO15 so
   firmware knows; pole B sits in the TMC2209 ENN return so the drivers
   release in copper. That replaced a controller, two power FETs, two signal
   FETs and a diode. See ["E-stop architecture"](#e-stop-architecture).
8. **The outputs are discrete FETs.** An AO3400A with a 1 k gate series and a
   10 k gate pull-down is a 1 A low-side switch for the cost of three
   passives, and it is the same FET as the ENN chain uses. The TBD62064AFG
   array it replaced was a whole extra part number for four channels nobody
   used.
9. **The barrel jack is a 3.5 A part** (Kycon KLDX-0202-AC, through-hole), so
   a 2.54 mm Phoenix MPT screw terminal (6 A) is fitted in parallel for the
   full three-motor load.
10. **The AMS1117 output capacitor is deliberately tantalum.** The loop wants
    some ESR and can ring on a pure MLCC.
11. **External inputs have no clamp diodes, and that is deliberate.** Every
    endstop and the J17 analogue input already carries a 1 kΩ series resistor
    and a 100 nF cap, which is a strong ESD network in its own right: an 8 kV
    human-body strike carries only 100 pF × 8 kV = 0.8 µC, which lifts a
    100 nF node by 8 V, and the 1 kΩ holds peak current to a few amps for
    roughly 150 ns. The ESP32-S3's own pin clamps absorb that comfortably.
    Dedicated SOD-323 TVS clamps on these seven lines would cost about 1 EUR
    each, a disproportionate share of the BOM for protection the RC network
    largely already provides. What they would add is miswiring tolerance,
    which is covered in the known-risks section.
12. **Each endstop is a dry contact or a hall sensor, chosen in firmware**, and
    is either a limit or a homing reference, also chosen in firmware. Both
    sensor types idle high and pull low, so the hardware is the same either
    way. Pin 1 of J11–J16 is a 3V3 branch behind F3, a 0.1 A PPTC, so a
    pinched sensor cable cannot collapse the rail under the ESP32. Never feed
    a sensor from 5 V or 24 V, and do not wire a PNP output in.

### Endstops: switch or hall sensor

The six inputs are `3V3 / GND / SIG` connectors with a 1 kΩ series resistor, a
100 nF cap and a 10 kΩ pull-up to 3.3 V, landing directly on an ESP32 GPIO.
That is the standard interface for either a dry contact or a 3-wire open-drain
sensor.

- **Switch.** Contact between pin 3 (SIG) and pin 2 (GND); pin 1 unused.
- **Hall sensor.** VCC on pin 1, GND on pin 2, open-drain OUT on pin 3.

Two constraints decide whether a given sensor works:

| Constraint | Value | Why |
| --- | --- | --- |
| Supply | must run at 3.3 V | Pin 1 is 3V3, not 5 V |
| V_OL at 0.3 mA | ≤ 0.55 V | The 1 kΩ : 10 kΩ divider adds 0.30 V on top of V_OL, and the ESP32-S3 V_IL is 0.25 × V_DD = 0.825 V |

Going straight to a GPIO is what makes that 0.55 V budget possible: the
ESP32-S3's 0.825 V V_IL is a higher ceiling than most I²C expanders offer, and
the whole 0.55 V comes out of it after the divider takes its 0.30 V.

The 3.3 V supply rules out most of the common parts: the ubiquitous A3144 needs
4.5 V, Allegro's A110x family needs 3.8 V, and Honeywell's SS41x needs 3.8 V.

**Reference part: TI DRV5033** (omnipolar) or **DRV5023** (unipolar). Verified
against the datasheet: 2.5–38 V supply, open-drain with a 30 mA sink,
r_DS(on) 22 Ω typical and 50 Ω worst case, 13 µs output delay, I_CC 2.7 mA
typical, SOT-23 or through-hole TO-92. At the 0.3 mA the pull-up sources, that
r_DS(on) puts V_OL at roughly 15 mV worst case, so the divider budget above is
met with two orders of magnitude to spare. The omnipolar DRV5033 is the more
forgiving choice because either magnet pole triggers it.

**Do not use the DRV5032** despite it looking similar. Its three open-drain
versions (FC, AJ, ZE) all duty-cycle their sampling at 20 Hz, so the output can
lag the magnet by up to 75 ms — several millimetres of overtravel at any useful
feed rate. Its 80 Hz version is push-pull only.

Current budget: six DRV5033 draw about 22 mA worst case, well inside F3's 0.1 A
hold, and a rounding error against the 3V3 rail. Move F3 up a size if you fit
thirstier sensors. Decouple each sensor with 100 nF *at the sensor*, as its
datasheet asks; the board cannot do that for you down a metre of cable.

One asymmetry is worth knowing: a hall sensor that loses power reads as
released, whereas a normally-closed contact reads as triggered when its cable is
cut. If you want a cut cable to trip the machine, mount the magnet so it is
present in the safe zone and configure that endstop inverted in firmware.

Nothing on the board distinguishes a limit from a homing datum; that is a
firmware role on the same wiring. Firmware can declare a connector a *reference*,
which is read and debounced as usual but never latches a fault, so the machine
may drive into it deliberately — a homing switch, or an index mark that a rotary
axis crosses every revolution. Declared as a limit instead, that same sensor
would stop the board on every turn. A rotary axis has no hard stop to push
against, so an index sensor on one of J11–J16 is the only way to home it: the
sensorless path needs something solid to detect.

## E-stop architecture

This is a hobby-grade motor stop, not an emergency-stop or certified safety
function.

J3 is a 4-pin screw terminal for a 2-pole normally-closed button:

| Pole | Wiring | Effect |
| --- | --- | --- |
| A | pins A1–A2, pulls `ESTOP_SENSE` (GPIO15) low when closed | Firmware sees the button and latches `Fault::EStop` |
| B | pins B1–B2, in series with `ENN_RTN` | Breaks Q1's source return, so the shared TMC2209 ENN cannot be pulled low |

ENN is pulled up to 3V3 by R15, and a TMC2209 with ENN high floats its outputs.
So the drivers are disabled by default and can only be enabled when firmware
asserts `DRV_EN` (GPIO21, into Q1's gate) **and** pole B is closed. Opening the
button releases motor torque in copper, with no firmware involvement and no
dependency on the ESP32 still running. R53 is a mandatory 10 kΩ gate-to-source
pull-down on Q1, keeping it off while GPIO21 is high-impedance during reset,
boot or with an unprogrammed MCU.

What this deliberately does *not* do:

- **It does not cut the motor rail.** VMOTOR is permanently live to the
  TMC2209 VS pins and to the 24 V output; nothing switches it.
  If a solenoid, brake or electromagnet must drop mechanically
  when the button is pressed, fit a 3-pole button and break its supply with the
  third pole, or use an external contactor. Firmware turning Q3 off is not the
  same guarantee.
- **It does not brake.** Releasing ENN lets the motors coast. A loaded vertical
  axis will back-drive.
- **There is no independent watchdog**, redundant contactor, force-guided
  relay, certified diagnostic coverage or guaranteed panic-time cut. The ESP
  watchdog and the firmware fault supervisor are best effort.

If no button is fitted, both poles need wire links across J3. Without the pole-B
link the drivers never enable at all, which is the intended failure direction.

## Power budget

| Rail | Source | Capability | Committed load |
| --- | --- | --- | --- |
| VMOTOR | Input, 6.3 A replaceable fuse | Input-limited (barrel jack 3.5 A, screw terminal 6 A) | 3 × TMC2209 VS, 24 V output on J19, aux header J21 |
| 5V_BUCK | LMR33630, 400 kHz | 3 A | Feeds the mux on VIN1 (priority input) |
| 5V_SYS | TPS2116 mux of 5V_BUCK (priority) and 5V_USB | 2.5 A on bulk power; 500 mA total on USB only | 3V3 LDO, 5 V output on J18, WS2812 strip |
| 3V3 | AMS1117 | ~0.5 A usable (0.85 W at the tab) | ESP32-S3 (500 mA peak), 3 × VCC_IO, OLED, endstops (≈22 mA if all six run hall sensors, behind F3) |

The buck taps VMOTOR directly, downstream of F1 and D1, with no fuse or TVS of
its own. On 12–24 V the 2.5 A mux, not the 3 A buck, is the ceiling on 5V_SYS;
size any big 5 V load against 2.5 A and give a long LED strip its own supply.
Because the mux is a FET pair and not a diode-OR, 5V_SYS is a full 5.0 V on
either source, and the buck holds the rail whenever it is up rather than
whichever input happens to be higher.

When powered only from USB, keep to a conservative 500 mA total board budget:
do not run motors, the 5 V output, RGB loads or external peripherals. F2 is a
thermal PTC, not a precision 500 mA current limiter. USB-only also means no
VMOTOR, so the TMC2209s have no VS and will not answer over UART — the startup
survey reports that as three silent drivers, which is expected rather than a
fault.

An RGB strip longer than roughly 30 pixels at one third brightness must be
powered from its own supply, with only DATA and GND coming back to the board.

## Firmware-facing summary

| Signal | GPIO |
| --- | --- |
| STEP 0–2 | 4, 5, 6 |
| DIR 0–2 | 7, 8, 9 |
| DIAG 0–2 | 10, 11, 12 (100 k pull-downs R62–R64) |
| Endstops E0–E5 (J11–J16) | 38, 39, 40, 41, 13, 14 |
| `ESTOP_SENSE` | 15 |
| `DRV_EN` | 21 |
| TMC UART TX / RX | 16, 47 |
| I²C SDA / SCL | 17, 18 (400 kHz, OLED at 0x3C/0x3D) |
| OUT1 5 V (J18) / OUT2 24 V (J19) | 42, 2 (both PWM-capable) |
| VMON | 1 (ADC1_CH0, 100 k / 10 k off VMOTOR) |
| J17 analogue in | 3 (ADC1_CH2) |
| WS2812 DATA | 48 (via RMT) |

Reserved: GPIO0 is the boot strap behind SW2, GPIO19/20 are USB, GPIO43/44 are
the debug UART on J5. **GPIO45 and GPIO46 are the only spare pins**, and both
are strapping pins, so anything hung on them must not drive them at reset.

TMC2209 UART addresses are 0–2 via the MS1/MS2 straps.

DIAG is a push-pull output, so the 100 k pull-downs (R62–R64, sheet 5) only
matter when a driver site is left empty: they keep that GPIO low instead of
floating into the fault supervisor. They stay fitted on a 2-axis build.
Firmware also enables the internal pull-down on each DIAG input, and the
internal pull-up on each endstop and on `ESTOP_SENSE`; the board's own 10 k
pull-ups set those levels, so the internal ones only add a weak parallel path.

Firmware drives the whole motion path: STEP/DIR, TMC UART, endstops, DIAG,
e-stop sense and driver enable. It homes either sensorlessly on StallGuard via
DIAG, or against a reference endstop on J11–J16, and it can run the step ISR and
the motion task on the second core so an application can have the first to
itself. At startup it surveys what is fitted: it reads VMON to report VMOTOR,
probes the OLED addresses, checks the e-stop, and asks each of the three UART
addresses for its `IOIN.version`.

Because VMOTOR is permanently live, that driver probe happens in a single pass
during construction rather than having to wait on a rail coming up. The I²C bus
carries nothing but the OLED header, so firmware hands the bus straight to the
application.

OUT1, OUT2 and the WS2812 are wired and waiting; they are the obvious next
contributor tasks, not missing hardware.

## Home-fab kit

Built so one stencil and a hot plate can do the whole board. Nothing smaller
than 0603. Cable-strain connectors — Phoenix screw terminals, the USB-C
hold-downs and every 2.54 mm header — stay through-board on purpose: a
yanked motor or endstop cable will lift an SMD pad. The ten signal headers
snap off one 40-pin breakaway strip, which is why they are a single BOM line;
the three motor headers are a line of their own. The motor headers are
unkeyed, so mark pin 1 on each housing and never plug or unplug a motor with
the board powered.

Resistor vocabulary, all 0603 except the 2512 sense parts:

| Value | Used for |
| --- | --- |
| 470 Ω | 3V3 rail LED (R61) and WS2812 data series (R58) |
| 1 kΩ | Endstop RC series, UART TX, FET gate series, 5 V rail LED, PGOOD LED |
| 4.7 kΩ | I²C pull-ups |
| 5.1 kΩ | USB-C CC (spec, do not substitute) |
| 10 kΩ | Most pull-ups/downs, VREF lower, ENN pull-up (R15), MS1/MS2 straps, VMOTOR monitor lower (R3), VMOTOR rail LED |
| 24.9 kΩ | LMR33630 FB lower (datasheet 5.0 V) |
| 33 kΩ | VREF top, mux PR1 lower |
| 100 kΩ | Buck FB top, UVLO lower, mux PR1 top, VMOTOR monitor top (R2), DIAG pull-downs (R62–R64) |
| 750 kΩ | LMR33630 UVLO top (~10.5 V start) |
| 0.11 Ω 2512 | TMC sense, keep this one |

Capacitor vocabulary:

| Value | Package | Used for |
| --- | --- | --- |
| 22 nF 50 V | 0603 | TMC charge pump only |
| 100 nF 50 V | 0603 | Decoupling, endstop RC, C_BOOT |
| 220 nF 50 V | 0603 | LMR33630 high-frequency input bypass |
| 1 µF 16 V | 0603 | LMR33630 VCC bypass |
| 4.7 µF 16 V | 0603 | TMC2209 5VOUT bypass |
| 10 µF 16 V | 0805 | 5 V / 3V3 / EN / USB |
| 10 µF 50 V | 1210 | VMOTOR / VS ceramics |
| 22 µF 16 V | 1206 | Buck output, 5V_SYS bulk, RGB reservoir |
| 22 µF tant | 3528 (B case) | AMS1117 output only |
| 220 µF 50 V | SMD can | Input bulk and each TMC VS |

Buck FB is 100 k / 24.9 k → 5.02 V (LMR33630 table). UVLO is 750 k / 100 k →
~10.5 V start. The VMOTOR monitor is 100 k / 10 k → 2.18 V at 24 V, inside
ADC1's range on the 11 dB attenuator. Mux handover is 100 k / 33 k → 4.03 V
against the TPS2116's 1.0 V reference, so the board only falls back to USB once
the buck has genuinely collapsed rather than merely sagged. It must not go much
lower: the AMS1117 drops about 1 V, so a handover near 3.3 V (a 43 k lower leg)
would let 3V3 fall below the ESP32-S3 brownout level before USB takes over.

## PCB layout in KiCad

A hobby 4-layer board. Copy the datasheet layouts for the buck and the three
TMC2209s, keep the ground plane intact, and do not get clever with the antenna.
The schematic generator is the netlist source of truth; these notes are the
placement and routing constraints. The `kicad/` project follows the generator;
`check_kicad.py` confirms it net by net.

### Before you place anything

- Use the manufacturer footprint for every IC, not a generic QFN / SOT-583.
- AMS1117 SOT-223 tab is **VOUT**, not ground. The copper pour under it is a
  3V3 pour.
- L1 is a Bourns SRP1265A (13.5 × 12.5 mm). KiCad has no footprint for it;
  the project's `Omnimotion.pretty/L_Bourns_SRP1265A` follows the Bourns
  recommended layout. Size the keepout to it.

### Stackup

| Layer | Use |
| --- | --- |
| L1 | Parts and signals |
| L2 | Solid GND. Do not cut it for traces. |
| L3 | Power: VMOTOR, 5V_SYS, 3V3 |
| L4 | Signals + GND fill |

1.6 mm FR-4, 1 oz outer / 1 oz inner, ENIG (QFN pads and the module).
Plated M3 holes MH1–MH4 are tied to GND. Use nylon hardware at MH4 to
keep metal away from the ESP32 antenna. MH1–MH3 provide the chassis
connections for the motor-cable EMI return.

### Zoning — place in this order

1. **U4 + J4.** Module antenna hangs over the board edge. Keepout ≥ 15 mm
   with no copper, no traces, no ground on any layer, and no metal
   (motors, heatsinks, connectors, chassis hardware, cable harnesses) in that volume.
   Dress the J18/J19 output cables toward the top edge and guide J17's
   cable away from the lower-right corner.
2. **U5–U7 + J8–J10.** Drivers along the motor-connector edge, ≥ 15 mm
   between exposed pads. L1 and the three TMC2209s sit on the far side of
   the board from the antenna.
3. **J1 / J2 / F1 / D1 / C1–C4.** Short, fat 6 A path. Barrel jack and screw
   terminal in parallel.
4. **U1 + L1.** Away from the antenna and away from the VMON divider.
5. **The VMON divider (R2/R3) and J17's RC.** Keep them off the buck SW node
   and off motor traces; they feed a 12-bit ADC.
6. Everything else fills in. J3 on an edge, not buried.

### Must-follow (the board fails if you skip these)

**TMC2209 (each of U5–U7)**

- Solder the exposed pad. ≥ 16 thermal vias, 0.3 mm drill / 0.6 mm pad,
  into L2 and L4. ≥ 400 mm² of L2 copper per driver.
- Within 3 mm of the package: 22 nF CPO–CPI, 100 nF VCP–VS, 4.7 µF
  5VOUT, 100 nF VCC_IO, both 0.11 Ω sense resistors.
- 220 µF SMD electrolytic within 10 mm of the VS pins. This is the
  capacitor that survives motor recirculation. Same can as the input bulk.
- Motor traces ≥ 1.5 mm on 1 oz, routed as tight A+/A− and B+/B− pairs,
  kept off L2 so return current does not cut the plane.
- Sense-resistor grounds go back to **that** driver's EP, not through
  the motor-connector return.

**LMR33630 (copy the TI layout)**

- C_IN on the same side as the IC. VIN–PGND loop < 4 mm².
- SW node kept small. No copper under the inductor keepout.
- C9 (220 nF) is the VIN high-frequency bypass, right at the VIN pins.
  C11 (100 nF) is C_BOOT, from BOOT to SW. Same 100 nF as everywhere else.
- Exposed pad to L2 with ≥ 9 thermal vias. EP is AGND and the heatsink.

**ESP32-S3-WROOM-1 + USB**

- C26/C27 within 3 mm of module pin 2.
- USB D+/D− as a 90 Ω pair, < 40 mm, no vias, referenced to L2.
  USBLC6-2 within 5 mm of J4; D+/D− go straight through, no stubs.
- Solder the USB4105 shell tabs. The shield is the mechanical
  anchor. The receptacle is top-mount: its front must reach the board edge,
  or the plug overmold hits the PCB before it seats. JP1 ties shield to GND.
- GPIO33/34 are not bonded out. GPIO35/36/37 are octal PSRAM. None of
  them get a trace.

**Discretes on the power path**

- D1 is both the surge clamp and the reverse-polarity crowbar. Give it a short,
  wide path to the input pins and to GND — it has to be able to blow F1.
- U2 carries the whole 5 V load through an 8-pin SOT-583 the size of a grain of rice.
  Its thermal path is the copper on VIN1/VIN2/VOUT, so pour those three nets
  rather than routing them as traces.
- Q1 is the ENN chain FET. Its source goes to `ENN_RTN`, which leaves the board
  at J3 pins B1–B2. That trace is a signal, but breaking it is what stops the
  motors, so route it as a deliberate pair to the connector rather than
  incidentally.
- Q2/Q3 are SOT-23 at up to 1 A. Give each drain and source pad as much copper
  as the footprint allows and keep the gate pull-downs at the FET.
- Keep D2/D3 (the output flybacks) at their connectors. With the TBD62064 gone
  they are the only clamp on either channel.
- AMS1117: ≥ 300 mm² of 3V3 copper on the tab. C22 (tantalum) close to
  VOUT. A pure MLCC there can make the LDO sing.

**Connectors and analogue**

- Put R46–R51 and R52 hard against their connectors rather than near the MCU.
  With no clamp diodes fitted, the series resistor is the only thing between
  an external wire and a GPIO, so the unprotected trace ahead of it should be
  as short as the layout allows.
- Endstop pin 1 is the F3 branch, not the raw rail. Silk must say
  `3V3 / GND / SIG` so nobody wires a PNP or 24 V sensor into it.
- Keep F3 and C63 next to the endstop connector block, not back at the LDO.
- J8–J10 silk: `A+ A− B+ B−`. A cross-coil swap makes the motor buzz.
- J3 silk: `E-STOP  A: sense  B: ENN — link both if unused`. Getting the two
  pairs swapped is the one wiring mistake that leaves the board looking dead.
- R58 (WS2812 series) stays at the MCU end, not at J20. D6 sits in the strip's
  5 V feed near J20.

### Trace widths (1 oz, good enough)

| Net | Width |
| --- | --- |
| Motor phases | ≥ 1.5 mm |
| VMOTOR | ≥ 2 mm or a pour |
| 5V_SYS | ≥ 1 mm |
| 3V3 | ≥ 0.6 mm |
| USB D+/D− | 90 Ω differential |
| STEP / DIR / UART / I²C | 0.25 mm is fine |

Stitch L4 GND fill to L2 with vias around the board edge and around
each driver. Power vias: several 0.3 mm rather than one heroic via.

### After routing, before Gerbers

- DRC against the fab's real constraints, not KiCad defaults.
- Walk L2: it should still be a plane under the TMC2209s and the buck,
  not Swiss cheese.
- Confirm L1, U4 and USB4105 against the chosen supplier's land pattern.
- Probe pads TP1–TP15 stay reachable with a 2 mm keepout.
- Bring-up order is on schematic sheet 9: power rails with no ICs, then buck,
  then LDO, then ESP32, then one driver.

## Known risks and open items

- **No overvoltage protection on the motor rail.** This is the headline risk of
  the whole design. TMC2209 V_VS abs max is 32 V; D1 (SMCJ26A) does not conduct
  meaningfully until 28.9 V and clamps at 42.1 V at full surge current, so a
  fast input transient or motor-regeneration event can exceed both the drivers'
  32 V limit and the buck's 38 V limit before the TVS contains it. Nothing on
  the board opens the rail to prevent that.
  Mitigate by supply choice: a bench supply or a quality 24 V brick on a short
  lead is fine, a long inductive feed or a shared supply with contactors is
  not. If your application needs that, put surge suppression upstream.
- **The buck shares the input protection.** F1 and D1 are all that stand
  between a 24 V transient and the LMR33630's 38 V absolute maximum; the buck
  branch has no fuse or TVS of its own.
- **F1 is fast-acting, and inrush is the reason to watch it.** The 154-series
  OMNI-BLOK has no 6 A part and offers no time-lag option above 5 A, so F1 is a
  6.3 A fast-acting cartridge with a 9.17 A²s melting I²t. Charging the 1.1 mF
  of VMOTOR bulk from 24 V costs roughly 3 A²s at 100 mΩ of source-plus-ESR,
  and closer to 7 A²s on a stiff supply with short leads. That is adequate but
  not generous, and fuses age with repeated pulses near their melting I²t.
  Cold-start the finished board a dozen times before trusting it. The escape
  hatch is cheap: the holder accepts any Nano2, so a 6.3 A Slo-Blo
  (045406.3MR, 75 A²s) is a fuse swap rather than a board change and gives up
  no current rating.
- **USB inrush is not compliance-qualified.** C17 and C30 put about 20 µF
  nominal directly on 5V_USB, before the mux output capacitance is considered.
  F2 is a thermal fuse, not controlled inrush limiting. Treat USB as a
  development-power fallback unless this is measured against the intended
  host or an inrush limiter is added.
- **The e-stop does not de-energise the 24 V output.** Covered in
  ["E-stop architecture"](#e-stop-architecture); worth repeating because it is
  the most likely thing for a builder to assume wrongly.
- **Thermal, TMC2209.** The main heat path runs from each exposed pad through
  at least 16 thermal vias into the solid inner ground plane. Above about
  1.2 A RMS per axis, add the optional top-mounted heatsinks HS1–HS3 listed
  at the end of the BOM as supplemental cooling.
- **Motor current and heat.** The fitted shunts are Vishay WSL2512R1100FEA,
  0.11 Ω, 1 %, 2512, 1 W. The continuous board rating is 1.4 A RMS per axis,
  contingent on validating driver temperature with all required thermal vias,
  copper, enclosure, airflow and ambient conditions.
- **The rail LEDs are approximate indicators.** D4 ("5 V rail bad") runs off
  5V_BUCK itself, so it lights only while the buck is up but out of regulation
  (start-up, overload, PGOOD fault); a buck with no input or a dead output
  leaves it dark. Feeding it from 5V_SYS would make it useful with USB attached
  but leaves its state undefined when VMOTOR is absent, so that is a design
  decision still to make. D9 (3V3) has only about 0.4 V across R61 with a
  2.9 V green LED, so its ~0.85 mA and its brightness follow the LED's V_F
  spread; a red LED or a smaller resistor would fix it at the cost of a new
  colour or value on the BOM.
- **AMS1117 headroom.** 0.85 W worst case in a SOT-223. If a future project
  adds 3.3 V peripherals, replace it with a small buck rather than pushing the
  LDO.
- **Two spare GPIOs, both strapping pins.** GPIO45 and GPIO46 are all that is
  left, so there is no expansion headroom to speak of; a project needing more
  I/O wants an I²C expander on J7 rather than a board respin.
- **Output flybacks are the only clamp.** The AO3400A is a 30 V part driving a
  24 V rail, so D2/D3 are load-bearing, not optional. Do not fit an inductive
  load without one.
- **No clamp diodes on the external inputs.** The 1 kΩ and 100 nF on each
  endstop and on J17 handle ESD well, but nothing clamps a sustained wrong
  voltage. 5 V into SIG is harmless, because the 1 kΩ holds injection into the
  ESP32's pin diode to about 1 mA. 24 V pushes roughly 20 mA into that same
  diode, which is at the absolute-maximum per-pin injection and will take the
  GPIO with it — and the module is soldered down, so that means replacing the
  whole WROOM. Silk the connectors clearly and never wire a PNP sensor or an
  externally powered output into SIG.
- **No current sensing on the outputs.** J21 can carry a shunt amplifier into
  GPIO3 if it becomes necessary.
- **Hall-sensor matrix stays off this board.** It talks to J21 and the I²C
  header. That is a separate thing from a hall sensor used as a single endstop,
  which J11–J16 support directly.
- **Nothing here has been fabricated.** Before ordering: confirm the buck
  inductor footprint against the part you bought, and check the Gerbers in
  the fab's own DFM viewer.
