#!/usr/bin/env python3
"""Build omnimotion_BOM.csv straight out of the schematic generator.

Every symbol drawn with a reference designator is recorded in `Sheet.parts`,
so the BOM cannot drift away from the schematic: each designator must be
unique and map to exactly one BOM line, and any value that is not in
VALUE_MAP below is reported as an error instead of being silently dropped.

Run:  python3 scripts/gen_bom.py [--check]
"""
import collections
import csv
import io
import os
import re
import urllib.parse

import gen_schematic as g

# ---------------------------------------------------------------------------
# part library: key -> BOM line data
# ---------------------------------------------------------------------------
R = "Thick film chip resistor, 1 %, 100 mW"
RES = "Yageo"
MUR = "Murata"
CER = "MLCC, X7R"


def r(val, mpn, pkg="0603", desc=R, note=""):
    return dict(value=val, desc=desc + ", " + pkg, mfr=RES, mpn=mpn,
                pkg=pkg, rating="\u2265 50 V working", note=note)


def c(val, mpn, pkg, rating, desc=CER, mfr=MUR, note=""):
    return dict(value=val, desc=desc + ", " + pkg, mfr=mfr, mpn=mpn, pkg=pkg,
                rating=rating, note=note)


LIB = {
    # ---------------------------------------------------------- resistors
    "R_0R11": dict(value="0.11 \u03a9 1 %", mfr="Vishay",
                   mpn="WSL2512R1100FEA", pkg="2512",
                   desc="Power metal strip current-sense resistor, 1 W",
                   rating="1 W; 0.22 W at 1.4 A RMS",
                   note="TMC2209 BRA/BRB sense. 1 % or better, low "
                        "inductance; board continuous limit is 1.4 A_RMS/axis."),
    "R_470R": r("470 \u03a9", "RC0603FR-07470RL",
                note="3V3 LED ballast and WS2812 series termination."),
    "R_1K": r("1 k\u03a9", "RC0603FR-071KL",
              note="Endstop/analogue RC series, TMC UART TX series, output-FET "
                   "gate series, PGOOD LED and 5 V rail LED ballast."),
    "R_4K7": r("4.7 k\u03a9", "RC0603FR-074K7L",
               note="I\u00b2C bus pull-ups (R38/R39) only."),
    "R_5K1": r("5.1 k\u03a9 1 %", "RC0603FR-075K1L",
               note="USB-C CC1/CC2 pull-downs. One resistor per CC pin, "
                    "never shared."),
    "R_10K": r("10 k\u03a9", "RC0603FR-0710KL",
               note="E-stop, EN, BOOT, ENN, UART-idle, PGOOD and mux-ST "
                    "pull-ups; endstop pull-ups; TMC2209 VREF lower leg and "
                    "MS1/MS2 address straps; VMOTOR monitor lower leg; "
                    "VMOTOR LED ballast; Q1 gate-source and the two "
                    "output-FET gate pull-downs."),
    "R_24K9": r("24.9 k\u03a9 1 %", "RC0603FR-0724K9L",
                note="LMR33630 FB lower. Datasheet 5.0 V table: "
                     "V_OUT = 1.0 \u00d7 (1 + 100 k / 24.9 k)."),
    "R_33K": r("33 k\u03a9", "RC0603FR-0733KL",
               note="TMC2209 VREF top leg and the TPS2116 PR1 lower leg."),
    "R_100K": r("100 k\u03a9", "RC0603FR-07100KL",
                note="Buck FB top, buck EN/UVLO lower leg, TPS2116 PR1 top, "
                     "the VMOTOR monitor divider into GPIO1, and the three "
                     "DIAG pull-downs R62-R64, which stay fitted when a "
                     "driver is left off."),
    "R_750K": r("750 k\u03a9", "RC0603FR-07750KL",
                note="LMR33630 EN/UVLO top. 750 k / 100 k \u2192 ~10.5 V "
                     "start, matching a 12-24 V input."),
    # --------------------------------------------------------- capacitors
    "C_22N": c("22 nF", "GRM188R71H223KA01D", "0603", "50 V",
               note="TMC2209 charge pump CPO-CPI. Datasheet-fixed value, do "
                    "not substitute."),
    "C_220N": c("220 nF", "GRM188R71H224KAC4D", "0603", "50 V",
                note="LMR33630 high-frequency input bypass. Datasheet-fixed "
                     "value; place directly across VIN and PGND."),
    "C_100N": c("100 nF", "GRM188R71H104KA93D", "0603", "50 V",
                note="Universal decoupling, endstop and analogue-input RC "
                     "filters and C_BOOT. 50 V rating covers VMOTOR-side use."),
    "C_1U16": c("1 \u00b5F", "GRM188R71C105KA12D", "0603", "16 V",
                note="LMR33630 VCC bypass. Use the datasheet value and place "
                     "directly between VCC and AGND."),
    "C_4U7": c("4.7 \u00b5F", "GRM188R61C475KE11D", "0603", "16 V",
               desc="MLCC, X5R",
               note="TMC2209 5VOUT bypass, at the top of the datasheet's "
                    "recommended 2.2-4.7 \u00b5F range."),
    "C_10U16": c("10 \u00b5F", "GRM21BR61C106KE15L", "0805", "16 V",
                 desc="MLCC, X5R",
                 note="5 V / 3V3 bulk, ESP32 EN and USB. "
                      "Nothing smaller than 0805 on this rail."),
    "C_10U50": c("10 \u00b5F", "GRM32ER71H106KA12L", "1210", "50 V",
                 note="VMOTOR and TMC VS ceramic bulk. 50 V part on a "
                      "24 V rail loses ~30 % to DC bias - that is budgeted."),
    "C_22U": c("22 \u00b5F", "GRM31CR61C226ME15L", "1206", "16 V",
               desc="MLCC, X5R",
               note="LMR33630 output (4 \u00d7 22 \u00b5F), TPS2116 output "
                    "and the RGB head-of-strip reservoir."),
    "C_22UT": c("22 \u00b5F", "T491B226K016AT", "EIA 3528-21 (B case)",
                "16 V",
                desc="Tantalum, 10 % ESR-stable", mfr="Kemet",
                note="AMS1117 output. Deliberately tantalum: the LDO loop "
                     "wants some ESR and can ring on a pure MLCC."),
    "C_220U50": c("220 \u00b5F", "EEE-FK1H221P", "10 \u00d7 10.2 mm SMD can",
                  "50 V", desc="Aluminium electrolytic, low ESR, FK series",
                  mfr="Panasonic",
                  note="The only electrolytic: two on VMOTOR and one per "
                       "TMC2209 VS. SMD can, reflow-friendly."),
    # ------------------------------------------------------------- diodes
    "D_TVS": dict(value="SMCJ26A", mfr="Littelfuse", mpn="SMCJ26A",
                  pkg="DO-214AB (SMC)",
                  desc="Unidirectional TVS diode, 1500 W",
                  rating="V_BR 28.9-31.9 V, V_C 42.1 V",
                  note="D1 is the only input protection device on the board "
                       "and does two jobs. Surge clamp: 26 V standoff clears a "
                       "24 V rail. Reverse-polarity crowbar: because it is "
                       "unidirectional, a backwards input forward-biases it "
                       "and it opens F1 - one fuse instead of one board. "
                       "Note that the 42.1 V clamp voltage is ABOVE the "
                       "TMC2209 32 V and LMR33630 38 V limits, so a sustained "
                       "overvoltage is survived only by blowing F1, not by "
                       "clamping. Feed this board 12-24 V."),
    "D_SS36": dict(value="SS36", mfr="Vishay", mpn="SS36-E3/57T",
                   pkg="DO-214AB (SMC)",
                   desc="Schottky rectifier, 60 V / 3 A",
                   rating="60 V, I_F 3 A",
                   note="D2/D3 are the OUT1 and OUT2 flybacks, and the only "
                        "clamp on either channel: the discrete AO3400A "
                        "switches have no internal clamp. D6 sits in series in the "
                        "WS2812 5 V feed and drops the strip's VDD by one "
                        "Schottky drop; that helps 3.3 V data but is not a "
                        "guaranteed level shift, so fit a 3.3 V-compatible "
                        "strip or an AHCT buffer. 60 V covers the 24 V rail; "
                        "using the same part on 5 V means one line item."),
    "D_USBLC6": dict(value="USBLC6-2SC6", mfr="STMicroelectronics",
                     mpn="USBLC6-2SC6", pkg="SOT-23-6",
                     desc="Dual-line USB ESD protection with V_BUS clamp",
                     rating="5.25 V working, 15 kV air discharge",
                     note="Place within 5 mm of J4, D+/D- straight through, "
                          "no stubs."),
    "D_LEDR": dict(value="LED red", mfr="W\u00fcrth Elektronik",
                   mpn="150080RS75000", pkg="0805",
                   desc="Red SMD LED, 620 nm", rating="V_F 2.0 V @ 5 mA",
                   note="D4 only: lights when the buck's PGOOD pin is LOW, "
                        "i.e. \"5 V rail bad\". It runs off 5V_BUCK, so it "
                        "only lights while the buck is up but out of "
                        "regulation; a dead buck leaves it dark. Firmware "
                        "state is the addressable pixel on J20."),
    "D_LEDG": dict(value="LED green", mfr="W\u00fcrth Elektronik",
                   mpn="150080VS75000", pkg="0805",
                   desc="Green SMD LED, 525 nm", rating="V_F 2.9 V @ 5 mA",
                   note="The three rail indicators only: VMOTOR, 5 V and "
                        "3V3. Firmware status is the pixel on J20, not an "
                        "LED here."),
    # -------------------------------------------------------- transistors
    "Q_AO3400A": dict(value="AO3400A", mfr="Alpha & Omega Semiconductor",
                      mpn="AO3400A", pkg="SOT-23",
                      desc="N-channel MOSFET, 30 V / 5.7 A, logic level",
                      rating="V_DS 30 V, V_GS(th) 1.45 V max, "
                             "R_DS(on) 31 m\u03a9 at 4.5 V",
                      note="Q1 is the TMC2209 ENN pull-down: its source is "
                           "ENN_RTN, so it can only pull the shared ENN low "
                           "when DRV_EN is high AND e-stop pole B closes "
                           "that return. Q2 and Q3 are the two low-side "
                           "output switches (5 V on J18, 24 V on J19), each "
                           "with a 1 k gate series and a 10 k gate "
                           "pull-down. Note 30 V V_DS on the 24 V channel: "
                           "keep D3's flyback loop short. AOS AO3400 is "
                           "obsolete on Digi-Key; order the UMW AO3400A "
                           "(4518-AO3400ACT-ND) or use stock."),
    # --------------------------------------------------------------- ICs
    "U_LMR33630": dict(value="LMR33630ADDA", mfr="Texas Instruments",
                       mpn="LMR33630ADDAR", pkg="SO PowerPAD-8 (DDA)",
                       desc="36 V, 3 A synchronous buck converter, 400 kHz",
                       rating="V_IN 3.8-36 V (38 V abs), 3 A",
                       note="Copy the datasheet layout exactly: C_IN on the "
                            "same side, tight VIN-PGND loop, small SW node, "
                            "thermal pad to GND. Digi-Key qty-1 line is the "
                            "tube LMR33630ADDA (296-48844-ND); ADDAR is the "
                            "same die on a 2500-pc reel."),
    "U_TPS2116": dict(value="TPS2116", mfr="Texas Instruments",
                      mpn="TPS2116DRLR", pkg="SOT-583 (DRL), 8 pins",
                      desc="Priority power multiplexer, 2.5 A, low R_ON",
                      rating="1.6-5.5 V (6 V abs), 2.5 A",
                      note="VIN1 = buck (priority), VIN2 = USB. Never sees "
                           "the motor rail. A FET mux rather than a diode-OR "
                           "because it wins twice over: no forward drop, so "
                           "5V_SYS stays at a full 5.0 V, and the buck wins "
                           "by construction instead of by whichever source "
                           "happens to be higher."),
    "U_AMS1117": dict(value="AMS1117-3.3", mfr="Advanced Monolithic Systems",
                      mpn="AMS1117-3.3", pkg="SOT-223",
                      desc="1 A fixed 3.3 V LDO regulator",
                      rating="V_IN 15 V abs, 1 A, dropout 1.3 V",
                      note="0.85 W worst case from 5 V: needs a copper pour "
                           "under the tab, not just the footprint. Digi-Key "
                           "stocks the UMW AMS1117-3.3 (4518-AMS1117-3.3CT-ND), "
                           "not the original AMS reel."),
    "U_ESP32": dict(value="ESP32-S3-WROOM-1-N16R8", mfr="Espressif Systems",
                    mpn="ESP32-S3-WROOM-1-N16R8", pkg="Module 18 \u00d7 "
                                                      "25.5 mm",
                    desc="Wi-Fi + BLE module, 16 MB flash, 8 MB octal PSRAM",
                    rating="3.0-3.6 V, 500 mA peak",
                    note="GPIO35/36/37 are consumed by the octal PSRAM. "
                         "Antenna must overhang the board edge with "
                         "\u2265 15 mm keepout on all layers."),
    "U_TMC2209": dict(value="TMC2209-LA-T", mfr="Analog Devices (Trinamic)",
                      mpn="TMC2209-LA-T", pkg="QFN-28 5 \u00d7 5 mm, EP",
                      desc="Stepper driver, 256 microsteps, StealthChop2, "
                           "UART",
                      rating="V_VS 4.75-29 V rec (32 V abs), 2.0 A peak",
                      note="Exposed pad MUST be soldered to GND with "
                           "\u2265 16 thermal vias. Address set by "
                           "MS1_AD0/MS2_AD1 straps."),
    # ------------------------------------------------------ magnetics etc
    "L_BUCK": dict(value="8.2 \u00b5H", mfr="Bourns",
                   mpn="SRP1265A-8R2M",
                   pkg="13.5 \u00d7 12.5 \u00d7 6.2 mm shielded",
                   desc="Power inductor, carbonyl powder core, "
                        "15.5 m\u03a9 max",
                   rating="I_sat 16 A, I_rms 10.5 A",
                   note="Sized for 3 A output with headroom at 400 kHz; "
                        "shielded so it can sit near the ADC island. KiCad "
                        "has no SRP1265A footprint: Omnimotion:"
                        "L_Bourns_SRP1265A follows the Bourns recommended "
                        "layout (two 4.6 \u00d7 8.0 mm pads, 5.0 mm gap). "
                        "Not pad-compatible with the SRP1245A or the "
                        "Coilcraft XAL6060."),
    "F_MAIN": dict(value="6.3 A / 125 V", mfr="Littelfuse",
                   mpn="015406.3DR",
                   pkg="OMNI-BLOK holder, 9.7 \u00d7 5 mm SMD",
                   desc="Fast-acting cartridge fuse in a replaceable SMD "
                        "holder",
                   rating="6.3 A, 125 V AC/DC, 50 A breaking, 9.17 A\u00b2s "
                          "melting I\u00b2t",
                   note="6.3 A is the IEC value: the 154 series steps "
                        "5 / 6.3 / 7 A and has no 6 A part at all. Fast-"
                        "acting because the pre-assembled 154T range stops at "
                        "5 A, which puts inrush on the critical path: charging the "
                        "1.1 mF of VMOTOR bulk from 24 V costs V\u00b2C/2R "
                        "\u2248 3 A\u00b2s at 100 m\u03a9 of source-plus-ESR "
                        "against a 9.17 A\u00b2s melting I\u00b2t, but only "
                        "~1.4x margin if the supply and leads are very stiff. "
                        "Cold-start the finished board a dozen times on the "
                        "real supply before trusting it; if it ever opens on "
                        "power-up, the holder takes any Nano2, so the 6.3 A "
                        "Slo-Blo 045406.3MR (75 A\u00b2s) is a fuse swap "
                        "rather than a board change and costs no rating. "
                        "Replaceable rather than soldered, "
                        "which matters on a board you will be reworking. Do "
                        "not go above 6.3 A: the 2.54 mm input terminal is "
                        "a 6 A part. A "
                        "resettable PTC was considered and rejected: nothing "
                        "exists at 24 V with a 6 A hold, and 50 m\u03a9 of "
                        "post-trip resistance at 6 A is real loss."),
    "F_USB": dict(value="0.5 A hold", mfr="Littelfuse",
                  mpn="1206L050/15YR",
                  pkg="1206", desc="PPTC resettable fuse",
                  rating="0.5 A hold, 1.0 A trip, 15 V",
                  note="Supports the conservative 500 mA USB-only board budget. "
                       "This thermal PTC is not a precision current limiter; "
                       "external loads must remain off on USB power."),
    "F_SENSOR": dict(value="0.1 A hold", mfr="Littelfuse",
                     mpn="1206L010/60WR",
                     pkg="1206", desc="PPTC resettable fuse",
                     rating="0.1 A hold, 0.25 A trip, 60 V, 1.5 \u03a9 min",
                     note="Feeds pin 1 of the six endstop connectors only, so "
                          "a pinched hall-sensor cable trips F3 instead of "
                          "collapsing 3V3 under the ESP32. Six DRV5033 at "
                          "3.6 mA leave 4\u00d7 margin; move up a size if you "
                          "fit thirstier sensors. Unused when the endstops are "
                          "plain switches."),
    "JMP": dict(value="0 \u03a9", mfr="Yageo", mpn="RC0603JR-070RL",
                pkg="0603", desc="Zero-ohm jumper", rating="1 A",
                note="USB shield to GND. Replace with 1 M\u03a9 \u2225 "
                     "4.7 nF if the shield must float."),
    "SW_TACT": dict(value="Tactile switch", mfr="C&K",
                    mpn="RS-282G05A3-SM RT",
                    pkg="6.0 \u00d7 3.6 \u00d7 2.5 mm SMD, 2 pins",
                    desc="SPST-NO tactile switch, 250 gf, gull wing",
                    rating="12 V / 50 mA, 50 k cycles",
                    note="SW1 RESET, SW2 BOOT strap. No runtime user "
                         "button. Any 2-pin \"3 \u00d7 6 \u00d7 2.5 mm\" "
                         "SMD tactile switch has this outline."),
    # ------------------------------------------------------- connectors
    "J_BARREL": dict(value="DC-005 5.5/2.1", mfr="Kycon",
                     mpn="KLDX-0202-AC", pkg="THT right-angle barrel jack",
                     desc="DC power jack, mates standard 5.5 / 2.1 mm plugs",
                     rating="24 Vdc / 3.5 A, 2.0 mm centre pin",
                     note="Through-hole jack; its normally-closed switch "
                          "contact (pin 3) is unused. 3.5 A is its rating - "
                          "use J2 for anything above that, including a full "
                          "three-motor load."),
    "J_SCREW2": dict(value="2P 2.54 mm", mfr="Phoenix Contact",
                     mpn="1725656 (MPT 0,5/ 2-2,54)", pkg="2.54 mm, 2 pos",
                     desc="PCB screw terminal block, tension sleeve",
                     rating="160 V / 6 A, 0.14-0.5 mm\u00b2 (26-20 AWG)",
                     note="J2 DC input, J18 (5 V) and J19 (24 V) outputs. "
                          "6 A makes J2 the input ceiling, just under F1's "
                          "6.3 A; a three-motor load at 12-24 V draws well "
                          "under that. 0.5 mm\u00b2 (20 AWG) is the largest "
                          "wire it takes."),
    "J_SCREW3": dict(value="3P 2.54 mm", mfr="Phoenix Contact",
                     mpn="1725669 (MPT 0,5/ 3-2,54)", pkg="2.54 mm, 3 pos",
                     desc="PCB screw terminal block, tension sleeve",
                     rating="160 V / 6 A, 0.14-0.5 mm\u00b2 (26-20 AWG)",
                     note="J20 addressable RGB strip (+5 V / DATA / GND)."),
    "J_SCREW4": dict(value="4P 2.54 mm", mfr="Phoenix Contact",
                     mpn="1725672 (MPT 0,5/ 4-2,54)", pkg="2.54 mm, 4 pos",
                     desc="PCB screw terminal block, tension sleeve",
                     rating="160 V / 6 A, 0.14-0.5 mm\u00b2 (26-20 AWG)",
                     note="J3 2-pole e-stop (A1/A2 to firmware, B1/B2 in the ENN return), J21 auxiliary power take-off."),
    "J_USBC": dict(value="USB-C 2.0", mfr="GCT", mpn="USB4105-GF-A",
                   pkg="16-pin SMD + THT hold-down",
                   desc="USB Type-C receptacle, USB 2.0, 16 position",
                   rating="5 A power contacts, 30 V",
                   note="Mid-mount tabs must be soldered; the shield is the "
                        "mechanical anchor. Digi-Key cut-tape SKU is the "
                        "USB4105-GF-A-120 pack (same connector)."),
    "J_HDR254": dict(value="2.54 mm 1\u00d740 strip",
                     mfr="Sullins Connector Solutions",
                     mpn="PRPC040SAAN-RC",
                     pkg="2.54 mm THT, 1\u00d740 breakaway",
                     desc="Breakaway pin header strip, single row, snapped "
                          "to length",
                     rating="3 A per contact, gold flash",
                     note="Buy ONE strip - the Qty column counts headers, not "
                          "strips. Snap it into 3 \u00d7 4-way (J5 debug UART, "
                          "J6 OLED, J7 I\u00b2C expansion: 3V3 / GND / SDA / "
                          "SCL) and 7 \u00d7 3-way (J11-J16 endstops "
                          "3V3 / GND / SIG, J17 analogue in 3V3 / GND / AIN), "
                          "using 33 of the 40 pins. Through-hole, so these go "
                          "on with an iron after reflow like the screw "
                          "terminals - which also makes them far harder to "
                          "tear off than an SMT pad when a sensor cable gets "
                          "yanked."),
    "J_MOTOR": dict(value="4P 2.54 mm header",
                    mfr="Sullins Connector Solutions",
                    mpn="PRPC004SAAN-RC", pkg="2.54 mm THT, 1\u00d74 vertical",
                    desc="Pin header, single row, mates a 1\u00d74 Dupont "
                         "housing",
                    rating="3 A per contact, gold flash",
                    note="Motor phases A+ A- B+ B-. 3 A per pin covers the "
                         "1.4 A_RMS (2 A peak) board limit. Unkeyed: mark "
                         "pin 1 on the housing, and never plug or unplug a "
                         "motor with the board powered - that kills "
                         "TMC2209s."),
    "TP_PAD": dict(value="test pad", desc="1 mm exposed pad, no component",
                   mfr="", mpn="", pkg="", rating="",
                   note="Test points. Bare 1 mm pads with 2 mm keepout; probe "
                        "rings are optional (Keystone 5015 fits if you want a "
                        "loop)."),
}

