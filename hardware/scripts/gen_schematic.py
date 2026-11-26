#!/usr/bin/env python3
"""Generate the full schematic for the Universal ESP32-S3 Motion Controller.

Run:  python3 scripts/gen_schematic.py [--check]   ->  generated/omnimotion_schematic.svg

--check regenerates in memory and fails if the committed file differs.
"""

import os
import sys

from schlib import Doc

PROJECT = "UNIVERSAL ESP32-S3 MOTION CONTROLLER  \u00b7  OMNIMOTION"
REV = "A.1"

doc = Doc(PROJECT, REV)


# ---------------------------------------------------------------------------
# Sheet 1 - architecture, power tree, constraints
# ---------------------------------------------------------------------------
def sheet_overview():
    s = doc.sheet("1 \u00b7 Architecture, Power Tree & Constraints",
                  "3-axis TMC2209 motion motherboard \u00b7 12-24 V in \u00b7 "
                  "4-layer PCB")

    s.zone(40, 78, 1258, 452, "SYSTEM ARCHITECTURE")
    s.blk(520, 108, 300, 74, "ESP32-S3-WROOM-1-N16R8",
          ["16 MB flash \u00b7 8 MB octal PSRAM",
           "Wi-Fi / BLE / native USB", "Sheet 4"], "blkg")

    cols = [
        (60, 260,
         ("STEP / DIR / DIAG \u00b7 UART",
          ["3+3+3 GPIO \u00b7 TX 16 / RX 47", "Sheets 5-7"], "blk"),
         ("U5-U7  3\u00d7 TMC2209-LA-T",
          ["VS = VMOTOR \u00b7 UART addr 0-2",
           "0.11 \u03a9 sense \u00b7 QFN28-EP"], "blkp"),
         ("3\u00d7 motor connectors  J8-J10", "blky")),
        (340, 200,
         ("I\u00b2C  400 kHz", ["SDA 17 / SCL 18", "Sheet 8"], "blk"),
         ("OLED J6  \u00b7  I\u00b2C exp J7", ["0x3C typ."], "blky"),
         None),
        (560, 250,
         ("Direct GPIO in",
          ["6 endstops 38-41 / 13-14",
           "ESTOP 15 \u00b7 VMON 1 \u00b7 AIN 3"], "blk"),
         ("1 k + 10 k + clamp per input", ["Sheet 8"], "blk"),
         ("J11-J16  \u00b7  J17  \u00b7  J3", "blky")),
        (830, 230,
         ("Direct GPIO out",
          ["OUT1 42 \u00b7 OUT2 2 \u00b7 RGB 48",
           "DRV_EN 21 \u00b7 Sheet 9"], "blk"),
         ("Q2 / Q3 AO3400A \u00b7 WS2812",
          ["2\u00d7 low-side switch"], "blkp"),
         ("J18  \u00b7  J19  \u00b7  J20", "blky")),
        (1080, 200,
         ("USB 2.0", ["D- 19 / D+ 20"], "blk"),
         ("D5 USBLC6-2SC6", ["ESD"], "blk"),
         ("USB-C  J4", "blky")),
    ]
    for x, w, r1, r2, r3 in cols:
        cx = x + w / 2
        s.blk(x, 236, w, 60, r1[0], r1[1], r1[2])
        s.arrow((670, 182), (670, 214), (cx, 214), (cx, 236))
        s.blk(x, 336, w, 58, r2[0], r2[1], r2[2])
        s.arrow((cx, 296), (cx, 336))
        if r3:
            s.blk(x, 434, w, 40, r3[0], (), r3[1], "ts")
            s.arrow((cx, 394), (cx, 434))

    # ------------------------------------------------------- key decisions
    s.note(1316, 78, 484, "KEY DESIGN DECISIONS",
           ["1. TMC2209 VS sits directly on VMOTOR. There is no ideal-diode",
            "   controller and no switched motor rail: motor current is cut "
            "at",
            "   the driver through ENN, which is what every RAMPS-class board",
            "   does. D1 alone handles surge and reverse polarity - it opens",
            "   F1 - so feed this board 12-24 V and nothing else.",
            "2. J3 is a 2-pole NC e-stop. Pole A drives ESTOP_SENSE (GPIO15) "
            "so",
            "   firmware can stop the planner; pole B sits in series with "
            "Q1's",
            "   source, so ENN can only be pulled low when DRV_EN is high AND",
            "   the button is closed. That part is copper, not firmware.",
            "3. Six endstops land on GPIO38-41 + GPIO13/14, straight onto the",
            "   MCU with no I\u00b2C expander anywhere in the path.",
            "4. One analogue input: J17 \u2192 GPIO3 (ADC1_CH2). VMOTOR is",
            "   monitored on GPIO1 (ADC1_CH0) by the on-chip 12-bit ADC.",
            "5. Three axes (U5-U7, J8-J10) and two outputs: discrete AO3400A",
            "   low-side switches, OUT1 5 V on J18, OUT2 24 V on J19.",
            "6. A series SS36 drops the WS2812 strip VDD by one diode drop.",
            "   That helps 3.3 V data but is no guaranteed level shift: fit a",
            "   3.3 V-compatible strip or an external AHCT buffer.",
            "7. GPIO35/36/37 are the N16R8 octal PSRAM and GPIO33/34 are not",
            "   bonded on WROOM-1; none are used. GPIO45/46 are the only "
            "spares.",
            "8. VREF must be defined at reset because I_scale_analog resets",
            "   to 1: a 33 k / 10 k divider off 5VOUT gives a safe "
            "~0.82 A_RMS",
            "   default until firmware clears the bit."],
           "boxw")

    # ------------------------------------------------------------ power tree
    s.zone(40, 546, 1258, 356, "POWER TREE")
    row = 588
    a = []
    for x, w, t, ln, c in [
            (58, 132, "DC INPUT", ["J1 barrel / J2 screw", "12-24 V DC"],
             "blky"),
            (216, 104, "F1", ["6.3 A / 125 V", "fuse"], "blkp"),
            (346, 116, "D1  TVS", ["SMCJ26A", "V_BR 28.9 V"], "blkp"),
            (488, 136, "VMOTOR", ["12-24 V", "220 \u00b5F \u00d7 2 SMD"],
             "blkp"),
            (650, 250, "U5-U7 TMC2209 VS  \u00b7  OUT2 24 V via Q3",
             ["220 \u00b5F per driver SMD",
              "always live \u2014 ENN cuts torque"], "blkp")]:
        a.append(s.blk(x, row, w, 62, t, ln, c))
    for i in range(len(a) - 1):
        s.arrow((a[i]["x"] + a[i]["w"], row + 31),
                (a[i + 1]["x"], row + 31), cls="arwp")

    row2 = 700
    b1 = s.blk(488, row2, 140, 58, "U1  LMR33630",
               ["5 V / 3 A buck", "taps VMOTOR direct"], "blkg")
    b2 = s.blk(658, row2, 110, 58, "5V_BUCK", ["5 V / 3 A"], "blkg")
    b3 = s.blk(798, row2, 150, 58, "U2  TPS2116",
               ["priority mux, 2.5 A", "buck wins, no drop"], "blkg")
    b4 = s.blk(978, row2, 110, 58, "5V_SYS", ["5.0 V \u00b7 2.5 A"], "blkg")
    s.arrow((556, row + 62), (556, row2), cls="arwp")
    for p, q in ((b1, b2), (b2, b3), (b3, b4)):
        s.arrow((p["x"] + p["w"], row2 + 29), (q["x"], row2 + 29), cls="arwp")

    c1 = s.blk(58, row2, 140, 58, "USB-C VBUS",
               ["5 V from host", "J4 + D5 USBLC6"], "blkg")
    c2 = s.blk(222, row2, 104, 58, "F2  PTC", ["500 mA hold"], "blkg")
    c3 = s.blk(350, row2, 110, 58, "5V_USB", ["dev / flash"], "blkg")
    for p, q in ((c1, c2), (c2, c3)):
        s.arrow((p["x"] + p["w"], row2 + 29), (q["x"], row2 + 29), cls="arwp")
    s.arrow((460, row2 + 29), (478, row2 + 29), (478, row2 + 76),
            (873, row2 + 76), (873, row2 + 58), cls="arwp")
    s.text(560, row2 + 92, "USB fallback \u2192 mux VIN2, only below 4.03 V "
                           "on PR1", "ts")

    row3 = 812
    d1 = s.blk(978, row3, 140, 56, "U3  AMS1117-3.3",
               ["LDO \u00b7 SOT-223", "0.85 W worst case"], "blkg")
    d2 = s.blk(1148, row3, 110, 56, "3V3", ["3.3 V / ~0.5 A"], "blkg")
    s.arrow((1033, row2 + 58), (1033, row3), cls="arwp")
    s.arrow((d1["x"] + d1["w"], row3 + 28), (d2["x"], row3 + 28), cls="arwp")
    s.blk(58, row3, 420, 56, "5V_SYS loads",
          ["Q2 / OUT1 \u00b7 WS2812 strip through D6 \u00b7 U3 LDO input"],
          "blky", "ts")
    s.blk(500, row3, 440, 56, "3V3 loads",
          ["ESP32-S3 \u00b7 3\u00d7 VCC_IO \u00b7 endstops + J17 \u00b7 "
           "OLED \u00b7 J21"], "blky", "ts")

    # -------------------------------------------------- absolute-maximum table
    s.table(1316, 566, ["Node / device", "Datasheet limit", "Design value"],
            [["TMC2209 V_VS", "32 V absolute maximum", "24 V, no active OVP"],
             ["TMC2209 rec. VS", "4.75 - 29 V", "12 - 24 V"],
             ["TMC2209 V_VIO", "5.5 V abs", "3.3 V"],
             ["TMC2209 5VOUT load", "25 mA int+ext", "~0.15 mA divider"],
             ["LMR33630 V_IN", "38 V abs, 36 V rec.", "24 V"],
             ["LMR33630 V_BOOT-SW", "5.5 V abs", "100 nF from BOOT"],
             ["AMS1117 V_IN", "15 V abs", "4.7 - 5 V"],
             ["AO3400A V_DS", "30 V", "24 V + SS36 clamp"],
             ["AO3400A V_GS(th)", "1.45 V max", "3.3 V gate drive"],
             ["TPS2116 VIN/VOUT", "6 V abs, 2.5 A", "5 V / 2.5 A"],
             ["SS36", "60 V / 3 A", "flyback + RGB feed"],
             ["USBLC6-2SC6", "5.25 V working", "VBUS + D+/D-"],
             ["ESP32-S3 GPIO", "3.3 V", "3.3 V only, no 5 V in"],
             ["ESP32-S3 ADC1", "~0 - 2.9 V calibrated", "VMON 2.18 V at 24 V"]],
            [172, 164, 140], "ABSOLUTE-MAXIMUM BUDGET (verified against "
                             "datasheets)", 26)

    # ------------------------------------------------------------ sheet index
    s.table(1316, 980, ["#", "Sheet", "Contents"],
            [["1", "Architecture", "block diagram, power tree, constraints"],
             ["2", "Input protection", "fuse, TVS, 2-pole e-stop, VMOTOR "
                                       "monitor"],
             ["3", "5 V / 3.3 V", "LMR33630 buck, TPS2116 mux, AMS1117"],
             ["4", "MCU + USB", "ESP32-S3-WROOM-1, USB-C, boot/reset"],
             ["5", "Axis M0", "TMC2209 channel, UART bus, ENN chain, DIAG "
                            "pull-downs"],
             ["6", "Axis M1", "TMC2209 channel + per-channel BOM"],
             ["7", "Axis M2", "TMC2209 channel + motor interface notes"],
             ["8", "Endstops + I\u00b2C", "6 direct-GPIO endstops, J17 "
                                          "analogue in, headers"],
             ["9", "Outputs + test", "Q2/Q3 low-side, RGB, LEDs, J21, test "
                                     "points"]],
            [30, 150, 300], "SHEET INDEX")

    s.note(40, 950, 1258, "NON-NEGOTIABLE LAYOUT / DESIGN RULES",
           ["\u2022 4 layers: L1 signal + components, L2 solid GND, L3 power "
            "planes (VMOTOR / 5V / 3V3 \u2014 there is no switched rail), L4 "
            "signal + GND fill.",
            "\u2022 Every TMC2209 exposed pad soldered to GND with \u2265 16 "
            "thermal vias (0.3 mm drill / 0.6 mm pad) into L2 and L4. C_VS "
            "ceramics, the 22 nF charge-pump cap and the two 0.11 \u03a9 "
            "sense",
            "  resistors all placed within 3 mm of their pins.",
            "\u2022 LMR33630: copy the TI layout \u2014 C_IN on the same "
            "side, VIN/PGND loop < 4 mm\u00b2, SW node kept small, no cut in "
            "the L2 pour underneath.",
            "\u2022 ESP32-S3 antenna hangs over the board edge; keepout of "
            "\u2265 15 mm with no copper on any layer, no traces, no ground.",
            "\u2022 USB D+/D- routed as a 90 \u03a9 differential pair, "
            "< 40 mm, no vias, referenced to L2, with D5 USBLC6 within 5 mm "
            "of J4.",
            "\u2022 Motor traces \u2265 1.5 mm on 1 oz copper, kept off L2, "
            "routed as tight A+/A- and B+/B- pairs to the connector.",
            "\u2022 The two surviving analogue nodes are the VMOTOR divider "
            "(sheet 2) and J17's RC (sheet 8). Keep both \u2265 20 mm from "
            "the SW node and the motor",
            "  traces, and return them to L2 close to the module rather than "
            "out among the drivers.",
            "\u2022 The e-stop is NOT a certified safety function. Pole B "
            "breaks Q1's source return in copper, so ENN releases all three "
            "drivers with no firmware in the",
            "  path \u2014 but VMOTOR and the 24 V output on J19 stay live. A "
            "load that must drop mechanically needs a third pole or an "
            "external contactor.",
            "\u2022 Do not fit the 24 V flyback D3 below 60 V, and keep its "
            "loop to Q3 short: the AO3400A is only a 30 V part."])
    return s




