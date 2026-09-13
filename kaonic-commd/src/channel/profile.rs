//! Profiles: the algorithm a channel runs. A client names one with a
//! `ProfileSpec`; this is the implementation the engine drives.

use kaonic_ctrl::channel::spec::{
    ChannelEvent, CodingSpec, DispatchSpec, Priority, ProfileSpec, QueuePolicy, Runtime, Tdd,
};
use kaonic_fec::{Adaptation, FecCode, FecSelector, FixedCode, PeerKey};
use std::time::Duration;

/// Everything that decides how a channel uses the air. Built once when the
/// channel is opened and shared between the ctrl handler and the engine; the
/// engine asks it before every frame and tells it what happened after.
pub trait Profile: Send + Sync {
    /// Coding policy: which FEC code each frame gets.
    fn adaptation(&self) -> &dyn Adaptation;
    /// Airtime discipline.
    fn tdd(&self) -> Tdd;
    /// Queueing and standing against other channels on the module.
    fn runtime(&self) -> Runtime;
    /// Module choice for channels that span several.
    fn dispatch(&self) -> DispatchSpec;
    /// `true` for a channel with no framing: one payload is one PSDU.
    fn raw(&self) -> bool {
        false
    }
    /// The adaptive selector, for peer feedback from clients; `None` when the
    /// coding is fixed and feedback has nothing to change.
    fn selector(&self) -> Option<&FecSelector<PeerKey>> {
        None
    }
    /// Feedback from the engine; a profile that adapts keeps its own interior
    /// state (the selector does), so this takes `&self`.
    fn on_event(&self, event: &ChannelEvent) {
        let _ = event;
    }
}

/// A profile assembled from the building blocks the daemon knows. Every
/// `ProfileSpec` variant is a parameterisation of this.
pub struct Composed {
    coding: Coding,
    tdd: Tdd,
    runtime: Runtime,
    dispatch: DispatchSpec,
    raw: bool,
}

enum Coding {
    Fixed(FixedCode),
    Adaptive(FecSelector<PeerKey>),
}

impl Profile for Composed {
    fn adaptation(&self) -> &dyn Adaptation {
        match &self.coding {
            Coding::Fixed(fixed) => fixed,
            Coding::Adaptive(selector) => selector,
        }
    }

    fn tdd(&self) -> Tdd {
        self.tdd
    }

    fn runtime(&self) -> Runtime {
        self.runtime
    }

    fn dispatch(&self) -> DispatchSpec {
        self.dispatch
    }

    fn raw(&self) -> bool {
        self.raw
    }

    fn selector(&self) -> Option<&FecSelector<PeerKey>> {
        match &self.coding {
            Coding::Adaptive(selector) => Some(selector),
            Coding::Fixed(_) => None,
        }
    }
}

/// Builds the implementation for a spec. Refuses nothing today; the engine
/// applies the module-level rules (one Raw channel per module, no bundling on
/// the default channel).
pub fn build(spec: &ProfileSpec) -> Box<dyn Profile> {
    let composed = match spec {
        // Exactly today's behaviour on the air.
        ProfileSpec::Robust => Composed {
            coding: Coding::Fixed(FixedCode(FecCode::Tm2048)),
            tdd: Tdd::default(),
            runtime: Runtime::default(),
            dispatch: DispatchSpec::RoundRobin,
            raw: false,
        },
        // Transfers: adapt the code per peer, fill frames, long bursts, and
        // give way to anything interactive.
        ProfileSpec::Bulk { class } => Composed {
            coding: Coding::Adaptive(FecSelector::new(*class)),
            tdd: Tdd {
                burst_frames: 8,
                listen_gap: Duration::from_millis(12),
                bundle_window: Duration::from_millis(5),
                bundle_max: 32,
                slot_offset: None,
            },
            runtime: Runtime {
                queue: QueuePolicy::Block { depth: 64 },
                priority: Priority::Bulk,
                ..Runtime::default()
            },
            dispatch: DispatchSpec::RoundRobin,
            raw: false,
        },
        // Voice: a light fixed code, the newest packet wins, short bursts so
        // the other side's over gets through, and one frame per packet
        // interval so several codec packets share the airtime of one frame.
        ProfileSpec::Voice {
            code,
            packet_interval,
        } => Composed {
            coding: Coding::Fixed(FixedCode(*code)),
            tdd: Tdd {
                burst_frames: 2,
                listen_gap: Duration::from_millis(4),
                // Coalesce codec packets that arrive within a packet interval
                // into one radio frame. Bundling here only helps for a source
                // that emits faster than the channel drains and is not itself
                // paced upstream; a Reticulum link paces its own output, so
                // for that path the effective fix is a larger codec frame.
                bundle_window: *packet_interval,
                bundle_max: 8,
                slot_offset: None,
            },
            runtime: Runtime {
                queue: QueuePolicy::DropOldest { depth: 8 },
                priority: Priority::Realtime,
                reassembly_window: Duration::from_millis(100),
                ..Runtime::default()
            },
            dispatch: DispatchSpec::RoundRobin,
            raw: false,
        },
        ProfileSpec::Raw => Composed {
            coding: Coding::Fixed(FixedCode(FecCode::None)),
            tdd: Tdd {
                bundle_window: Duration::ZERO,
                bundle_max: 1,
                ..Tdd::default()
            },
            runtime: Runtime::default(),
            dispatch: DispatchSpec::RoundRobin,
            raw: true,
        },
        ProfileSpec::Custom {
            coding,
            tdd,
            runtime,
            dispatch,
        } => Composed {
            coding: match coding {
                CodingSpec::Fixed(code) => Coding::Fixed(FixedCode(*code)),
                CodingSpec::Adaptive { default } => Coding::Adaptive(FecSelector::new(*default)),
            },
            tdd: *tdd,
            runtime: *runtime,
            dispatch: *dispatch,
            raw: false,
        },
    };
    Box::new(composed)
}

/// The class a profile starts from, for logging.
pub fn describe(spec: &ProfileSpec) -> String {
    match spec {
        ProfileSpec::Robust => "robust".into(),
        ProfileSpec::Bulk { class } => format!("bulk/{:?}", class),
        ProfileSpec::Voice {
            code,
            packet_interval,
        } => format!("voice/{:?}/{}ms", code, packet_interval.as_millis()),
        ProfileSpec::Raw => "raw".into(),
        ProfileSpec::Custom { coding, .. } => match coding {
            CodingSpec::Fixed(code) => format!("custom/{:?}", code),
            CodingSpec::Adaptive { default } => format!("custom/adaptive/{:?}", default),
        },
    }
}