# designator prefix -> raw schematic value -> library key, in BOM line order
VALUE_MAP = {
    "U": {"LMR33630ADDA": "U_LMR33630", "TPS2116DRL": "U_TPS2116",
          "AMS1117-3.3": "U_AMS1117",
          "ESP32-S3-WROOM-1-N16R8": "U_ESP32",
          "TMC2209-LA-T  QFN28-EP": "U_TMC2209"},
    "Q": {"AO3400A": "Q_AO3400A"},
    "D": {"SMCJ26A": "D_TVS",
          "SS36": "D_SS36",
          "USBLC6-2SC6": "D_USBLC6",
          "red 0805": "D_LEDR",
          "grn  VMOTOR": "D_LEDG", "grn  5 V": "D_LEDG",
          "grn  3V3": "D_LEDG"},
    "L": {"8.2 \u00b5H  \u2265 6 A": "L_BUCK"},
    "F": {"6.3 A / 125 V": "F_MAIN", "0.5 A PTC": "F_USB",
          "0.1 A PTC": "F_SENSOR"},
    "R": {"0.11 \u03a9 1%": "R_0R11", "470 R": "R_470R",
          "1 k": "R_1K", "4.7 k": "R_4K7", "5.1 k 1%": "R_5K1",
          "10 k": "R_10K", "24.9 k": "R_24K9",
          "33 k": "R_33K", "100 k": "R_100K", "750 k": "R_750K"},
    "C": {"22 nF 50 V": "C_22N", "100 nF": "C_100N",
          "220 nF 50 V": "C_220N", "100 nF 50 V": "C_100N",
          "1 \u00b5F 16 V": "C_1U16", "4.7 \u00b5F 16 V": "C_4U7",
          "10 \u00b5F": "C_10U16", "10 \u00b5F 16 V": "C_10U16",
          "10 \u00b5F 50 V": "C_10U50", "22 \u00b5F 16 V": "C_22U",
          "22 \u00b5F tant": "C_22UT", "220 \u00b5F 50 V": "C_220U50"},
    "J": {"DC-005  5.5/2.1": "J_BARREL", "2P 2.54 mm screw": "J_SCREW2",
          "4P 2.54 mm screw": "J_SCREW4",
          "2P 2.54": "J_SCREW2", "3P 2.54 RGB": "J_SCREW3",
          "4P 2.54  AUX POWER": "J_SCREW4", "USB-C 16P 2.0": "J_USBC",
          "4P 2.54 mm": "J_HDR254", "4P 2.54  OLED": "J_HDR254",
          "4P 2.54  I\u00b2C EXP": "J_HDR254", "3P 2.54": "J_HDR254",
          "4P 2.54 MOTOR": "J_MOTOR"},
    "SW": {"RESET": "SW_TACT", "BOOT": "SW_TACT"},
    "JP": {"0 R fitted": "JMP"},
    "TP": {None: "TP_PAD"},
}