# ---------------------------------------------------------------------------
# Sheet 2 - input protection, e-stop
# ---------------------------------------------------------------------------
def sheet_input():
    s = doc.sheet("2 \u00b7 DC Input Protection & E-Stop",
                  "12-24 V in \u2192 VMOTOR \u00b7 fuse + TVS \u00b7 "
                  "2-pole e-stop")
    Y = 170

    # ---------------------------------------------------- input connectors
    j1 = s.conn(56, 96, "J1", "DC-005  5.5/2.1", ["+VIN", "GND"], 120, "right",
                32)
    j2 = s.conn(56, 220, "J2", "2P 2.54 mm screw", ["+VIN", "GND"], 120,
                "right", 32)
    s.hv(j1["+VIN"], (250, Y))
    s.hv(j2["+VIN"], (250, Y))
    s.dot(250, Y)
    for j in (j1, j2):
        s.line(*j["GND"], 216, j["GND"][1])
        s.gnd(216, j["GND"][1])
    s.text(56, 372, "J1 and J2 are wired in parallel \u2014 fit whichever "
                    "suits the machine.", "ts")
    s.text(56, 385, "J1 is a 3.5 A part, so use J2 above that, and when all "
                    "three motors", "ts")
    s.text(56, 398, "run near their limit.", "ts")

    # ------------------------------------------------------- fuse + TVS
    s.fuse(270, Y, "F1", "6.3 A / 125 V")
    s.line(250, Y, 270, Y)
    s.line(334, Y, 1210, Y)
    s.diode_v(380, 200, "D1", "SMCJ26A", 60, "z", flip=True)
    s.line(380, Y, 380, 200)
    s.dot(380, Y)
    s.gnd(380, 260)

    # --------------------------------------------------- VMOTOR rail + bulk
    s.text(560, 150, "VMOTOR  \u2014 fused, TVS-clamped, always on", "pwr")
    for x, ref, val, pol in ((470, "C1", "220 \u00b5F 50 V", True),
                             (560, "C2", "220 \u00b5F 50 V", True),
                             (650, "C3", "10 \u00b5F 50 V", False),
                             (740, "C4", "100 nF 50 V", False)):
        c = s.cap_v(x, 196, ref, val, 56, pol, side="b")
        s.line(x, Y, x, 196)
        s.dot(x, Y)
        s.gnd(x, c["2"][1])
    s.tp(830, Y, "TP1", "t")
    s.dot(830, Y)
    s.arrow((900, Y), (900, 128))
    s.text(910, 118, "to U1 LMR33630 buck (sheet 3)", "ts")
    s.arrow((1060, Y), (1060, 128))
    s.text(1070, 118, "to 3\u00d7 TMC2209 VS (sheets 5-7)", "ts")
    s.text(1070, 131, "and the 24 V output (sheet 9)", "ts")
    s.netlabel(1210, Y, "VMOTOR", "r", "flagp")

    # -------------------------------------------------------- VMOTOR monitor
    s.zone(56, 540, 620, 260, "VMOTOR MONITOR \u2192 GPIO1 / ADC1_CH0")
    s.netlabel(200, 610, "VMOTOR", "l", "flagp")
    s.line(200, 610, 230, 610)
    s.res_h(230, 610, "R2", "100 k", 100)
    s.line(330, 610, 390, 610)
    s.dot(390, 610)
    r10 = s.res_v(390, 650, "R3", "10 k", side="l")
    s.line(390, 610, 390, 650)
    s.gnd(390, r10["2"][1])
    c10 = s.cap_v(480, 650, "C6", "100 nF")
    s.wire((390, 610), (480, 610), (480, 650))
    s.dot(480, 610)
    s.gnd(480, c10["2"][1])
    s.line(480, 610, 570, 610)
    s.netlabel(570, 610, "VMON", "r", "flagp")
    s.text(90, 750, "24 V \u2192 2.18 V and 30 V \u2192 2.73 V, both inside "
                    "the ESP32-S3 ADC1 range at", "ts")
    s.text(90, 766, "11 dB attenuation. Firmware only needs \"is bulk "
                    "present, and roughly what\", so", "ts")
    s.text(90, 782, "the on-chip 12-bit ADC is all this needs.", "ts")

    # --------------------------------------------------------------- e-stop
    s.zone(56, 880, 900, 330, "E-STOP \u2014 2-pole NC button: one pole to "
                              "firmware, one in the ENN return")
    j3 = s.conn(90, 950, "J3", "4P 2.54 mm screw", ["A1", "A2", "B1", "B2"],
                140, "right", 34)

    # pole A: tells firmware
    s.line(*j3["A1"], 270, j3["A1"][1])
    s.gnd(270, j3["A1"][1])
    sense = j3["A2"][1]
    s.line(*j3["A2"], 640, sense)
    s.rail(420, sense - 64, "+3V3")
    s.res_v(420, sense - 64, "R1", "10 k")
    s.dot(420, sense)
    c5 = s.cap_v(510, sense, "C5", "100 nF")
    s.dot(510, sense)
    s.gnd(510, c5["2"][1])
    s.netlabel(640, sense, "ESTOP_SENSE", "r")
    s.text(300, 1195, "closed \u2192 low = running   \u00b7   pressed or "
                      "cable cut \u2192 high = stopped", "ts")

    # pole B: sits in the TMC2209 ENN pull-down return
    s.line(*j3["B1"], 300, j3["B1"][1])
    s.netlabel(300, j3["B1"][1], "ENN_RTN", "r")
    s.line(*j3["B2"], 270, j3["B2"][1])
    s.gnd(270, j3["B2"][1])

    s.note(980, 290, 816, "WHY TWO POLES",
           ["Pole B is wired in series with the source of Q1, the FET that "
            "pulls the shared",
            "TMC2209 ENN line low (sheet 5). Opening the button breaks that "
            "return, ENN",
            "floats up to +3V3 through R15, and all three drivers release "
            "their outputs.",
            "That happens in copper, with no firmware in the path.",
            "",
            "Pole A only tells firmware what happened, so it can stop the "
            "planner, drop the",
            "step queue and forget its position instead of resuming into a "
            "moved axis.",
            "",
            "This is NOT a certified safety function.",
            "",
            "TRADE: the motor rail now stays live when stopped, so the 24 V "
            "output does not",
            "de-energise mechanically. If a solenoid or brake must drop on "
            "e-stop, use a",
            "3-pole button and put the third pole in that load, or an "
            "external contactor.",
            "",
            "If no button is fitted, link A1-A2 and B1-B2. Without the B link "
            "the drivers",
            "never enable."], "boxw")

    s.note(980, 640, 816, "WHAT THE INPUT PROTECTION COVERS",
           ["D1 and F1 are the whole input-protection scheme. There is no "
            "ideal-diode",
            "controller, no overvoltage cut and no switched motor rail.",
            "",
            "  Reverse polarity \u2014 D1 is unidirectional, so a backwards "
            "input forward-biases",
            "  it and it blows F1. F1 is a replaceable cartridge in an "
            "OMNI-BLOK holder, so",
            "  the cost of a wrong-polarity plug is one fuse rather than one "
            "board.",
            "",
            "  Overvoltage \u2014 a 36 V or 48 V supply also drives D1 hard "
            "and opens F1, but the",
            "  drivers see the clamp voltage until it does. That is slow and "
            "not certain.",
            "  Feed this board 12-24 V.",
            "",
            "  Switched rail \u2014 there is no VM_SW. Motor current is cut "
            "at the driver via ENN,",
            "  which is what every RAMPS-class board does. It also means a "
            "TMC2209 answers",
            "  over UART at boot, so the startup driver survey is a single "
            "pass."], "boxw")
    return s


