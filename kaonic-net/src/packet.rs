use kaonic_radio::frame::Frame;

pub struct Packet<const S: usize> {
    frame: Frame<S>,
}
