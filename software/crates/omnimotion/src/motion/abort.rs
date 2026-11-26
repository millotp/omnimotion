use core::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AbortTicket(u32);

impl AbortTicket {
    pub const fn generation(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AbortState {
    pub requested: AbortTicket,
    pub acknowledged: AbortTicket,
}

impl AbortState {
    pub fn pending(self) -> bool {
        self.requested != self.acknowledged
    }

    pub fn is_acknowledged(self, ticket: AbortTicket) -> bool {
        generation_is_acknowledged(self.acknowledged.0, ticket.0)
    }
}

/// Wrap-aware request/acknowledgement generations for an ISR-owned abort.
pub struct AbortGenerations {
    requested: AtomicU32,
    acknowledged: AtomicU32,
}

impl AbortGenerations {
    pub const fn new() -> Self {
        Self {
            requested: AtomicU32::new(0),
            acknowledged: AtomicU32::new(0),
        }
    }

    pub fn request(&self) -> AbortTicket {
        AbortTicket(
            self.requested
                .fetch_add(1, Ordering::AcqRel)
                .wrapping_add(1),
        )
    }

    pub fn requested(&self) -> AbortTicket {
        AbortTicket(self.requested.load(Ordering::Acquire))
    }

    pub fn state(&self) -> AbortState {
        let acknowledged = AbortTicket(self.acknowledged.load(Ordering::Acquire));
        let requested = self.requested();
        AbortState {
            requested,
            acknowledged,
        }
    }

    pub fn pending(&self) -> bool {
        self.state().pending()
    }

    pub fn acknowledge(&self, ticket: AbortTicket) {
        self.acknowledged
            .store(ticket.generation(), Ordering::Release);
    }

    pub fn is_acknowledged(&self, ticket: AbortTicket) -> bool {
        self.state().is_acknowledged(ticket)
    }
}

impl Default for AbortGenerations {
    fn default() -> Self {
        Self::new()
    }
}

pub fn generation_is_acknowledged(acknowledged: u32, requested: u32) -> bool {
    acknowledged.wrapping_sub(requested) < (1 << 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_and_acknowledgement_cross_wrap() {
        let aborts = AbortGenerations {
            requested: AtomicU32::new(u32::MAX),
            acknowledged: AtomicU32::new(u32::MAX),
        };

        let wrapped = aborts.request();
        assert_eq!(wrapped.generation(), 0);
        assert!(aborts.state().pending());
        assert!(!aborts.state().is_acknowledged(wrapped));

        aborts.acknowledge(wrapped);
        assert!(!aborts.state().pending());
        assert!(aborts.state().is_acknowledged(AbortTicket(u32::MAX)));
        assert!(aborts.state().is_acknowledged(wrapped));
    }
}