# ---------------------------------------------------------------------------
# Sheet 3 - 5 V buck, TPS2116 priority mux, 3.3 V LDO
# ---------------------------------------------------------------------------
def sheet_supplies():
    s = doc.sheet("3 \u00b7 5 V Buck, Priority Mux & 3.3 V LDO",
                  "VMOTOR \u2192 5V_BUCK \u2192 5V_SYS \u2192 3V3 \u00b7 "
                  "TI reference values")

    # ============================================================ LMR33630
    s.zone(46, 92, 1750, 540, "5 V / 3 A STEP-DOWN \u2014 TI recommended "
                              "configuration for 400 kHz, V_OUT = 5 V")
    s.netlabel(180, 210, "VMOTOR", "l", "flagp")
    s.line(180, 210, 540, 210)
    s.text(200, 168, "Tapped straight off VMOTOR, downstream of F1 and D1 on "
                     "sheet 2. There is no separate buck fuse or TVS.", "ts")
    for x, ref, val in ((300, "C7", "10 \u00b5F 50 V"),
                        (370, "C8", "10 \u00b5F 50 V"),
                        (440, "C9", "220 nF 50 V")):
        c = s.cap_v(x, 210, ref, val, 56, side="b")
        s.dot(x, 210)
        s.gnd(x, c["2"][1])
    u2 = s.ic(600, 240, 200, 240, "U1", "LMR33630ADDA",
              left=[("2", "VIN"), ("3", "EN"), ("6", "VCC")],
              right=[("8", "SW"), ("7", "BOOT"), ("5", "FB"), ("4", "PG")],
              bottom=[("1", "PGND"), ("EP", "AGND")])
    s.wire(u2["VIN"], (540, 300), (540, 210))

    # EN with programmable UVLO
    s.dot(500, 210)
    s.res_v(500, 240, "R4", "750 k")
    s.line(500, 210, 500, 240)
    s.wire((500, 304), (500, 360), u2["EN"])
    s.dot(500, 360)
    r14 = s.res_v(500, 380, "R5", "100 k")
    s.line(500, 360, 500, 380)
    s.gnd(500, r14["2"][1])

    # VCC - routed under the EN divider so it crosses nothing
    s.wire(u2["VCC"], (570, 420), (570, 490), (440, 490), (440, 510))
    c14 = s.cap_v(440, 510, "C10", "1 \u00b5F 16 V", 52, side="l")
    s.gnd(440, c14["2"][1])

    # BOOT / SW
    s.line(u2["SW"][0], 288, 940, 288)
    s.cap_v(870, 288, "C11", "100 nF 50 V", 48)
    s.dot(870, 288)
    s.wire(u2["BOOT"], (870, 336))
    s.ind_h(940, 288, "L1", "8.2 \u00b5H  \u2265 6 A", 90)
    s.line(1030, 288, 1560, 288)
    for x, ref, val in ((1070, "C12", "22 \u00b5F 16 V"),
                        (1160, "C13", "22 \u00b5F 16 V"),
                        (1250, "C14", "22 \u00b5F 16 V"),
                        (1340, "C15", "22 \u00b5F 16 V")):
        c = s.cap_v(x, 288, ref, val, 56, side="b")
        s.dot(x, 288)
        s.gnd(x, c["2"][1])
    s.rail(1420, 288, "5V_BUCK")
    s.dot(1420, 288)
    s.tp(1500, 288, "TP2", "t")
    s.dot(1500, 288)
    s.netlabel(1560, 288, "5V_BUCK", "r", "flagp")

    # feedback divider
    s.dot(1480, 288)
    s.wire((1480, 288), (1480, 340))
    s.res_v(1480, 340, "R6", "100 k")
    s.line(1480, 404, 1480, 424)
    s.dot(1480, 424)
    s.netlabel(1420, 424, "FB", "l")
    s.line(1420, 424, 1480, 424)
    r16 = s.res_v(1480, 444, "R7", "24.9 k")
    s.line(1480, 424, 1480, 444)
    s.gnd(1480, r16["2"][1])
    s.wire(u2["FB"], (880, 384))
    s.netlabel(880, 384, "FB", "r")

    # power good
    s.line(u2["PG"][0], 432, 900, 432)
    s.dot(900, 432)
    s.res_h(920, 432, "R8", "10 k", 80)
    s.line(900, 432, 920, 432)
    s.netlabel(1000, 432, "5V_BUCK", "r", "flagp")
    s.wire((900, 432), (900, 512), (960, 512))
    s.diode_h(960, 512, "D4", "red 0805", 56, "led", True, flip=True)
    s.res_h(1036, 512, "R9", "1 k", 80)
    s.line(1016, 512, 1036, 512)
    s.netlabel(1116, 512, "5V_BUCK", "r", "flagp")
    s.text(960, 570, "D4 lights when PG is LOW = \"5 V rail bad\".", "ts")
    s.text(960, 586, "It runs off 5V_BUCK, so a dead buck leaves it dark.",
           "ts")

    s.gnd(u2["PGND"][0], u2["PGND"][1])
    s.gnd(u2["AGND"][0], u2["AGND"][1])
    s.text(600, 540, "EP is both AGND and the heatsink \u2014 solder it to "
                     "the", "ts")
    s.text(600, 556, "L2 ground plane with \u2265 9 thermal vias.", "ts")

    s.table(1584, 400, ["Item", "TI value"],
            [["f_SW", "400 kHz (A grade)"],
             ["L", "8 \u00b5H (8.2 \u00b5H fitted)"],
             ["C_OUT", "4 \u00d7 22 \u00b5F"],
             ["R_FBT", "100 k\u03a9"],
             ["R_FBB", "24.9 k\u03a9"],
             ["C_IN + C_HF", "10 \u00b5F + 220 nF"],
             ["C_BOOT", "100 nF"],
             ["C_VCC", "1 \u00b5F"],
             ["C_FF", "not fitted"]],
            [92, 118], "DATASHEET TABLE 9-2")

    # ============================================================= TPS2116
    s.zone(46, 660, 1080, 660, "5 V PRIORITY MUX \u2014 buck wins, USB is the "
                               "fallback")
    u3 = s.ic(420, 720, 220, 180, "U2", "TPS2116DRL",
              left=[("3", "VIN1"), ("5", "MODE"), ("4", "PR1")],
              right=[("2,7", "VOUT"), ("8", "ST")],
              bottom=[("6", "VIN2"), ("1", "GND")])
    s.netlabel(180, 765, "5V_BUCK", "l", "flagp")
    s.line(180, 765, 400, 765)
    c20 = s.cap_v(250, 765, "C16", "10 \u00b5F 16 V", 56, side="b")
    s.dot(250, 765)
    s.gnd(250, c20["2"][1])
    s.wire(u3["MODE"], (370, 810), (370, 765))
    s.dot(370, 765)
    s.text(300, 700, "MODE tied to VIN1 = automatic priority mode", "ts")

    s.dot(300, 765)
    s.res_v(300, 785, "R10", "100 k")
    s.line(300, 765, 300, 785)
    s.wire((300, 849), (300, 855), u3["PR1"])
    s.dot(300, 855)
    r20 = s.res_v(300, 875, "R11", "33 k")
    s.line(300, 855, 300, 875)
    s.gnd(300, r20["2"][1])

    s.wire(u3["VIN2"], (u3["VIN2"][0], 980), (620, 980))
    s.netlabel(620, 980, "5V_USB", "r", "flagp")
    c21 = s.cap_v(560, 980, "C17", "10 \u00b5F 16 V", 52, side="l")
    s.dot(560, 980)
    s.gnd(560, c21["2"][1])
    s.gnd(u3["GND"][0], u3["GND"][1])

    s.line(u3["VOUT"][0], 780, 900, 780)
    for x, ref, val in ((700, "C18", "22 \u00b5F 16 V"),
                        (820, "C19", "100 nF 50 V")):
        c = s.cap_v(x, 780, ref, val, 56)
        s.dot(x, 780)
        s.gnd(x, c["2"][1])
    s.rail(860, 780, "5V_SYS")
    s.dot(860, 780)
    s.netlabel(900, 780, "5V_SYS", "r", "flagp")

    s.wire(u3["ST"], (670, 840), (670, 890), (1070, 890))
    s.rail(1010, 790, "+3V3")
    s.res_v(1010, 790, "R12", "10 k")
    s.line(1010, 854, 1010, 890)
    s.dot(1010, 890)
    s.tp(1070, 890, "TP15", "r")

    s.note(150, 1030, 950, "SWITCHOVER MATH",
           ["PR1 threshold: V_IN1 switches away when PR1 falls below "
            "V_REF = 1.0 V.",
            "V_trip = 1.0 V \u00d7 (100 k + 33 k) / 33 k = 4.03 V, so the "
            "board hands over to USB only after the buck has genuinely "
            "collapsed.",
            "ST is open-drain and pulls LOW while VIN1 is NOT in use, i.e. "
            "\"running on USB\". TP15 exposes it; it is deliberately not",
            "wired to a GPIO because the only spare pins are GPIO45/46, "
            "which are strapping pins - read VMON instead.",
            "The mux is rated 2.5 A, below the buck's 3 A. Loads above ~2 A "
            "(big LED strips) must be fed from their own connector,",
            "not through 5V_SYS."])

    # ============================================================== AMS1117
    s.zone(1150, 660, 646, 340, "3.3 V LINEAR REGULATOR")
    u4 = s.ic(1330, 740, 170, 110, "U3", "AMS1117-3.3",
              left=[("3", "VIN")], right=[("2", "VOUT")],
              bottom=[("1", "GND")])
    s.netlabel(1180, 795, "5V_SYS", "l", "flagp")
    s.line(1180, 795, 1310, 795)
    for x, ref, val in ((1200, "C20", "10 \u00b5F"),
                        (1270, "C21", "100 nF")):
        c = s.cap_v(x, 795, ref, val, 56, side="b")
        s.dot(x, 795)
        s.gnd(x, c["2"][1])
    s.line(1520, 795, 1700, 795)
    for x, ref, val in ((1570, "C22", "22 \u00b5F tant"),
                        (1650, "C23", "100 nF")):
        c = s.cap_v(x, 795, ref, val, 56, side="b")
        s.dot(x, 795)
        s.gnd(x, c["2"][1])
    s.netlabel(1700, 795, "+3V3", "r", "flagp")
    s.gnd(u4["GND"][0], u4["GND"][1])
    s.text(1180, 920, "SOT-223 tab is VOUT, NOT ground \u2014 the cooling "
                      "pour is a 3V3 pour.", "ts")
    s.text(1180, 936, "C22 is deliberately tantalum: the AMS1117 loop needs "
                      "some ESR and can", "ts")
    s.text(1180, 952, "oscillate on a pure MLCC output. Fit 22 \u00b5F/10 V "
                      "tantalum or an MLCC with 0.5-1 \u03a9", "ts")
    s.text(1180, 968, "in series.", "ts")
    s.text(1180, 988, "Worst case: (5 \u2212 3.3) \u00d7 0.5 A = 0.85 W. "
                      "Give the tab \u2265 300 mm\u00b2 of copper.", "ts")

    return s


