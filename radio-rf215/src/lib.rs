use bus::Bus;
use transceiver::{Band09, Band24, Transreceiver};

pub mod bus;
pub mod error;
pub mod radio;
pub mod regs;
pub mod transceiver;

pub struct Rf215<I: Bus + Copy> {
    trx_09: Transreceiver<Band09, I>,
    trx_24: Transreceiver<Band24, I>,
}
