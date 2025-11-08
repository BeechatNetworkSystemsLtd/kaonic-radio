use kaonic_radio::{error::KaonicError, frame::Frame, platform, radio::Radio};

fn main() {
    simple_logger::SimpleLogger::new().env().init().unwrap();

    log::info!("Start Radio Test");

    let tx_mode = std::env::args().any(|arg| arg == "--tx");

    let mut machine = platform::create_machine().expect("kaonic machine");

    let mut radio = machine.take_radio(0).expect("valid radio module");

    let mut frame = Frame::new();

    let mut counter = 0u64;
    loop {
        if tx_mode {
            frame.copy_from_slice(format!("// TEST DATA {} //", counter).as_bytes());
            match radio.transmit(&frame) {
                Ok(_) => {
                    counter += 1;
                    log::trace!("TX[{:8}] {}", counter, frame.len());
                }
                Err(KaonicError::Timeout) => {
                    log::warn!("TX Timeout");
                }
                Err(_) => {
                    log::error!("transmit error");
                }
            }
            counter += 1;
        } else {
            match radio.receive(&mut frame, core::time::Duration::from_millis(1000)) {
                Ok(recv) => {
                    counter += 1;
                    log::trace!(
                        "RX[{:8}] rssi:{} edv:{} {}",
                        counter,
                        recv.rssi,
                        recv.edv,
                        frame
                    );
                }
                Err(KaonicError::Timeout) => {}
                Err(_) => {
                    log::error!("receive error");
                }
            }

            if let Ok(scan) = radio.scan(core::time::Duration::from_millis(100)) {
                log::trace!("SCAN rssi:{} edv:{}", scan.rssi, scan.edv)
            }
        }
    }
}