# ---------------------------------------------------------------------------
# Sheet 4 - ESP32-S3 module and USB
# ---------------------------------------------------------------------------
def sheet_mcu():
    s = doc.sheet("4 \u00b7 ESP32-S3-WROOM-1-N16R8 & USB-C",
                  "native USB \u00b7 strapping pins respected \u00b7 octal "
                  "PSRAM pins avoided")

    # (module pin, pin name, net); a net starting "nc" is left unconnected
    left = [("1", "GND", "GND"), ("2", "3V3", "+3V3"), ("3", "EN", "EN_MCU"),
            ("4", "IO4", "TMC1_STEP"), ("5", "IO5", "TMC2_STEP"),
            ("6", "IO6", "TMC3_STEP"), ("7", "IO7", "TMC1_DIR"),
            ("8", "IO15", "ESTOP_SENSE"), ("9", "IO16", "TMC_UART_TX"),
            ("10", "IO17", "I2C_SDA"), ("11", "IO18", "I2C_SCL"),
            ("12", "IO8", "TMC2_DIR"), ("13", "IO19", "USB_D-"),
            ("14", "IO20", "USB_D+"), ("15", "IO3", "AIN"),
            ("16", "IO46", "nc \u00b7 spare / ROM print"),
            ("17", "IO9", "TMC3_DIR"), ("18", "IO10", "TMC1_DIAG"),
            ("19", "IO11", "TMC2_DIAG"), ("20", "IO12", "TMC3_DIAG")]
    right = [("21", "IO13", "E4"), ("22", "IO14", "E5"),
             ("23", "IO21", "DRV_EN"), ("24", "IO47", "TMC_UART_RX"),
             ("25", "IO48", "RGB_DATA"),
             ("26", "IO45", "nc \u2013 spare / strap VDD_SPI"),
             ("27", "IO0", "IO0_BOOT"),
             ("28", "IO35", "nc \u2013 octal PSRAM"),
             ("29", "IO36", "nc \u2013 octal PSRAM"),
             ("30", "IO37", "nc \u2013 octal PSRAM"),
             ("31", "IO38", "E0"), ("32", "IO39", "E1"),
             ("33", "IO40", "E2"), ("34", "IO41", "E3"),
             ("35", "IO42", "OUT1_EN"), ("36", "RXD0", "U0RXD"),
             ("37", "TXD0", "U0TXD"), ("38", "IO2", "OUT2_EN"),
             ("39", "IO1", "VMON"), ("40", "GND", "GND"),
             ("41", "EPAD", "GND")]
    u5 = s.ic(640, 150, 320, 940, "U4", "ESP32-S3-WROOM-1-N16R8",
              left=[(num, name, num) for num, name, _ in left],
              right=[(num, name, num) for num, name, _ in right])

    for side, pins in (("l", left), ("r", right)):
        stub = -10 if side == "l" else 10
        for num, _, net in pins:
            x, y = u5[num]
            if net == "GND":
                s.line(x, y, x + stub, y)
                s.gnd(x + stub, y)
            elif net.startswith("nc"):
                s.nc(x, y, net, side)
            else:
                s.netlabel(x, y, net, side,
                           "flagp" if net == "+3V3" else "flag")

    # ------------------------------------------------- 3V3 bank, EN and BOOT
    s.zone(46, 150, 430, 940, "MODULE SUPPLY, RESET & BOOT")
    s.netlabel(150, 200, "+3V3", "l", "flagp")
    s.line(150, 200, 450, 200)
    for x, ref, val in ((190, "C24", "10 \u00b5F"), (280, "C25", "10 \u00b5F"),
                        (370, "C26", "100 nF"), (450, "C27", "100 nF")):
        c = s.cap_v(x, 200, ref, val, 56, side="b")
        if x < 450:
            s.dot(x, 200)
        s.gnd(x, c["2"][1])
    s.text(60, 186, "Place C26/C27 within 3 mm of module pin 2.", "ts")

    s.rail(200, 400, "+3V3")
    s.res_v(200, 400, "R16", "10 k")
    s.line(200, 464, 200, 490)
    s.dot(200, 490)
    s.netlabel(140, 490, "EN_MCU", "l")
    s.line(140, 490, 200, 490)
    c34 = s.cap_v(300, 510, "C28", "10 \u00b5F", 52)
    s.wire((200, 490), (300, 490), (300, 510))
    s.gnd(300, c34["2"][1])
    sw1 = s.button(200, 620, "SW1", "RESET")
    s.line(200, 490, 200, 620)
    s.wire(sw1["2"], (340, 620), (340, 654))
    s.gnd(340, 654)
    s.text(60, 700, "EN network: 10 k pull-up + 10 \u00b5F (same 10 \u00b5F "
                    "used everywhere on 3V3/5 V).", "ts")
    s.text(60, 716, "Gives ~100 ms of POR delay after 3V3 is valid.", "ts")

    s.rail(200, 810, "+3V3")
    s.res_v(200, 810, "R17", "10 k")
    s.line(200, 874, 200, 900)
    s.dot(200, 900)
    s.netlabel(140, 900, "IO0_BOOT", "l")
    s.line(140, 900, 200, 900)
    c35 = s.cap_v(300, 920, "C29", "100 nF", 52)
    s.wire((200, 900), (300, 900), (300, 920))
    s.gnd(300, c35["2"][1])
    sw2 = s.button(200, 1010, "SW2", "BOOT")
    s.line(200, 900, 200, 1010)
    s.wire(sw2["2"], (340, 1010), (340, 1036))
    s.gnd(340, 1036)
    s.text(60, 1062, "GPIO0 is only a strap at reset. There is no runtime",
           "ts")
    s.text(60, 1078, "user button: SW1 resets and SW2 straps for download.",
           "ts")

    # ------------------------------------------------------------- USB-C
    s.zone(1120, 150, 676, 660, "USB-C 2.0 \u2014 data + conservative 500 mA, "
                               "ESD protected")
    j4 = s.conn(1150, 200, "J4", "USB-C 16P 2.0",
                ["VBUS", "GND", "SHLD", "D+", "D-", "CC1", "CC2"], 120,
                "right", 40)
    s.line(1290, 229, 1450, 229)
    s.fuse(1450, 229, "F2", "0.5 A PTC")
    s.line(1514, 229, 1640, 229)
    c36 = s.cap_v(1580, 229, "C30", "10 \u00b5F 16 V", 56, side="b")
    s.dot(1580, 229)
    s.gnd(1580, c36["2"][1])
    s.netlabel(1640, 229, "5V_USB", "r", "flagp")
    s.line(*j4["GND"], 1310, j4["GND"][1])
    s.gnd(1310, j4["GND"][1])
    s.jumper(1340, 309, "JP1", "0 R fitted")
    s.line(1290, 309, 1340, 309)
    s.wire((1386, 309), (1412, 309), (1412, 340))
    s.gnd(1412, 340)
    s.text(1300, 182, "SHLD \u2192 GND through JP1. Replace with 1 M\u03a9 "
                      "\u2225 4.7 nF if the shield must float.", "ts")

    d7 = s.ic(1440, 400, 150, 150, "D5", "USBLC6-2SC6",
              left=[("1", "I/O1"), ("3", "I/O2")],
              right=[("6", "I/O1'"), ("4", "I/O2'")],
              top=[("5", "VBUS")], bottom=[("2", "GND")])
    s.wire(j4["D+"], (1390, 349), (1390, 450), (1420, 450))
    s.wire(j4["D-"], (1360, 389), (1360, 500), (1420, 500))
    s.wire(d7["VBUS"], (1515, 270), (1420, 270), (1420, 229))
    s.dot(1420, 229)
    s.gnd(d7["GND"][0], d7["GND"][1])
    s.line(1610, 450, 1680, 450)
    s.netlabel(1680, 450, "USB_D+", "r")
    s.line(1610, 500, 1680, 500)
    s.netlabel(1680, 500, "USB_D-", "r")

    s.wire(j4["CC1"], (1320, 429), (1320, 620), (1360, 620))
    s.res_h(1360, 620, "R18", "5.1 k 1%", 90)
    s.wire((1450, 620), (1490, 620), (1490, 654))
    s.gnd(1490, 654)
    s.wire(j4["CC2"], (1300, 469), (1300, 710), (1360, 710))
    s.res_h(1360, 710, "R19", "5.1 k 1%", 90)
    s.wire((1450, 710), (1490, 710), (1490, 744))
    s.gnd(1490, 744)
    s.text(1560, 624, "5.1 k on BOTH CC pins =", "ts")
    s.text(1560, 638, "advertise a 5 V UFP sink.", "ts")
    s.text(1560, 714, "Never share one resistor", "ts")
    s.text(1560, 728, "between CC1 and CC2.", "ts")
    s.text(1160, 780, "USB-only operation has a 500 mA total design budget. F2 "
                     "is thermal protection, not a precision limiter.", "ts")

    # --------------------------------------------------- debug header + notes
    s.zone(1120, 840, 676, 250, "DEBUG UART \u00b7 U0")
    j5 = s.conn(1150, 892, "J5", "4P 2.54 mm",
                ["3V3", "U0TXD", "U0RXD", "GND"], 120, "right", 32)
    s.rail(1330, 917, "+3V3")
    s.line(1290, 917, 1330, 917)
    s.netlabel(1400, 949, "U0TXD", "r")
    s.line(1290, 949, 1400, 949)
    s.netlabel(1400, 981, "U0RXD", "r")
    s.line(1290, 981, 1400, 981)
    s.wire(j5["GND"], (1340, 1013), (1340, 1038))
    s.gnd(1340, 1038)
    s.text(1150, 1078, "The console can also run over native USB (USB-Serial-"
                       "JTAG), so J5 is optional.", "ts")

    s.note(46, 1130, 1230, "ESP32-S3-WROOM-1-N16R8 PIN RULES \u2014 verified "
                           "against the Espressif module datasheet",
           ["\u2022 GPIO35/36/37 (module pins 28-30) are wired to the "
            "in-package OCTAL PSRAM on any R8 part and are left unconnected "
            "here. GPIO33/34 are not bonded out at all.",
            "\u2022 Strapping pins left floating so the module's internal weak "
            "pulls decide: GPIO45 (VDD_SPI = 3.3 V, weak pull-down) and "
            "GPIO46 (ROM print + boot, weak pull-down) are the",
            "  only two spare pins on the board and are deliberately left "
            "unconnected \u2014 do not hang a driven signal on either. GPIO0 "
            "has the only external strap: 10 k pull-up + SW2,",
            "  exactly the documented download-mode arrangement.",
            "\u2022 GPIO3 is a strapping pin too (JTAG source select), but "
            "here it only sees J17's 1 k / 100 nF RC (sheet 8), so it still "
            "floats at reset. JTAG source select affects",
            "  neither boot mode nor flash voltage, so using GPIO3 as "
            "ADC1_CH2 is safe.",
            "\u2022 GPIO19/20 carry native USB D-/D+ and nothing else; the "
            "PHY is internal so no series resistors are needed. GPIO43/44 "
            "stay on the U0 debug console.",
            "\u2022 GPIO39-42 are also the external-JTAG pads (MTCK/MTDO/MTDI/"
            "MTMS). 39/40/41 are endstop inputs and 42 is OUT1_EN, "
            "which disables external JTAG",
            "  only \u2014 USB-Serial-JTAG still works.",
            "\u2022 GPIO39 idles HIGH after reset and GPIO21/42/47 fall more "
            "slowly than the rest. Each endstop has its own 10 k pull-up to "
            "3V3 (sheet 8), each output gate a",
            "  10 k pull-down (sheet 9) and each DIAG input a 100 k pull-down "
            "(sheet 5), so nothing is left to the pin's own reset behaviour. "
            "DRV_EN on GPIO21 is the exception \u2014 see sheet 5.",
            "\u2022 ANTENNA: the module must overhang the PCB edge. Keepout of "
            "\u2265 15 mm with no copper, no traces and no ground on any of the "
            "four layers, and no metal (motors,",
            "  heatsinks, connectors, chassis) within that volume. The buck "
            "inductor L1 and all three TMC2209 sit on the far side of the "
            "board (sheet 1 zoning)."])
    return s


# ---------------------------------------------------------------------------
# Sheets 5-7 - the three TMC2209 channels
#
# Every axis gets its own page carrying an identical, complete instance of the
# datasheet standard application circuit (section 3.1). Only the reference
# designators, the STEP/DIR/DIAG nets and the two address straps change.
# ---------------------------------------------------------------------------
AXES = [
    # ref  axis      idx  addr    C0  R0  conn
    ("U5", "M0 / X", 1, (0, 0), 31, 20, "J8"),
    ("U6", "M1 / Y", 2, (1, 0), 39, 26, "J9"),
    ("U7", "M2 / Z", 3, (0, 1), 47, 32, "J10"),
]