def _dk_url(pn):
    return ("https://www.digikey.com/en/products/detail?keywords=" +
            urllib.parse.quote(pn))


# Live Digi-Key cut-tape / qty-1 SKUs (checked 2026-08-31 via Octopart,
# Findchips distributor feeds, and Digi-Key product pages). Prefer CT-ND
# or tube/tray with MOQ 1. Product-page URLs are used when the catalog
# ID is known; otherwise the keywords URL resolves the same SKU.
DK = {
    "U_LMR33630": ("296-48844-ND",
                   "https://www.digikey.com/en/products/detail/"
                   "texas-instruments/LMR33630ADDA/8571549"),
    "U_TPS2116": ("296-TPS2116DRLRCT-ND",
                  _dk_url("296-TPS2116DRLRCT-ND")),
    "U_AMS1117": ("4518-AMS1117-3.3CT-ND",
                  "https://www.digikey.com/en/products/detail/"
                  "umw/AMS1117-3-3/17635254"),
    "U_ESP32": ("5407-ESP32-S3-WROOM-1-N16R8CT-ND",
                "https://www.digikey.com/en/products/detail/"
                "espressif-systems/ESP32-S3-WROOM-1-N16R8/16162642"),
    "U_TMC2209": ("175-TMC2209-LA-TCT-ND",
                  _dk_url("175-TMC2209-LA-TCT-ND")),
    "Q_AO3400A": ("4518-AO3400ACT-ND",
                  _dk_url("4518-AO3400ACT-ND")),
    "D_TVS": ("", _dk_url("SMCJ26A")),
    "D_SS36": ("SS36-E3/57TGICT-ND",
               _dk_url("SS36-E3/57TGICT-ND")),
    "D_USBLC6": ("497-5235-1-ND",
                 "https://www.digikey.com/en/products/detail/"
                 "stmicroelectronics/USBLC6-2SC6/1040559"),
    "D_LEDR": ("", _dk_url("150080RS75000")),
    "D_LEDG": ("732-4986-1-ND",
               _dk_url("732-4986-1-ND")),
    "L_BUCK": ("", _dk_url("SRP1265A-8R2M")),
    "F_MAIN": ("", _dk_url("015406.3DR")),
    "F_USB": ("", _dk_url("1206L050/15YR")),
    "F_SENSOR": ("F1206L010-60WRCT-ND", _dk_url("1206L010/60WR")),
    "R_0R11": ("", _dk_url("WSL2512R1100FEA")),
    "R_470R": ("311-470HRCT-ND", _dk_url("311-470HRCT-ND")),
    "R_1K": ("311-1.00KHRCT-ND", _dk_url("311-1.00KHRCT-ND")),
    "R_4K7": ("311-4.70KHRCT-ND", _dk_url("311-4.70KHRCT-ND")),
    "R_5K1": ("311-5.10KHRCT-ND", _dk_url("311-5.10KHRCT-ND")),
    "R_10K": ("311-10.0KHRCT-ND", _dk_url("311-10.0KHRCT-ND")),
    "R_24K9": ("311-24.9KHRCT-ND", _dk_url("311-24.9KHRCT-ND")),
    "R_33K": ("311-33.0KHRCT-ND", _dk_url("311-33.0KHRCT-ND")),
    "R_100K": ("311-100KHRCT-ND", _dk_url("311-100KHRCT-ND")),
    "R_750K": ("311-750KHRCT-ND", _dk_url("311-750KHRCT-ND")),
    "C_22N": ("", _dk_url("GRM188R71H223KA01D")),
    "C_220N": ("", _dk_url("GRM188R71H224KAC4D")),
    "C_100N": ("", _dk_url("GRM188R71H104KA93D")),
    "C_1U16": ("", _dk_url("GRM188R71C105KA12D")),
    "C_4U7": ("", _dk_url("GRM188R61C475KE11D")),
    "C_10U16": ("", _dk_url("GRM21BR61C106KE15L")),
    "C_10U50": ("", _dk_url("GRM32ER71H106KA12L")),
    "C_22U": ("", _dk_url("GRM31CR61C226ME15L")),
    "C_22UT": ("", _dk_url("T491B226K016AT")),
    "C_220U50": ("", _dk_url("EEE-FK1H221P")),
    "JMP": ("311-0.0GRCT-ND", _dk_url("311-0.0GRCT-ND")),
    "SW_TACT": ("CKN10384CT-ND",
                "https://www.digikey.com/en/products/detail/"
                "c-k/RS-282G05A3-SM-RT/2747190"),
    "J_BARREL": ("2092-KLDX-0202-AC-ND", _dk_url("2092-KLDX-0202-AC-ND")),
    "J_SCREW2": ("", _dk_url("1725656")),
    "J_SCREW3": ("", _dk_url("1725669")),
    "J_SCREW4": ("", _dk_url("1725672")),
    "J_USBC": ("2073-USB4105-GF-A-120CT-ND",
               "https://www.digikey.com/en/products/detail/"
               "gct/USB4105-GF-A-120/14559037"),
    "J_HDR254": ("S1011EC-40-ND", _dk_url("S1011EC-40-ND")),
    "J_MOTOR": ("", _dk_url("PRPC004SAAN-RC")),
    "TP_PAD": ("", ""),
}

