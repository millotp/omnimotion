pub mod adc {
    use core::marker::PhantomData;

    #[derive(Clone, Copy, Debug)]
    pub enum Attenuation {
        _11dB,
    }

    pub struct AdcCalCurve<ADCI>(PhantomData<ADCI>);

    pub struct AdcConfig<ADCI>(PhantomData<ADCI>);

    impl<ADCI> AdcConfig<ADCI> {
        pub fn new() -> Self {
            Self(PhantomData)
        }

        pub fn enable_pin_with_cal<PIN, CS>(
            &mut self,
            pin: PIN,
            _attenuation: Attenuation,
        ) -> AdcPin<PIN, ADCI, CS> {
            AdcPin {
                _pin: pin,
                _p: PhantomData,
            }
        }
    }

    pub struct AdcPin<PIN, ADCI, CS> {
        _pin: PIN,
        _p: PhantomData<(ADCI, CS)>,
    }

    pub struct Adc<'d, ADCI, Dm> {
        _p: PhantomData<(&'d mut (), ADCI, Dm)>,
    }

    impl<'d, ADCI> Adc<'d, ADCI, crate::Blocking> {
        pub fn new(_adc: ADCI, _config: AdcConfig<ADCI>) -> Self {
            Self { _p: PhantomData }
        }

        /// The only analogue input the board reads is VMON; the curve calibration returns pin mV.
        pub fn read_blocking<PIN, CS>(&mut self, _pin: &mut AdcPin<PIN, ADCI, CS>) -> u16 {
            omnimotion_sim::hal::vmon_mv()
        }
    }
}