def tmc_channel(s, spec):
    """Draw one complete TMC2209 channel into the standard 46,90 zone."""
    ref, axis, idx, addr, c0, r0, jref = spec
    C = ["C%d" % (c0 + i) for i in range(8)]
    R = ["R%d" % (r0 + i) for i in range(6)]
    tag = "M%d" % (idx - 1)
    net = "TMC%d" % idx
    a0, a1 = addr

    s.zone(46, 90, 1260, 1090,
           "AXIS %s \u2014 %s TMC2209-LA-T, complete channel per datasheet "
           "\u00a73.1 \u00b7 UART address %d" % (axis, ref, a0 + 2 * a1))
    u = s.ic(460, 300, 490, 650, ref, "TMC2209-LA-T  QFN28-EP",
             left=[("2", "ENN"), ("16", "STEP"), ("19", "DIR"),
                   ("14", "PDN_UART"), ("11", "DIAG"), ("12", "INDEX"),
                   ("17", "VREF"), ("9", "MS1_AD0"), ("10", "MS2_AD1"),
                   ("7", "SPREAD"), ("13", "CLK"), ("20", "STDBY")],
             right=[("24", "OA1"), ("21", "OA2"), ("26", "OB1"), ("1", "OB2")],
             top=[("22,28", "VS"), ("6", "VCP"), ("4", "CPO"), ("5", "CPI"),
                  ("8", "5VOUT"), ("15", "VCC_IO")],
             bottom=[("23", "BRA"), ("27", "BRB"), ("25", "NC"),
                     ("3,18", "GND"), ("EP", "EP")])

    # ---------------------------------------------------- VS rail + bulk bank
    s.wire(u["VS"], (530, 170))
    s.line(150, 170, 530, 170)
    s.dot(460, 170)
    s.netlabel(150, 170, "VMOTOR", "l", "flagp")
    for x, cref, val, pol in ((200, C[0], "220 \u00b5F 50 V", True),
                              (280, C[1], "10 \u00b5F 50 V", False),
                              (360, C[2], "100 nF 50 V", False)):
        c = s.cap_v(x, 170, cref, val, 56, pol, side="b")
        s.dot(x, 170)
        s.gnd(x, c["2"][1])
    s.text(150, 130, "VS bank \u2014 low-ESR, \u2264 3 mm from pins 22/28, "
                     "return straight into the EP thermal pad", "ts")

    # ------------------------------------------------------- charge pump
    s.cap_v(460, 190, C[6], "100 nF 50 V", 56, side="l")
    s.line(460, 170, 460, 190)
    s.wire((460, 246), (460, 250), (600, 250), (600, 280))
    s.cap_h(670, 200, C[5], "22 nF 50 V", 70)
    s.line(670, 200, 670, 280)
    s.line(740, 200, 740, 280)

    # --------------------------------------------------------- 5VOUT + VCC_IO
    s.wire(u["5VOUT"], (810, 180), (1200, 180))
    c = s.cap_v(1140, 180, C[3], "4.7 \u00b5F 16 V", 56, side="b")
    s.dot(1140, 180)
    s.gnd(1140, c["2"][1])
    s.netlabel(1200, 180, tag + "_5VOUT", "r")

    s.wire(u["VCC_IO"], (880, 225), (1050, 225))
    c = s.cap_v(1010, 225, C[4], "100 nF", 56, side="b")
    s.dot(1010, 225)
    s.gnd(1010, c["2"][1])
    s.netlabel(1050, 225, "+3V3", "r", "flagp")

    # ------------------------------------------------------- control signals
    for name, n in (("ENN", "DRV_ENN"), ("STEP", net + "_STEP"),
                    ("DIR", net + "_DIR"), ("PDN_UART", "TMC_UART"),
                    ("DIAG", net + "_DIAG")):
        p = u[name]
        s.netlabel(300, p[1], n, "l")
        s.line(300, p[1], 440, p[1])

    s.nc(*u["INDEX"], "nc \u00b7 INDEX unused", "l")

    s.netlabel(300, u["VREF"][1], tag + "_VREF", "l")
    s.line(300, u["VREF"][1], 440, u["VREF"][1])

    # ------------------------------------------- address straps + tie-offs
    for y, rref, bit in ((700, R[4], a0), (750, R[5], a1)):
        s.res_h(300, y, rref, "10 k", 90)
        s.line(390, y, 440, y)
        if bit:
            s.rail(300, y, "+3V3")
        else:
            s.gnd(300, y)
    for y in (800, 850, 900):
        s.line(280, y, 440, y)
    s.line(280, 800, 280, 940)
    s.dots((280, 850), (280, 900))
    s.gnd(280, 940)
    s.text(140, 678, "UART address %d" % (a0 + 2 * a1), "ts")

    # -------------------------------------------------- motor outputs -> Jn
    j = s.conn(1140, 470, jref, "4P 2.54 MOTOR", ["A+", "A-", "B+", "B-"], 150,
               "left", 44)
    for name, xv, jp in (("OA1", 1040, "A+"), ("OA2", 1060, "A-"),
                         ("OB1", 1080, "B+"), ("OB2", 1100, "B-")):
        p, q = u[name], j[jp]
        s.wire(p, (xv, p[1]), (xv, q[1]), (1120, q[1]))
    s.text(1140, 700, "2.54 mm pin header, unkeyed:", "ts")
    s.text(1140, 714, "mark pin 1, never hot-plug.", "ts")
    s.text(1140, 728, "\u2265 1.5 mm traces, A/B", "ts")
    s.text(1140, 742, "pairs routed together.", "ts")

    # ------------------------------------------------------ sense resistors
    for name, rref in (("BRA", R[0]), ("BRB", R[1])):
        x = u[name][0]
        s.line(x, 970, x, 1000)
        r = s.res_v(x, 1000, rref, "0.11 \u03a9 1%")
        s.gnd(x, r["2"][1])
    s.gnd(u["NC"][0], u["NC"][1])
    s.gnd(u["GND"][0], u["GND"][1])
    s.gnd(u["EP"][0], u["EP"][1])
    s.text(920, 995, "EP: \u2265 16 thermal vias to L2/L4", "ts")
    s.text(920, 1009, "2512, 1 W sense resistors, low", "ts")
    s.text(920, 1023, "inductance, own GND path", "ts")
    s.text(920, 1037, "back to the EP pad.", "ts")

    # ------------------------------------------------------ VREF divider
    s.text(70, 1000, "CURRENT-SET REFERENCE \u00b7 VREF", "tb")
    s.netlabel(150, 1030, tag + "_5VOUT", "l")
    s.line(150, 1030, 210, 1030)
    s.res_h(210, 1030, R[2], "33 k", 90)
    s.line(300, 1030, 350, 1030)
    s.dot(350, 1030)
    r = s.res_v(350, 1060, R[3], "10 k")
    s.line(350, 1030, 350, 1060)
    s.gnd(350, r["2"][1])
    c = s.cap_v(430, 1060, C[7], "100 nF")
    s.wire((350, 1030), (430, 1030), (430, 1060))
    s.dot(430, 1030)
    s.gnd(430, c["2"][1])
    s.line(430, 1030, 470, 1030)
    s.netlabel(470, 1030, tag + "_VREF", "r")


def sheet_m0():
    s = doc.sheet("5 \u00b7 Axis M0 \u00b7 TMC2209 Reference Channel, UART Bus "
                  "& Enable Chain",
                  "TMC2209-LA-T standard application circuit \u00b7 0.11 \u03a9 "
                  "sense \u00b7 UART address 0")
    tmc_channel(s, AXES[0])

    # ==================================================== shared: UART bus
    s.zone(1330, 90, 466, 340, "SINGLE-WIRE UART BUS \u2014 shared by U5-U7 "
                               "(datasheet fig. 4.1)")
    s.netlabel(1450, 200, "TMC_UART_TX", "l")
    s.line(1450, 200, 1490, 200)
    s.res_h(1490, 200, "R13", "1 k", 80)
    s.wire((1570, 200), (1620, 200), (1620, 270))
    s.line(1450, 270, 1700, 270)
    s.netlabel(1450, 270, "TMC_UART", "l")
    s.dot(1620, 270)
    s.rail(1700, 180, "+3V3")
    s.res_v(1700, 180, "R14", "10 k")
    s.line(1700, 244, 1700, 270)
    s.wire((1560, 270), (1560, 330))
    s.dot(1560, 270)
    s.netlabel(1560, 330, "TMC_UART_RX", "r")
    s.text(1340, 366, "R13 is the only series element the datasheet asks for; "
                      "RXD taps the bus", "ts")
    s.text(1340, 380, "directly. R14 keeps the idle level HIGH so no driver "
                      "sees a framing error", "ts")
    s.text(1340, 394, "before the ESP32 UART is initialised. Set "
                      "GCONF.pdn_disable = 1 in firmware", "ts")
    s.text(1340, 408, "or standstill current reduction fights the bus.", "ts")

    # ================================================= shared: enable chain
    s.zone(1330, 450, 466, 520, "DRIVER ENABLE \u2014 hardware AND of "
                                "firmware and e-stop")
    s.rail(1506, 510, "+3V3")
    s.res_v(1506, 510, "R15", "10 k")
    s.line(1506, 574, 1506, 640)
    s.dot(1506, 590)
    s.line(1506, 590, 1600, 590)
    s.netlabel(1600, 590, "DRV_ENN", "r")
    s.mosfet(1440, 640, "Q1", "AO3400A")
    s.netlabel(1380, 674, "DRV_EN", "l")
    s.line(1380, 674, 1440, 674)
    s.dot(1380, 674)
    r53 = s.res_v(1380, 674, "R53", "10 k", 74, side="l")
    s.line(1380, r53["2"][1], 1506, 748)
    s.line(1506, 708, 1506, 748)
    s.netlabel(1506, 748, "ENN_RTN", "r")
    s.text(1340, 790, "Q1's source is ENN_RTN, not GND: e-stop pole B on J3 "
                      "(sheet 2) closes", "ts")
    s.text(1340, 804, "that return in copper. ENN can therefore only be "
                      "pulled low when DRV_EN", "ts")
    s.text(1340, 818, "is high AND the button is closed \u2014 a hardware "
                      "AND with no firmware in", "ts")
    s.text(1340, 832, "the path. ENN HIGH = all three power stages floating, "
                      "and R15 holds it", "ts")
    s.text(1340, 846, "there, so the board wakes up with the motors "
                      "disabled.", "ts")
    s.text(1340, 866, "The e-stop return is the only condition ANDed with "
                      "DRV_EN. One FET, one", "ts")
    s.text(1340, 880, "series contact, nothing else in the enable path.",
           "ts")
    s.text(1340, 900, "R53 is mandatory: it holds Q1 V_GS at zero while "
                      "GPIO21 is high-impedance", "ts")
    s.text(1340, 914, "during reset, boot and with an unprogrammed MCU. "
                      "Connect it gate-to-source", "ts")
    s.text(1340, 928, "rather than gate-to-board-GND because Q1's source is "
                      "the switched ENN_RTN.", "ts")

    s.table(1360, 1020, ["Driver", "Axis", "MS2_AD1", "MS1_AD0", "Addr"],
            [["U5", "M0 / X", "GND", "GND", "0"],
             ["U6", "M1 / Y", "GND", "+3V3", "1"],
             ["U7", "M2 / Z", "+3V3", "GND", "2"]],
            [66, 74, 78, 78, 44], "UART ADDRESS STRAPS (10 k each)")

    # ================================================ shared: DIAG pull-downs
    s.zone(1330, 1110, 466, 160, "DIAG PULL-DOWNS \u2014 fitted even with "
                                 "a driver left off")
    for i, x in enumerate((1430, 1570, 1710)):
        s.netlabel(x, 1150, "TMC%d_DIAG" % (i + 1), "l")
        s.line(x, 1150, x + 20, 1150)
        r = s.res_v(x + 20, 1150, "R%d" % (62 + i), "100 k")
        s.gnd(x + 20, r["2"][1])
    s.text(1340, 1258, "DIAG is push-pull, so these only hold an empty "
                       "driver site's GPIO low.", "ts")

    s.note(46, 1190, 1240, "CURRENT SETTING \u00b7 VERIFIED AGAINST THE "
                           "TMC2209 DATASHEET",
           ["I_RMS = 325 mV / (R_SENSE + 20 m\u03a9) \u00d7 1/\u221a2 "
            "\u00d7 V_VREF / 2.5 V. Fit 0.11 \u03a9, 2512, 1 W shunts and cap "
            "configured continuous current at 1.4 A_RMS per axis.",
            "GCONF.i_scale_analog resets to 1, so VREF is live at power-up: "
            "the 33 k / 10 k divider takes 5VOUT down to 1.16 V \u2192 "
            "0.82 A_RMS default. Firmware then clears the bit and sets "
            "IRUN/IHOLD over UART.",
            "Do not load 5VOUT beyond the 25 mA the regulator allows; the "
            "divider draws 104 \u00b5A. The 4.7 \u00b5F ceramic must sit "
            "directly between pin 8 and the EP pad.",
            "The 22 nF/50 V charge-pump capacitor sits between CPO and CPI and "
            "the 100 nF/50 V between VCP and VS \u2014 both are mandatory, "
            "both must be \u2264 3 mm from the package.",
            "SPREAD, CLK and STDBY are tied to GND: internal 12 MHz clock, "
            "StealthChop as the reset default (UART can still flip it via "
            "GCONF.en_spreadCycle), standby disabled."])
    return s


