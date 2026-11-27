/// One bit per connector: bit n is endstop En, and E0-E5 are J11-J16.
pub const ENDSTOPS: usize = 6;

/// What is wired to a connector. Both sensor types idle high through the 10 k
/// pull-up and pull low when they mean something, so only the settling time
/// differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndstopKind {
    /// Not fitted. Never reports triggered, whatever the pin reads.
    Unused,
    /// Dry contact between pin 3 (SIG) and pin 2 (GND).
    Switch,
    /// 3.3 V open-drain hall sensor, powered from pin 1.
    Hall,
}

/// What it means when a connector triggers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndstopRole {
    /// A hard limit: latches `Fault::Endstop` and disarms.
    Limit,
    /// A datum the machine deliberately drives into. Debounced and readable,
    /// but never raises a fault, so a job may cross it as often as it likes.
    Reference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndstopConfig {
    pub kind: EndstopKind,
    pub role: EndstopRole,
    /// True when a low input means triggered. False inverts it, so
    /// normally-closed wiring trips when the cable is cut.
    pub active_low: bool,
}

impl EndstopConfig {
    pub const UNUSED: Self = Self {
        kind: EndstopKind::Unused,
        role: EndstopRole::Limit,
        active_low: true,
    };

    pub const SWITCH: Self = Self {
        kind: EndstopKind::Switch,
        role: EndstopRole::Limit,
        active_low: true,
    };

    pub const HALL: Self = Self {
        kind: EndstopKind::Hall,
        role: EndstopRole::Limit,
        active_low: true,
    };

    pub const SWITCH_REFERENCE: Self = Self {
        role: EndstopRole::Reference,
        ..Self::SWITCH
    };

    pub const HALL_REFERENCE: Self = Self {
        role: EndstopRole::Reference,
        ..Self::HALL
    };

    /// A contact bounces for a few milliseconds; a hall output is solid state
    /// and the 100 us input RC has already settled it.
    fn samples(self) -> u8 {
        match self.kind {
            EndstopKind::Unused => u8::MAX,
            EndstopKind::Switch => 3,
            EndstopKind::Hall => 1,
        }
    }
}

/// The fitted endstops that `inputs` (bit n is the level of En) read as
/// triggered, undebounced.
pub fn triggered(config: &[EndstopConfig; ENDSTOPS], inputs: u8) -> u8 {
    let mut mask = 0;
    for (endstop, config) in config.iter().enumerate() {
        if config.kind == EndstopKind::Unused {
            continue;
        }
        let low = inputs & (1 << endstop) == 0;
        if low == config.active_low {
            mask |= 1 << endstop;
        }
    }
    mask
}

/// Debounces the six endstop inputs against their configured sensor types.
pub struct EndstopBank {
    config: [EndstopConfig; ENDSTOPS],
    agreed: [u8; ENDSTOPS],
}

impl EndstopBank {
    pub fn new(config: [EndstopConfig; ENDSTOPS]) -> Self {
        Self {
            config,
            agreed: [0; ENDSTOPS],
        }
    }

    pub fn config(&self, endstop: usize) -> Option<EndstopConfig> {
        self.config.get(endstop).copied()
    }

    /// Bits for the fitted connectors serving `role`. Intersect with a `sample`
    /// result to separate faults from datums a job is expected to hit.
    pub fn role_mask(&self, role: EndstopRole) -> u8 {
        let mut mask = 0;
        for (endstop, config) in self.config.iter().enumerate() {
            if config.kind != EndstopKind::Unused && config.role == role {
                mask |= 1 << endstop;
            }
        }
        mask
    }

    pub fn triggered_now(&self, inputs: u8) -> u8 {
        triggered(&self.config, inputs)
    }

    /// Feeds one raw sample of the six inputs and returns the endstops that
    /// have been triggered for their configured number of samples.
    pub fn sample(&mut self, inputs: u8) -> u8 {
        let now = self.triggered_now(inputs);
        let mut confirmed = 0;

        for endstop in 0..ENDSTOPS {
            let bit = 1 << endstop;
            if now & bit == 0 {
                self.agreed[endstop] = 0;
                continue;
            }
            self.agreed[endstop] = self.agreed[endstop].saturating_add(1);
            if self.agreed[endstop] >= self.config[endstop].samples() {
                confirmed |= bit;
            }
        }

        confirmed
    }

    /// True while a level is part way through its debounce, which is when the
    /// caller must keep polling instead of waiting for the next interrupt.
    pub fn settling(&self) -> bool {
        self.agreed.iter().any(|&agreed| agreed > 0)
    }

    pub fn reset(&mut self) {
        self.agreed = [0; ENDSTOPS];
    }
}