# The KiCad footprint each part needs; a tuple when the line covers several
# sizes. check_kicad.py holds the PCB to this, and gen_footprint_sheet.py
# draws it.
R0603 = "Resistor_SMD:R_0603_1608Metric"
C0603 = "Capacitor_SMD:C_0603_1608Metric"
MPT = ("TerminalBlock_Phoenix:TerminalBlock_Phoenix_MPT-0,5-{0}-2.54_1x0{0}"
       "_P2.54mm_Horizontal")
FOOTPRINT = {
    "R_0R11": "Resistor_SMD:R_2512_6332Metric",
    "R_470R": R0603, "R_1K": R0603, "R_4K7": R0603, "R_5K1": R0603,
    "R_10K": R0603, "R_24K9": R0603, "R_33K": R0603, "R_100K": R0603,
    "R_750K": R0603, "JMP": R0603,
    "C_22N": C0603, "C_220N": C0603, "C_100N": C0603, "C_1U16": C0603,
    "C_4U7": C0603,
    "C_10U16": "Capacitor_SMD:C_0805_2012Metric",
    "C_10U50": "Capacitor_SMD:C_1210_3225Metric",
    "C_22U": "Capacitor_SMD:C_1206_3216Metric",
    "C_22UT": "Capacitor_Tantalum_SMD:CP_EIA-3528-21_Kemet-B",
    "C_220U50": "Capacitor_SMD:CP_Elec_10x10.5",
    "D_TVS": "Diode_SMD:D_SMC",
    "D_SS36": "Diode_SMD:D_SMC",
    "D_USBLC6": "Package_TO_SOT_SMD:SOT-23-6",
    "D_LEDR": "LED_SMD:LED_0805_2012Metric",
    "D_LEDG": "LED_SMD:LED_0805_2012Metric",
    "Q_AO3400A": "Package_TO_SOT_SMD:SOT-23",
    "U_LMR33630": "Omnimotion:LMR33630_HSOP8_ThermalVias_9",
    "U_TPS2116": "Package_TO_SOT_SMD:SOT-583-8",
    "U_AMS1117": "Package_TO_SOT_SMD:SOT-223-3_TabPin2",
    "U_ESP32": "RF_Module:ESP32-S3-WROOM-1",
    "U_TMC2209": "Omnimotion:TMC2209_VQFN28_ThermalVias_0.3mm",
    "L_BUCK": "Omnimotion:L_Bourns_SRP1265A",
    "F_MAIN": "Fuse:Fuseholder_Littelfuse_Nano2_154x",
    "F_USB": "Fuse:Fuse_1206_3216Metric",
    "F_SENSOR": "Fuse:Fuse_1206_3216Metric",
    "SW_TACT": "Button_Switch_SMD:SW_SPST_CK_RS282G05A3",
    "J_BARREL": "Connector_BarrelJack:BarrelJack_Kycon_KLDX-0202-xC"
                "_Horizontal",
    "J_SCREW2": MPT.format(2), "J_SCREW3": MPT.format(3),
    "J_SCREW4": MPT.format(4),
    "J_USBC": "Omnimotion:USB_C_Receptacle_GCT_USB4105-xx-A_16P_TopMnt"
              "_Horizontal",
    "J_HDR254": ("Connector_PinHeader_2.54mm:PinHeader_1x03_P2.54mm_Vertical",
                 "Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical"),
    "J_MOTOR": "Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical",
    "TP_PAD": "TestPoint:TestPoint_Pad_D1.0mm",
}


