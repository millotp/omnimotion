//! Register blocks reached through `GPIO::regs()` and `TIMG0::regs()`, in the svd2rust shape the step ISR
//! uses. The TIMG0 registers act as on the chip: LOADLO/LOADHI only stage a value that a write to LOAD
//! copies into the counter, CONFIG.EN stops and resumes the counter where it stood, and INT_CLR is the only
//! way to clear a pending (level-triggered) interrupt.

pub struct Bits(pub(crate) u32);

impl Bits {
    /// # Safety
    /// Mirrors svd2rust: any value is accepted.
    pub unsafe fn bits(&mut self, value: u32) -> &mut Self {
        self.0 = value;
        self
    }

    pub fn load_lo(&mut self) -> ValueW<'_> {
        ValueW(self)
    }
    pub fn load_hi(&mut self) -> ValueW<'_> {
        ValueW(self)
    }
    pub fn load(&mut self) -> ValueW<'_> {
        ValueW(self)
    }
}

pub struct Read(pub(crate) u32);

impl Read {
    pub fn bits(&self) -> u32 {
        self.0
    }
}

fn written(f: impl FnOnce(&mut Bits) -> &mut Bits) -> u32 {
    let mut w = Bits(0);
    f(&mut w);
    w.0
}

// ---- GPIO ----

pub struct GpioRegs;
pub struct W1ts;
pub struct W1tc;
pub struct Out;

impl GpioRegs {
    pub fn out_w1ts(&self) -> &W1ts {
        &W1ts
    }
    pub fn out_w1tc(&self) -> &W1tc {
        &W1tc
    }
    pub fn out(&self) -> &Out {
        &Out
    }
}

impl W1ts {
    pub fn write(&self, f: impl FnOnce(&mut Bits) -> &mut Bits) {
        omnimotion_sim::hal::gpio_update(u64::from(written(f)), 0);
    }
}

impl W1tc {
    pub fn write(&self, f: impl FnOnce(&mut Bits) -> &mut Bits) {
        omnimotion_sim::hal::gpio_update(0, u64::from(written(f)));
    }
}

impl Out {
    pub fn read(&self) -> Read {
        Read(omnimotion_sim::hal::gpio_out() as u32)
    }
}

// ---- TIMG0 ----

pub struct TimgRegs;
pub struct IntClr;
pub struct T;
pub struct TConfig;
pub struct LoadLo;
pub struct LoadHi;
pub struct Load;

impl TimgRegs {
    pub fn int_clr(&self) -> &IntClr {
        &IntClr
    }
    /// Timer 0 is the only one modelled.
    pub fn t(&self, n: usize) -> &T {
        assert_eq!(n, 0, "only TIMG0 timer 0 is simulated");
        &T
    }
}

/// INT_CLR bits written, one per timer.
pub struct IntClrW(u8);

pub struct ClearBit<'a> {
    w: &'a mut IntClrW,
    bit: u8,
}

impl<'a> ClearBit<'a> {
    pub fn clear_bit_by_one(self) -> &'a mut IntClrW {
        self.w.0 |= 1 << self.bit;
        self.w
    }
}

impl IntClrW {
    pub fn t(&mut self, n: u8) -> ClearBit<'_> {
        ClearBit { w: self, bit: n }
    }
}

impl IntClr {
    pub fn write(&self, f: impl FnOnce(&mut IntClrW) -> &mut IntClrW) {
        let mut w = IntClrW(0);
        f(&mut w);
        if w.0 & 1 != 0 {
            omnimotion_sim::hal::timer(|t, _| t.int_raw = false);
        }
    }
}

impl T {
    pub fn config(&self) -> &TConfig {
        &TConfig
    }
    pub fn loadlo(&self) -> &LoadLo {
        &LoadLo
    }
    pub fn loadhi(&self) -> &LoadHi {
        &LoadHi
    }
    pub fn load(&self) -> &Load {
        &Load
    }
}

#[derive(Clone, Copy)]
pub struct ConfigW {
    en: bool,
    alarm_en: bool,
}

pub struct Flag<'a> {
    w: &'a mut ConfigW,
    alarm: bool,
}

impl<'a> Flag<'a> {
    fn put(self, value: bool) -> &'a mut ConfigW {
        if self.alarm {
            self.w.alarm_en = value;
        } else {
            self.w.en = value;
        }
        self.w
    }
    pub fn set_bit(self) -> &'a mut ConfigW {
        self.put(true)
    }
    pub fn clear_bit(self) -> &'a mut ConfigW {
        self.put(false)
    }
}

impl ConfigW {
    pub fn en(&mut self) -> Flag<'_> {
        Flag {
            w: self,
            alarm: false,
        }
    }
    pub fn alarm_en(&mut self) -> Flag<'_> {
        Flag {
            w: self,
            alarm: true,
        }
    }
}

pub struct BitR(bool);

impl BitR {
    pub fn bit_is_clear(&self) -> bool {
        !self.0
    }
}

pub struct ConfigR(ConfigW);

impl ConfigR {
    pub fn en(&self) -> BitR {
        BitR(self.0.en)
    }
}

impl TConfig {
    pub fn read(&self) -> ConfigR {
        ConfigR(omnimotion_sim::hal::timer(|t, _| ConfigW {
            en: t.en,
            alarm_en: t.alarm_en,
        }))
    }

    pub fn modify(&self, f: impl for<'w> FnOnce(&ConfigR, &'w mut ConfigW) -> &'w mut ConfigW) {
        let r = self.read();
        let mut w = r.0;
        f(&r, &mut w);
        omnimotion_sim::hal::timer(|t, now| {
            t.set_enabled(w.en, now);
            t.alarm_en = w.alarm_en;
        });
    }
}

pub struct ValueW<'a>(&'a mut Bits);

impl<'a> ValueW<'a> {
    /// # Safety
    /// Mirrors svd2rust: any value is accepted.
    pub unsafe fn bits(self, value: u32) -> &'a mut Bits {
        self.0 .0 = value;
        self.0
    }
}

impl LoadLo {
    pub fn write(&self, f: impl FnOnce(&mut Bits) -> &mut Bits) {
        let lo = u64::from(written(f));
        omnimotion_sim::hal::timer(|t, _| t.reload = (t.reload & !0xFFFF_FFFF) | lo);
    }
}

impl LoadHi {
    /// The counter is 54 bits wide.
    pub fn write(&self, f: impl FnOnce(&mut Bits) -> &mut Bits) {
        let hi = u64::from(written(f) & 0x3F_FFFF);
        omnimotion_sim::hal::timer(|t, _| t.reload = (t.reload & 0xFFFF_FFFF) | hi << 32);
    }
}

/// Any write to LOAD copies LOADLO/LOADHI into the counter.
impl Load {
    pub fn write(&self, f: impl FnOnce(&mut Bits) -> &mut Bits) {
        written(f);
        omnimotion_sim::hal::timer(|t, now| {
            let value = t.reload;
            t.set_counter(value, now);
        });
    }
}
