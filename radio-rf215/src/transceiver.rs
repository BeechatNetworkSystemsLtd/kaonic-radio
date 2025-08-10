use crate::bus::Bus;
use crate::radio::{Band, Radio, RadioChannel, RadioFrequency};
use crate::regs::{self, RegisterAddress};

pub struct Band24;
pub struct Band09;

impl Band for Band09 {
    const RADIO_ADDRESS: RegisterAddress = regs::RG_RF09_BASE_ADDRESS;
    const MIN_FREQUENCY: RadioFrequency = 389_500_000;
    const MAX_FREQUENCY: RadioFrequency = 1_020_000_000;
    const FREQUENCY_OFFSET: RadioFrequency = 0;
    const MAX_CHANNEL: RadioChannel = 255;
}

impl Band for Band24 {
    const RADIO_ADDRESS: RegisterAddress = regs::RG_RF24_BASE_ADDRESS;
    const MIN_FREQUENCY: RadioFrequency = 2_400_000_000;
    const MAX_FREQUENCY: RadioFrequency = 2_483_500_000;
    const FREQUENCY_OFFSET: RadioFrequency = 1_500_000_000;
    const MAX_CHANNEL: RadioChannel = 511;
}

pub struct Transreceiver<B: Band, I: Bus> {
    radio: Radio<B, I>,
}

impl<B: Band, I: Bus> Transreceiver<B, I> {
    pub(crate) fn new() -> Self {
        Self {
            radio: Radio::<B, I>::new(),
        }
    }
}