def sheet_m1():
    s = doc.sheet("6 \u00b7 Axis M1 \u00b7 TMC2209 Channel & Per-Channel BOM",
                  "identical to M0 \u00b7 UART address 1 \u00b7 MS1_AD0 "
                  "strapped high")
    tmc_channel(s, AXES[1])

    s.table(1336, 130, ["Function", "M0", "M1", "M2"],
            [["VS bulk 220 \u00b5F 50 V SMD", "C31", "C39", "C47"],
             ["VS 10 \u00b5F 50 V", "C32", "C40", "C48"],
             ["VS 100 nF 50 V", "C33", "C41", "C49"],
             ["5VOUT 4.7 \u00b5F 16 V", "C34", "C42", "C50"],
             ["VCC_IO 100 nF", "C35", "C43", "C51"],
             ["CPO-CPI 22 nF 50 V", "C36", "C44", "C52"],
             ["VCP-VS 100 nF 50 V", "C37", "C45", "C53"],
             ["VREF filter 100 nF", "C38", "C46", "C54"],
             ["sense A 0.11 \u03a9 1%", "R20", "R26", "R32"],
             ["sense B 0.11 \u03a9 1%", "R21", "R27", "R33"],
             ["VREF top 33 k", "R22", "R28", "R34"],
             ["VREF bottom 10 k", "R23", "R29", "R35"],
             ["MS1_AD0 strap 10 k", "R24", "R30", "R36"],
             ["MS2_AD1 strap 10 k", "R25", "R31", "R37"],
             ["motor connector", "J8", "J9", "J10"],
             ["driver", "U5", "U6", "U7"]],
            [170, 78, 78, 78], "PER-CHANNEL COMPONENTS \u2014 THE SAME "
                               "CIRCUIT THREE TIMES")

    s.note(1336, 470, 460, "THERMAL BUDGET \u2014 TMC2209 TABLE 20.1",
           ["Datasheet dissipation: 1.4 W at 1.0 A_RMS and 2.8 W at",
            "1.4 A_RMS (24 V, two-phase sinewave, 20-40 kHz chopper).",
            "R_th(j-a) = 30 K/W on a 2s2p multilayer board, R_th(j-c) = 6 K/W.",
            "1.4 W \u2192 \u0394T \u2248 42 K, so 1.0 A_RMS per axis is safe "
            "with a solid",
            "plane; three axes together put 4.2 W into the same copper.",
            "MOSFET R_DS(on) = 0.17 \u03a9 typ / 0.21 \u03a9 max per switch.",
            "",
            "Continuous rating is 1.4 A_RMS/axis only after thermal validation "
            "in the final enclosure.",
            "Rules: \u2265 16 vias (0.3 mm) per EP pad, \u2265 400 mm\u00b2 of "
            "L2 copper",
            "per driver, \u2265 15 mm between EP pads, no other heat source in",
            "that band, and set the OTP/UART prewarning to 120 \u00b0C so",
            "firmware can drop IRUN before the 150 \u00b0C shutdown."])

    s.note(46, 1190, 1240, "WHY EVERY AXIS GETS ITS OWN PAGE",
           ["The three channels are electrically identical, so they are drawn "
            "as three full instances rather than one instance plus a note: it "
            "makes the netlist, the BOM line quantities",
            "and the placement review unambiguous. Sheets 5-7 differ only in "
            "the reference designators, in the STEP/DIR/DIAG nets that reach "
            "the ESP32, and in the two address straps.",
            "M1 is the Y axis on the chessboard, the zen garden and the "
            "line-art machine. On a 2-axis machine leave U7 and its passives "
            "off the board: nothing else depends on them, and R64 (sheet 5) "
            "keeps the empty DIAG input low."])
    return s


def sheet_m2():
    s = doc.sheet("7 \u00b7 Axis M2 \u00b7 TMC2209 Channel & Motor Interface "
                  "Notes",
                  "identical to M0 \u00b7 UART address 2 \u00b7 MS2_AD1 "
                  "strapped high")
    tmc_channel(s, AXES[2])

    s.note(1336, 110, 460, "MOTOR CONNECTOR AND CABLE",
           ["Pinout is A+ A- B+ B- (coil A on pins 1-2, coil B on 3-4).",
            "A NEMA17 with a 4-wire loom: measure the coils before wiring,",
            "a swapped pair inside one coil only reverses that axis, but a",
            "cross-coil swap (A+/B+) makes the motor buzz and stall.",
            "",
            "Never unplug a motor while VMOTOR is live \u2014 and it is live",
            "whenever the board is powered, e-stop or not. The",
            "inductive kick can exceed the 32 V absolute maximum on VS.",
            "Power the board down first; pressing the e-stop is not enough.",
            "",
            "Traces \u2265 1.5 mm on 1 oz copper for 1.5 A_RMS, coil pairs",
            "routed side by side so the chopper current loop stays small,",
            "and kept off L2 so the return current does not cut the plane."])

    s.note(1336, 400, 460, "DIAG / STALLGUARD AND MICROSTEPPING",
           ["DIAG goes HIGH on overtemperature, short, undervoltage or a",
            "StallGuard stall. Driver protection flags require ENN inactive;",
            "the velocity-qualified StallGuard indication is not latched.",
            "Each axis has its own DIAG pin on the ESP32 (GPIO10/11/12), so",
            "firmware can tell which driver faulted; the shared DRV_ENN",
            "then clears all three at once.",
            "",
            "In UART mode MS1/MS2 are the address, NOT the microstep",
            "selector: resolution comes from CHOPCONF.MRES (1 to 1/256",
            "with interpolation), so the straps never need to change when",
            "the microstep setting does.",
            "",
            "StallGuard4 needs StealthChop; sensorless homing therefore",
            "works on this board without any endstop on that axis."])

    s.note(46, 1190, 1240, "AXIS ASSIGNMENT PER PROJECT \u00b7 AND WHAT IS "
                           "DELIBERATELY NOT ON THIS BOARD",
           ["Chessboard: M0 = X gantry, M1 = Y gantry, M2 unused; the "
            "electromagnet hangs on OUT2, the 24 V low-side switch on sheet 9.",
            "Zen garden: M0 = radial arm, M1 = turntable, M2 = optional tool "
            "lift; the LED strip runs from the RGB header J20.",
            "Line-art machine: M0 = X, M1 = Y, M2 = pen lift; the "
            "string-tension sensor lands on J17 \u2192 GPIO3 (ADC1_CH2) and "
            "the six endstops go straight to GPIO (sheet 8).",
            "No motor-current sense amplifier: the TMC2209 regulates coil "
            "current itself and reports through DIAG/StallGuard, so an "
            "external shunt would add heat and noise for no gain.",
            "No opto-isolation on STEP/DIR: everything shares one ground and "
            "the drivers are on the same PCB, which is what makes the 0.11 "
            "\u03a9 sense resistors and short VS loops possible.",
            "No per-axis fuse: F1 protects the whole 12-24 V input and the "
            "drivers current-limit themselves. A shorted motor cable trips "
            "the driver's short detection into DIAG, not the fuse."])
    return s