def footprints(key):
    fp = FOOTPRINT[key]
    return (fp,) if isinstance(fp, str) else fp


HEADER = ["Item", "Qty", "References", "Value", "Description",
          "Manufacturer", "Manufacturer P/N", "Package", "Footprint",
          "Rating verified against datasheet", "Sheet",
          "Numero de reference Digi-Key", "URL Digi-Key",
          "Notes / why this part"]

# Not drawn on the schematic: (references, qty, value, description, note)
EXTRA = [
    ("PCB", 1, "Rev A.1",
     "4-layer FR-4, 1.6 mm, 1 oz outer / 1 oz inner, ENIG",
     "L1 signal, L2 solid GND, L3 power planes, L4 signal + GND fill. "
     "ENIG for the QFN and the module pads."),
    ("MH1-MH4", 4, "M3", "M3 mounting holes, 6.5 mm keepout",
     "Plated and tied to GND; the chassis is the EMI return for the "
     "motor cables."),
    ("HS1-HS3", 3, "heatsink (opt)", "Adhesive heatsink with a 6 x 6 mm maximum base",
     "Optional above ~1.2 A_RMS per axis. Centre the base on the TMC2209 "
     "with electrically insulating thermal adhesive and check clearance to "
     "the adjacent capacitors."),
]

OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                   "generated", "omnimotion_BOM.csv")


def split(ref):
    m = re.fullmatch(r"([A-Z]+)(\d+)", ref)
    if not m:
        raise SystemExit("malformed reference designator %r" % ref)
    return m.group(1), int(m.group(2))


def main():
    doc = g.build()
    parts = [(ref, val, s.number) for s in doc.sheets for ref, val in s.parts]

    counts = collections.Counter(ref for ref, _, _ in parts)
    dupes = sorted((r for r, n in counts.items() if n > 1), key=split)
    if dupes:
        raise SystemExit("duplicate reference designators: %s" %
                         " ".join(dupes))

    groups = {}
    unknown = []
    for ref, val, sheet in parts:
        pre = split(ref)[0]
        key = VALUE_MAP.get(pre, {}).get(val)
        if key is None:
            unknown.append((ref, val))
            continue
        e = groups.setdefault((pre, key), {"refs": [], "sheets": set()})
        e["refs"].append(ref)
        e["sheets"].add(sheet)
    if unknown:
        raise SystemExit("unmapped values: %r" % sorted(unknown))

    rows = []
    for pre in VALUE_MAP:
        keys = sorted((k for p, k in groups if p == pre),
                      key=lambda k: min(split(r)[1]
                                        for r in groups[pre, k]["refs"]))
        for k in keys:
            p, e = LIB[k], groups[pre, k]
            refs = sorted(e["refs"], key=split)
            dk_pn, dk_url = DK[k]
            rows.append([
                len(rows) + 1, len(refs), " ".join(refs), p["value"],
                p["desc"], p["mfr"], p["mpn"], p["pkg"],
                " / ".join(footprints(k)), p["rating"],
                ",".join(str(s) for s in sorted(e["sheets"])),
                dk_pn, dk_url, p["note"]])
    for refs, qty, value, desc, note in EXTRA:
        rows.append([len(rows) + 1, qty, refs, value, desc,
                     "", "", "", "", "", "", "", "", note])

    buf = io.StringIO()
    w = csv.writer(buf, lineterminator="\n")
    w.writerow(HEADER)
    w.writerows(rows)
    g.emit(OUT, buf.getvalue())

    pads = sum(1 for ref in counts if split(ref)[0] == "TP")
    print("%d BOM lines, %d placed parts, %d test pads" %
          (len(rows), len(counts) - pads, pads))


if __name__ == "__main__":
    main()
