//! Shares the air between the radios of one node.
//!
//! Two rules, both measured on the kaonic1s with bonded traffic at 6 m:
//!
//! - One radio on the air at a time. The two are a few centimetres apart
//!   with about 20 dB between them, so the one that is not transmitting
//!   assesses the channel as busy anyway; letting both try just costs
//!   the assessment retries, and where they did overlap the far side
//!   failed 4 frames in 10.
//!
//! - A quiet gap after a frame before the *other* radio's frame starts.
//!   The far side reads the first frame out of its buffer — up to 2.6 ms
//!   over the 6 MHz bus, after the interrupt reaches the worker — and a
//!   frame arriving on its sibling during that read-out comes out with a
//!   good header and a corrupt payload. Measured: no gap, half the frames
//!   fail; 2–2.5 ms, none fail but 2 in 100 are lost outright; 3.5 ms
//!   and up, 1 or 2 in 600 fail, the same as a single radio. The same
//!   radio's next frame is further away than that on its own.
//!
//! Loading the frame into the transmitter takes 1.3–2.6 ms and is done
//! before the air is claimed, so the gap is the whole cost of the
//! hand-over. With the gap the air carries a frame every ~13.5 ms,
//! about 900 kbit/s at 1792-byte frames; the receiver's read-out sets
//! that, not the transmitter.
//!
//! `KAONIC_AIR_GAP_US` overrides the gap and `KAONIC_AIR_GATE=off`
//! removes both rules, for measurement.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

struct State {
    /// The radio on the air, if any.
    airborne: Option<usize>,
    /// When the last frame from this node ended.
    last_air_end: Option<Instant>,
}

static STATE: Mutex<State> = Mutex::new(State {
    airborne: None,
    last_air_end: None,
});
static CHANGED: Condvar = Condvar::new();

const GAP: Duration = Duration::from_micros(3500);

fn gap() -> Duration {
    static VALUE: OnceLock<Duration> = OnceLock::new();
    *VALUE.get_or_init(|| {
        Duration::from_micros(super::env::var_or(
            "KAONIC_AIR_GAP_US",
            GAP.as_micros() as u64,
        ))
    })
}

fn gated() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("KAONIC_AIR_GATE").as_deref() != Ok("off"))
}

/// The air belongs to one radio until this is dropped.
pub struct OnAir {
    radio: usize,
}

/// Claims the air for `radio`: returns once no other radio is on it and
/// the gap since the last frame has passed. Covers the channel
/// assessment, the frame, and the wait for its end.
pub fn on_air(radio: usize) -> OnAir {
    if !gated() {
        return OnAir { radio };
    }

    let gap = gap();
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        let now = Instant::now();
        let quiet_at = state.last_air_end.map(|end| end + gap);
        let quiet = quiet_at.map_or(true, |at| now >= at);
        let free = state.airborne.map_or(true, |other| other == radio);
        if quiet && free {
            break;
        }
        state = match quiet_at.filter(|at| free && *at > now) {
            Some(at) => {
                CHANGED
                    .wait_timeout(state, at - now)
                    .unwrap_or_else(|e| e.into_inner())
                    .0
            }
            None => CHANGED.wait(state).unwrap_or_else(|e| e.into_inner()),
        };
    }
    state.airborne = Some(radio);
    OnAir { radio }
}

impl Drop for OnAir {
    fn drop(&mut self) {
        if !gated() {
            return;
        }
        let end = Instant::now();
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if state.airborne == Some(self.radio) {
            state.airborne = None;
        }
        state.last_air_end = Some(end);
        drop(state);
        CHANGED.notify_all();
    }
}