# ---------------------------------------------------------------------------
# Sheet 8 - endstops straight onto GPIO, one analogue input, I2C headers
# ---------------------------------------------------------------------------
def sheet_io():
    s = doc.sheet("8 \u00b7 Endstops, Analogue Input & I\u00b2C Headers",
                  "6 direct-GPIO endstops \u00b7 one ADC1 input \u00b7 "
                  "400 kHz bus for OLED / expansion")

    # ===================================================== zone A: endstops
    s.zone(46, 84, 780, 660, "ENDSTOP INPUTS \u2014 6 \u00d7 filtered, pulled "
                             "up, no clamp diodes, straight onto GPIO")
    cells = [(60, 140, "J11", "E0", "38", "R40", "R46", "C55"),
             (60, 330, "J12", "E1", "39", "R41", "R47", "C56"),
             (60, 520, "J13", "E2", "40", "R42", "R48", "C57"),
             (440, 140, "J14", "E3", "41", "R43", "R49", "C58"),
             (440, 330, "J15", "E4", "13", "R44", "R50", "C59"),
             (440, 520, "J16", "E5", "14", "R45", "R51", "C60")]
    for x0, y0, jref, net, gpio, rpu, rs, cref in cells:
        s.conn(x0, y0, jref, "3P 2.54", ["3V3", "GND", "SIG"], 100,
               "right", 30)
        s.line(x0 + 120, y0 + 24, x0 + 150, y0 + 24)
        s.netlabel(x0 + 150, y0 + 24, "3V3_SNS", "r", "flagp")
        s.line(x0 + 120, y0 + 54, x0 + 140, y0 + 54)
        s.gnd(x0 + 140, y0 + 54)
        s.line(x0 + 120, y0 + 84, x0 + 150, y0 + 84)
        s.res_h(x0 + 150, y0 + 84, rs, "1 k", 70)
        s.line(x0 + 220, y0 + 84, x0 + 250, y0 + 84)
        s.dot(x0 + 250, y0 + 84)
        s.res_v(x0 + 250, y0 + 20, rpu, "10 k", 64, side="l")
        s.netlabel(x0 + 250, y0 + 20, "+3V3", "r", "flagp")
        c = s.cap_v(x0 + 250, y0 + 100, cref, "100 nF", 52)
        s.line(x0 + 250, y0 + 84, x0 + 250, y0 + 100)
        s.gnd(x0 + 250, c["2"][1])
        s.line(x0 + 250, y0 + 84, x0 + 290, y0 + 84)
        s.netlabel(x0 + 290, y0 + 84, net, "r")
        s.text(x0 + 292, y0 + 64, "GPIO" + gpio, "ts")
    s.text(56, 706, "Either a dry contact from SIG to GND, or a 3.3 V "
                    "open-drain hall sensor fed from pin 1. Both idle high "
                    "and pull low, so firmware selects the type", "ts")
    s.text(56, 720, "per input. 1 k + 100 nF = 100 \u00b5s assert, 10 k gives "
                    "1 ms release. The 1 k : 10 k divider adds 0.30 V, so "
                    "sensor V_OL must stay under 0.55 V to", "ts")
    s.text(56, 734, "clear the ESP32-S3's 0.825 V V_IL. No clamp diodes: the "
                    "1 k and 100 nF are the only protection. Never feed 5 V, "
                    "24 V or PNP into SIG.", "ts")

    # ================================================= zone B: analogue in
    s.zone(846, 84, 470, 400, "ANALOGUE INPUT \u2192 GPIO3 / ADC1_CH2")
    s.conn(880, 150, "J17", "3P 2.54", ["3V3", "GND", "AIN"], 100,
           "right", 40)
    s.line(1000, 179, 1030, 179)
    s.netlabel(1030, 179, "+3V3", "r", "flagp")
    s.line(1000, 219, 1040, 219)
    s.gnd(1040, 219)
    s.line(1000, 259, 1060, 259)
    s.res_h(1060, 259, "R52", "1 k", 70)
    s.line(1130, 259, 1170, 259)
    s.dot(1170, 259)
    c61 = s.cap_v(1170, 275, "C61", "100 nF", 52)
    s.line(1170, 259, 1170, 275)
    s.gnd(1170, c61["2"][1])
    s.line(1170, 259, 1210, 259)
    s.netlabel(1210, 259, "AIN", "r")
    s.text(866, 384, "The board has one analogue input. GPIO3 is ADC1_CH2, "
                     "so it", "ts")
    s.text(866, 400, "can be read while WiFi is up \u2014 ADC2 cannot. "
                     "1 k + 100 nF gives", "ts")
    s.text(866, 416, "~100 \u00b5s settling, which is plenty for a tension "
                     "or pot reading.", "ts")
    s.text(866, 436, "No clamp diode here either \u2014 R52 and C61 are the "
                     "protection.", "ts")
    s.text(866, 456, "A load cell needs its own amplifier (HX711 or INA125) "
                     "on the", "ts")
    s.text(866, 472, "sensor; use 0-2.9 V for calibrated measurement. Never "
                     "exceed 3.3 V or feed 5 V / 24 V.", "ts")

    # ========================================= zone C: endstop sensor supply
    s.zone(846, 504, 470, 240, "ENDSTOP SENSOR SUPPLY \u00b7 protected 3V3")
    s.netlabel(880, 580, "+3V3", "l", "flagp")
    s.line(880, 580, 920, 580)
    s.fuse(920, 580, "F3", "0.1 A PTC")
    s.line(984, 580, 1130, 580)
    s.dot(1050, 580)
    c63 = s.cap_v(1050, 580, "C63", "10 \u00b5F 16 V", 52)
    s.gnd(1050, c63["2"][1])
    s.netlabel(1130, 580, "3V3_SNS", "r", "flagp")
    s.text(866, 676, "Pin 1 of J11-J16 only. A pinched sensor cable trips F3 "
                     "instead of", "ts")
    s.text(866, 692, "browning out the ESP32. Plain switches ignore this "
                     "branch entirely.", "ts")
    s.text(866, 712, "F3 holds 0.1 A, so six 3.6 mA hall sensors leave "
                     "4\u00d7 margin.", "ts")

    # ==================================================== GPIO map + notes
    s.table(1346, 130, ["Signal", "ESP32-S3 pin"],
            [["E0      J11", "GPIO38"],
             ["E1      J12", "GPIO39"],
             ["E2      J13", "GPIO40"],
             ["E3      J14", "GPIO41"],
             ["E4      J15", "GPIO13"],
             ["E5      J16", "GPIO14"],
             ["AIN     J17", "GPIO3 / ADC1_CH2"],
             ["I2C_SDA J6/J7", "GPIO17"],
             ["I2C_SCL J6/J7", "GPIO18"]],
            [250, 200], "WHAT THIS SHEET COSTS IN GPIO")

    s.note(1346, 340, 450, "WHY ENDSTOPS GO STRAIGHT TO THE MCU",
           ["All six endstops sit on real GPIOs with real edge interrupts",
            "rather than behind an I\u00b2C expander, so an assert is visible "
            "to",
            "firmware immediately instead of costing a bus round trip behind",
            "an interrupt pin. That is worth the pins it spends: GPIO38-41",
            "plus GPIO13/14 carry them, and the table above is the bill.",
            "",
            "The same argument covers the analogue side. One input on GPIO3",
            "(ADC1_CH2) and VMOTOR sensing on GPIO1 (ADC1_CH0) both use the",
            "on-chip 12-bit ADC, so there is no external ADC and nothing on",
            "the I\u00b2C bus that the board itself depends on."])

    s.note(1346, 530, 450, "I\u00b2C ADDRESS MAP AND BUS BUDGET",
           ["Nothing on the board sits on the bus. R38/R39 (4.7 k) pull it",
            "up for whatever is plugged into J6 or J7.",
            "0x3C or 0x3D: typical SSD1306 OLED on J6. Everything else free.",
            "400 kHz is the design speed; with only header capacitance",
            "(~60 pF) the ESP32 can run the bus faster if you want to.",
            "J7 is a 4-pin header with no INT pin: there is no spare",
            "interrupt-capable GPIO to give it, because GPIO45/46 are",
            "strapping pins and must stay undriven."])

    # ================================================== zone D: I2C bus etc.
    s.zone(46, 764, 780, 400, "I\u00b2C BUS \u00b7 PULL-UPS \u00b7 OLED "
                              "HEADER \u00b7 EXPANSION HEADER")
    s.text(56, 802, "One 4.7 k pair serves the whole bus \u2014 just the "
                    "OLED header and the expansion header "
                    "(~60 pF total).", "ts")
    s.line(500, 820, 560, 820)
    s.rail(560, 820, "+3V3")
    s.dot(560, 820)
    s.res_v(560, 820, "R39", "4.7 k", 60)
    s.line(500, 820, 500, 896)
    s.res_v(500, 896, "R38", "4.7 k")
    s.line(140, 880, 790, 880)
    s.netlabel(140, 880, "I2C_SCL", "l")
    s.line(140, 960, 750, 960)
    s.netlabel(140, 960, "I2C_SDA", "l")
    s.dots((500, 960), (560, 880))

    j6 = s.conn(180, 1010, "J6", "4P 2.54  OLED", ["3V3", "GND", "SDA", "SCL"],
                140, "right", 30)
    j7 = s.conn(520, 1010, "J7", "4P 2.54  I\u00b2C EXP",
                ["3V3", "GND", "SDA", "SCL"], 150, "right", 30)
    for j, sda, scl in ((j6, 420, 460), (j7, 750, 790)):
        x = j["3V3"][0] + 20
        s.line(*j["3V3"], x, j["3V3"][1])
        s.rail(x, j["3V3"][1], "+3V3")
        s.line(*j["GND"], x, j["GND"][1])
        s.gnd(x, j["GND"][1])
        s.wire(j["SDA"], (sda, j["SDA"][1]), (sda, 960))
        s.wire(j["SCL"], (scl, j["SCL"][1]), (scl, 880))
    s.dots((420, 960), (460, 880))

    s.note(846, 764, 950, "ENDSTOPS \u2014 MECHANICAL SWITCH OR HALL SENSOR, "
                          "SELECTED IN FIRMWARE",
           ["Switch: dry contact between pin 3 (SIG) and pin 2 (GND); pin 1 "
            "unused. Hall: 3.3 V open-drain, VCC on pin 1, OUT on pin 3.",
            "DRV5033 (2.5-38 V, 22 \u03a9 r_DS(on), 13 \u00b5s) is the "
            "reference part; its TO-92 suits frame mounting.",
            "Do not fit a duty-cycled sensor such as the DRV5032, whose "
            "open-drain versions sample at 20 Hz and can take 75 ms to "
            "report.",
            "Allegro A110x needs 3.8 V and will not run here. Decouple each "
            "sensor at the sensor with 100 nF.",
            "A hall sensor that loses power reads as released, unlike an NC "
            "contact: configure the axis inverted in firmware if you want a",
            "cut cable to trip instead."])

    s.note(846, 874, 950, "GPIO CHOICE FOR THE SIX ENDSTOPS",
           ["GPIO38-41 are the old SPI block, freed by deleting the microSD "
            "socket and the SPI expansion header. GPIO13/14 came free with "
            "the",
            "dropped fourth axis. All six are plain input-capable pins on "
            "ADC2 or on no ADC at all, so nothing here fights WiFi.",
            "GPIO39 idles HIGH after reset, which is the released state for "
            "these inputs, so a reset never looks like a triggered endstop.",
            "None of the six is a strapping pin, so an endstop held closed at "
            "power-up cannot change boot mode or flash voltage. That is why",
            "GPIO45/46 stayed empty instead of taking two of these inputs.",
            "Each input has its own 10 k pull-up to 3V3. Firmware also enables "
            "the internal pull-up, which only adds a weak parallel path."])

    s.note(846, 984, 950, "WHAT THE ON-CHIP ADC IS GOOD FOR",
           ["12 bits over an approximately 0-2.9 V calibrated range at 11 dB "
            "attenuation, and the ESP32-S3 SAR ADC is noisier and less "
            "linear than a dedicated part.",
            "That is acceptable here because both measurements are "
            "coarse: \"is the motor supply present, and roughly what\" on "
            "GPIO1,",
            "and a slow tension or position reading on GPIO3. Average in "
            "firmware and apply the calibration curve from efuse.",
            "If a project genuinely needs 16-bit resolution, put an ADS1115 "
            "on J7 as a daughterboard - the bus and the 3V3 pin are there "
            "for it."])
    return s


# ---------------------------------------------------------------------------
# Sheet 9 - power outputs, RGB, LEDs, aux power, test points
# ---------------------------------------------------------------------------
def out_ch(s, qx, oy, hdr, innet, rser, rpd, qref, dref, jref, rail, outnet):
    """One discrete low-side N-FET channel: gate network, FET, flyback, Jn."""
    s.text(qx - 250, oy - 50, hdr, "tb")
    # load rail runs straight through to the connector's +V pin
    s.netlabel(qx - 190, oy, rail, "l", "flagp")
    s.line(qx - 190, oy, qx + 180, oy)
    # gate: 1 k series from the GPIO, 10 k pull-down at the gate node
    s.netlabel(qx - 190, oy + 80, innet, "l")
    s.line(qx - 190, oy + 80, qx - 170, oy + 80)
    s.res_h(qx - 170, oy + 80, rser, "1 k", 64)
    s.line(qx - 106, oy + 80, qx, oy + 80)
    s.dot(qx - 60, oy + 80)
    s.res_v(qx - 60, oy + 96, rpd, "10 k", 56)
    s.line(qx - 60, oy + 80, qx - 60, oy + 96)
    s.gnd(qx - 60, oy + 152)
    q = s.mosfet(qx, oy + 46, qref, "AO3400A")
    s.gnd(q["S"][0], q["S"][1])
    # switched return: drain to connector pin 2, flyback back up to the rail
    s.line(q["D"][0], q["D"][1], qx + 180, oy + 46)
    s.diode_v(qx + 130, oy, dref, "SS36", 46, "s", flip=True)
    s.dots((qx + 130, oy), (qx + 130, oy + 46))
    s.conn(qx + 200, oy - 32, jref, "2P 2.54", ["+V", outnet], 96, "left", 46)


def sheet_outputs():
    s = doc.sheet("9 \u00b7 Power Outputs, RGB, LEDs & Test Points",
                  "2 discrete low-side FET outputs \u00b7 WS2812 \u00b7 rail "
                  "LEDs \u00b7 aux power \u00b7 probe points")

    # ================================================= zone A: OUT1 at 5 V
    s.zone(46, 84, 620, 340, "OUT1 \u2014 5 V LOW-SIDE SWITCH  \u00b7  Q2 "
                             "AO3400A  \u00b7  gate from GPIO42")
    out_ch(s, 350, 170, "OUT1 \u00b7 PWM \u00b7 fan, 5 V solenoid, lamp",
           "OUT1_EN", "R54", "R55", "Q2", "D2", "J18", "5V_SYS", "OUT1")
    s.text(60, 360, "R54 damps the gate edge, R55 holds the gate at 0 V on an "
                    "unprogrammed or reset board \u2014 that", "ts")
    s.text(60, 376, "pull-down is the only thing keeping the output off "
                    "before firmware runs, so do not omit it.", "ts")
    s.text(60, 396, "5V_SYS sits behind the 2.5 A TPS2116 (sheet 3), so the "
                    "AO3400A is the limit here: budget 1 A.", "ts")
    s.text(60, 412, "The load's + lead goes to J18 pin 1 and Q2 breaks the "
                    "return on pin 2. D2 is mandatory.", "ts")

    # =============================================== zone B: OUT2 at 12-24 V
    s.zone(46, 454, 620, 340, "OUT2 \u2014 12-24 V LOW-SIDE SWITCH  \u00b7  "
                              "Q3 AO3400A  \u00b7  gate from GPIO2")
    out_ch(s, 350, 540, "OUT2 \u00b7 PWM \u00b7 electromagnet, 24 V strip",
           "OUT2_EN", "R56", "R57", "Q3", "D3", "J19", "VMOTOR", "OUT2")
    s.text(60, 730, "Fed from VMOTOR, which is live whenever the board is "
                    "powered. Pressing the e-stop cuts motor", "ts")
    s.text(60, 746, "torque at ENN but does NOT de-energise this output. A "
                    "solenoid or brake that must drop", "ts")
    s.text(60, 762, "mechanically needs a 3-pole e-stop button or an external "
                    "contactor \u2014 see sheet 2.", "ts")
    s.text(60, 782, "AO3400A is a 30 V part on a 24 V rail: keep D3's loop "
                    "short and never feed more than 24 V in.", "ts")

    # ================================================= zone C: RGB / WS2812
    s.zone(686, 84, 620, 430, "ADDRESSABLE RGB OUTPUT \u2014 GPIO48 direct, "
                              "strip VDD dropped by D6")
    s.netlabel(760, 160, "5V_SYS", "l", "flagp")
    s.line(760, 160, 800, 160)
    s.diode_h(800, 160, "D6", "SS36", 56, "s", below=False)
    s.line(856, 160, 1120, 160)
    s.text(890, 132, "\u2192 strip VDD \u2248 4.4-4.7 V, load-dependent",
           "ts")
    s.dot(1000, 160)
    c81 = s.cap_v(1000, 160, "C62", "22 \u00b5F 16 V", 56, side="l")
    s.gnd(1000, c81["2"][1])
    s.wire((1120, 160), (1120, 229), (1160, 229))

    s.netlabel(760, 300, "RGB_DATA", "l")
    s.line(760, 300, 1000, 300)
    s.res_h(1000, 300, "R58", "470 R", 64)
    s.wire((1064, 300), (1140, 300), (1140, 269), (1160, 269))

    s.conn(1180, 200, "J20", "3P 2.54 RGB", ["+5V", "DATA", "GND"], 100,
           "left", 40)
    s.wire((1160, 309), (1100, 309))
    s.gnd(1100, 309)

    s.text(700, 400, "No level shifter. D6 lowers strip VDD by one Schottky "
                     "drop, but its drop depends on strip current.", "ts")
    s.text(700, 416, "Use a current WS2812B V6 strip specified for 3.3 V data. "
                     "Older 0.7 \u00d7 VDD variants have no", "ts")
    s.text(700, 432, "guaranteed worst-case margin here; add an external "
                     "AHCT buffer if the fitted strip requires 5 V logic.", "ts")
    s.text(700, 452, "D6 is supply isolation, not a substitute for checking "
                     "the exact pixel datasheet.", "ts")
    s.text(700, 472, "R58 damps the line reflection and must stay at the "
                     "driver end, not at the strip. C62 is the head-of-strip",
           "ts")
    s.text(700, 488, "reservoir the WS2812 datasheet asks for. RGB_DATA is "
                     "GPIO48, so the 800 kHz waveform comes out of the", "ts")
    s.text(700, 504, "RMT peripheral in hardware and is immune to WiFi "
                     "jitter.", "ts")

    # ====================================================== zone D: LEDs
    s.zone(686, 534, 620, 360, "RAIL INDICATOR LEDs")
    rows = [(600, "VMOTOR", "R59", "10 k", "D7", "grn  VMOTOR"),
            (680, "5V_SYS", "R60", "1 k", "D8", "grn  5 V"),
            (760, "+3V3", "R61", "470 R", "D9", "grn  3V3")]
    for y, net, rr, rv, dd, dv in rows:
        s.netlabel(775, y, net, "l", "flagp")
        s.line(775, y, 815, y)
        s.res_h(815, y, rr, rv, 64)
        s.line(879, y, 900, y)
        s.diode_h(900, y, dd, dv, 52, "led")
        s.line(952, y, 980, y)
        s.gnd(980, y)
    s.text(700, 820, "Three always-on rail indicators, and they double as a "
                     "first-look diagnostic: VMOTOR lit but", "ts")
    s.text(700, 836, "5 V dark isolates the buck, 5 V lit but 3V3 dark "
                     "isolates the LDO.", "ts")
    s.text(700, 856, "R59 is the same 10 k as the rest of the board: 2.1 mA "
                     "at 24 V, 45 mW in a 0603.", "ts")
    s.text(700, 876, "There is no discrete STATUS or ERROR LED. Firmware "
                     "state is the first pixel on J20.", "ts")

    # ================================================ zone E: aux power J21
    s.zone(1326, 84, 470, 260, "AUX POWER TAKE-OFF")
    j26 = s.conn(1380, 140, "J21", "4P 2.54  AUX POWER",
                 ["VMOTOR", "5V_SYS", "3V3", "GND"], 160, "right", 30)
    for pin, net in (("VMOTOR", "VMOTOR"), ("5V_SYS", "5V_SYS"),
                     ("3V3", "+3V3")):
        p = j26[pin]
        s.netlabel(p[0], p[1], net, "r", "flagp")
    p = j26["GND"]
    s.line(p[0], p[1], p[0] + 20, p[1])
    s.gnd(p[0] + 20, p[1])
    s.text(1340, 300, "J21 feeds daughterboards \u2014 the chess Hall-sensor "
                      "matrix, an", "ts")
    s.text(1340, 316, "external driver, a servo rail. VMOTOR is limited only "
                      "by F1, so", "ts")
    s.text(1340, 332, "fuse the daughterboard itself. Budget ~150 mA on "
                      "5V_SYS, 300 mA on 3V3.", "ts")

    # ================================================== zone F: test points
    s.zone(1326, 374, 470, 360, "TEST POINTS \u2014 1 mm pads, probe-loop "
                                "friendly")
    tps = [("TP3", "5V_SYS"), ("TP4", "+3V3"), ("TP5", "GND"),
           ("TP6", "GND"), ("TP7", "USB_D+"), ("TP8", "USB_D-"),
           ("TP9", "TMC_UART_TX"), ("TP10", "TMC_UART_RX"),
           ("TP11", "I2C_SDA"), ("TP12", "I2C_SCL"),
           ("TP13", "ESTOP_SENSE"), ("TP14", "TMC1_STEP")]
    for i, (ref, net) in enumerate(tps):
        col, row = i // 6, i % 6
        x = 1390 + 216 * col
        y = 440 + 40 * row
        s.tp(x, y, ref, "l")
        s.line(x, y, x + 30, y)
        if net == "GND":
            s.gnd(x + 30, y)
        else:
            cls = ("flagp" if net in ("VMOTOR", "5V_BUCK", "5V_SYS", "+3V3")
                   else "flag")
            s.netlabel(x + 30, y, net, "r", cls)
    s.text(1340, 668, "TP1 VMOTOR (sheet 2), TP2 5V_BUCK and TP15 "
                      "(TPS2116 ST) on sheet 3 are drawn next to the", "ts")
    s.text(1340, 684, "nodes they belong to. TP1-TP4 are the rail-by-rail "
                      "power check: probe", "ts")
    s.text(1340, 700, "them in order before fitting U4 or any TMC2209.", "ts")
    s.text(1340, 720, "TP5/TP6 sit at opposite corners for a scope ground "
                      "spring; TP7/TP8 are pad-only.", "ts")

    # ================================================= connector index table
    s.table(46, 860, ["Ref", "Part / pitch", "Function"],
            [["J1", "DC-005 barrel 5.5/2.1", "12-24 V DC input"],
             ["J2", "2P screw 2.54", "12-24 V DC input, high current"],
             ["J3", "4P screw 2.54", "2-pole NC e-stop: sense + ENN return"],
             ["J4", "USB-C 16P 2.0", "USB data + 5 V"],
             ["J5", "4P header 2.54 THT", "U0 console / spare UART"],
             ["J6", "4P header 2.54 THT", "I\u00b2C OLED"],
             ["J7", "4P header 2.54 THT", "I\u00b2C expansion"],
             ["J8-J10", "4P header 2.54 THT", "stepper motors M0-M2"],
             ["J11-J16", "3P header 2.54 THT", "endstops E0-E5"],
             ["J17", "3P header 2.54 THT", "analogue in \u2192 GPIO3"],
             ["J18", "2P screw 2.54", "OUT1, 5 V low-side"],
             ["J19", "2P screw 2.54", "OUT2, 12-24 V low-side"],
             ["J20", "3P screw 2.54", "addressable RGB strip"],
             ["J21", "4P screw 2.54", "aux power take-off"],
             ["SW1", "3x6 mm tactile", "RESET (EN)"],
             ["SW2", "3x6 mm tactile", "BOOT strap (GPIO0)"]],
            [80, 170, 350], "CONNECTOR AND CONTROL INDEX")

    s.note(686, 924, 620, "OUTPUT STAGE RULES",
           ["\u2022 Both channels are discrete low-side N-FETs: the load's "
            "positive lead goes to the",
            "  connector's +V pin and the FET breaks the return. Do not swap "
            "the pins.",
            "\u2022 Each gate has a 1 k series resistor and a 10 k pull-down. "
            "The pull-down is what",
            "  leaves the output off on an unprogrammed, reset or "
            "disconnected MCU. Keep it.",
            "\u2022 Flyback diodes are mandatory for solenoids, relays, "
            "valves, pumps and electro-",
            "  magnets. D2 and D3 are both SS36 (60 V / 3 A).",
            "\u2022 The AO3400A sets the limit on both channels: budget 1 A "
            "continuous, V_DS under",
            "  30 V. 5V_SYS is further capped by the 2.5 A mux, and by F2's "
            "500 mA PTC on USB.",
            "\u2022 Both gates are 3.3 V driven. AO3400A V_GS(th) is 1.45 V "
            "max and R_DS(on) is",
            "  31 m\u03a9 at 4.5 V, so expect roughly 50 m\u03a9 at 3.3 V "
            "\u2014 50 mV of drop at 1 A."])

    s.note(686, 1084, 620, "WHY DISCRETE SWITCHES RATHER THAN AN ARRAY",
           ["Two AO3400A low-side switches, reusing the part number already "
            "on the board for",
            "the driver-enable FET. A darlington array would bring a higher "
            "voltage rating and",
            "internal clamps, but also a new part number and more board area "
            "than two SOT-23s.",
            "The trade is that D2 and D3 are the only clamp on either "
            "channel, so an inductive",
            "load with no flyback fitted will take the FET with it.",
            "",
            "Status is the addressable pixel on J20 rather than discrete "
            "status and error LEDs,",
            "which keeps the whole UI to one part and one GPIO."])

    s.note(1326, 764, 470, "BRING-UP ORDER",
           ["1. No ICs fitted: check F1 and D1 orientation. Apply 12 V "
            "through a",
            "   current-limited supply and confirm VMOTOR at TP1.",
            "2. Fit U1 and the buck passives. Confirm 5.0 V \u00b1 2 % at TP2 "
            "with no load.",
            "3. Fit U2 and U3. Confirm TP3 = 5.0 V and TP4 = 3.3 V, and "
            "check the 3V3 LED.",
            "   Then unplug the barrel jack and plug in USB: TP3 must hold "
            "5 V and TP15",
            "   must pull LOW, which is the mux reporting it has handed over.",
            "4. Fit U4. Confirm USB enumeration and flash a blink sketch. "
            "GPIO0 strap",
            "   check. Read VMON on GPIO1 and confirm it tracks the input.",
            "5. Close J3 (or link A1-A2 and B1-B2). Confirm ESTOP_SENSE goes "
            "low.",
            "6. Fit Q1. With the ESP32 held in reset, confirm DRV_ENN sits at "
            "3V3, then",
            "   drive DRV_EN high and confirm DRV_ENN falls only while J3 is "
            "closed.",
            "7. Fit U5 only. Read IFCNT over UART at address 0, then run one "
            "axis at 0.3 A.",
            "8. Fit U6 and U7, then Q2/Q3 and the output stages last.",
            "9. Cold-start F1 a dozen times from the real supply with "
            "everything fitted.",
            "   Inrush is about a third of the fast-acting I\u00b2t, so if it "
            "ever nuisance-trips,",
            "   drop the 6.3 A Slo-Blo 045406.3MR into the same holder."])

    s.note(1326, 1056, 470, "SPARE CAPACITY",
           ["GPIO45 and GPIO46 are the only unused pins, and both are "
            "strapping",
            "pins: leave them unconnected rather than routing them to a "
            "header.",
            "GPIO0 is the BOOT strap and GPIO19/20 and GPIO43/44 are USB and "
            "the",
            "debug console. Everything else is allocated on sheets 4-9.",
            "If a project needs more I/O, hang it off J7 (I\u00b2C) or J21 "
            "(power) as a",
            "daughterboard. There is no room left on the module itself."])
    return s


SHEETS = [sheet_overview, sheet_input, sheet_supplies, sheet_mcu,
          sheet_m0, sheet_m1, sheet_m2,
          sheet_io, sheet_outputs]


OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                   "generated", "omnimotion_schematic.svg")


def build():
    """Draw every sheet, refusing a drawing whose wiring is ambiguous."""
    for fn in SHEETS:
        fn()
    problems = doc.problems()
    if problems:
        raise SystemExit("schematic connectivity errors:\n  " +
                         "\n  ".join(problems))
    return doc


def emit(path, text):
    """Write a generated file or, with --check, only compare it."""
    name = os.path.basename(path)
    if "--check" in sys.argv[1:]:
        with open(path, encoding="utf-8", newline="") as f:
            if f.read() != text:
                raise SystemExit("%s is stale: rerun without --check" % name)
        print("%s is up to date" % name)
    else:
        with open(path, "w", encoding="utf-8", newline="") as f:
            f.write(text)
        print("wrote %s" % name)


def main():
    build()
    emit(OUT, doc.svg())


if __name__ == "__main__":
    main()
