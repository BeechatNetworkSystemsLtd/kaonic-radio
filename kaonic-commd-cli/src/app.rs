use std::collections::VecDeque;

pub const MAX_RX_LOG: usize = 200;
pub const TX_POWER_MIN: u32 = 0;
pub const TX_POWER_MAX: u32 = 30;

// ─── Modulation ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModType {
    Off,
    Ofdm,
    Qpsk,
    Fsk,
}

impl ModType {
    pub const ALL: &'static [ModType] = &[ModType::Off, ModType::Ofdm, ModType::Qpsk, ModType::Fsk];

    pub fn label(&self) -> &'static str {
        match self {
            ModType::Off => "OFF",
            ModType::Ofdm => "OFDM",
            ModType::Qpsk => "QPSK",
            ModType::Fsk => "FSK",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfdmMcs {
    BpskC12_4x = 0,
    BpskC12_2x = 1,
    QpskC12_2x = 2,
    QpskC12 = 3,
    QpskC34 = 4,
    QamC12 = 5,
    QamC34 = 6,
}

impl OfdmMcs {
    pub const ALL: &'static [OfdmMcs] = &[
        OfdmMcs::BpskC12_4x,
        OfdmMcs::BpskC12_2x,
        OfdmMcs::QpskC12_2x,
        OfdmMcs::QpskC12,
        OfdmMcs::QpskC34,
        OfdmMcs::QamC12,
        OfdmMcs::QamC34,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            OfdmMcs::BpskC12_4x => "BPSK 1/2 4x",
            OfdmMcs::BpskC12_2x => "BPSK 1/2 2x",
            OfdmMcs::QpskC12_2x => "QPSK 1/2 2x",
            OfdmMcs::QpskC12 => "QPSK 1/2",
            OfdmMcs::QpskC34 => "QPSK 3/4",
            OfdmMcs::QamC12 => "16-QAM 1/2",
            OfdmMcs::QamC34 => "16-QAM 3/4",
        }
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn index(&self) -> usize {
        *self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfdmOpt {
    Opt1 = 0,
    Opt2 = 1,
    Opt3 = 2,
    Opt4 = 3,
}

impl OfdmOpt {
    pub const ALL: &'static [OfdmOpt] =
        &[OfdmOpt::Opt1, OfdmOpt::Opt2, OfdmOpt::Opt3, OfdmOpt::Opt4];

    pub fn label(&self) -> &'static str {
        match self {
            OfdmOpt::Opt1 => "Option 1",
            OfdmOpt::Opt2 => "Option 2",
            OfdmOpt::Opt3 => "Option 3",
            OfdmOpt::Opt4 => "Option 4",
        }
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn index(&self) -> usize {
        *self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QpskFchip {
    F100 = 0,
    F200 = 1,
    F1000 = 2,
    F2000 = 3,
}

impl QpskFchip {
    pub const ALL: &'static [QpskFchip] =
        &[QpskFchip::F100, QpskFchip::F200, QpskFchip::F1000, QpskFchip::F2000];

    pub fn label(&self) -> &'static str {
        match self {
            QpskFchip::F100 => "100 kHz",
            QpskFchip::F200 => "200 kHz",
            QpskFchip::F1000 => "1 MHz",
            QpskFchip::F2000 => "2 MHz",
        }
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn index(&self) -> usize {
        *self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QpskMode {
    Mode0 = 0,
    Mode1 = 1,
    Mode2 = 2,
    Mode3 = 3,
    Mode4 = 4,
}

impl QpskMode {
    pub const ALL: &'static [QpskMode] = &[
        QpskMode::Mode0,
        QpskMode::Mode1,
        QpskMode::Mode2,
        QpskMode::Mode3,
        QpskMode::Mode4,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            QpskMode::Mode0 => "Mode 0",
            QpskMode::Mode1 => "Mode 1",
            QpskMode::Mode2 => "Mode 2",
            QpskMode::Mode3 => "Mode 3",
            QpskMode::Mode4 => "Mode 4",
        }
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn index(&self) -> usize {
        *self as usize
    }
}

// ─── Field item (field or section header) ───────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldItem {
    Section(&'static str),
    Field(Field),
}

// ─── Config field cursor ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    ServerAddr,
    Module,
    FreqMhz,
    Channel,
    ChannelSpacingKhz,
    BwFilter,
    ModType,
    // OFDM-specific
    OfdmMcs,
    OfdmOpt,
    // QPSK-specific
    QpskFchip,
    QpskMode,
    // Shared
    TxPower,
}

impl Field {
    pub fn label(&self) -> &'static str {
        match self {
            Field::ServerAddr => "Server Address",
            Field::Module => "Module",
            Field::FreqMhz => "Freq (MHz)",
            Field::Channel => "Channel",
            Field::ChannelSpacingKhz => "Ch Spacing (kHz)",
            Field::BwFilter => "BW Filter",
            Field::ModType => "Modulation",
            Field::OfdmMcs => "OFDM MCS",
            Field::OfdmOpt => "OFDM Opt",
            Field::QpskFchip => "QPSK Fchip",
            Field::QpskMode => "QPSK Mode",
            Field::TxPower => "TX Power (dBm)",
        }
    }
}

// ─── Statistics ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default)]
pub struct ModuleStatsSnapshot {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes:   u64,
    pub tx_bytes:   u64,
    pub rx_errors:  u64,
    pub tx_errors:  u64,
}

// ─── RX log entry ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RxEntry {
    pub module: u8,
    pub len: usize,
    pub rssi: i32,
    pub preview: String, // hex preview of first bytes
}

// ─── Application state ──────────────────────────────────────────────────────

pub struct App {
    // Connection
    pub server_addr: String,
    pub connected: bool,
    pub status_msg: String,

    // How many modules the server reported
    pub module_count: usize,
    pub serial: String,
    pub mtu: u32,
    pub version: String,

    // Per-module statistics (refreshed ~1 s)
    pub stats: Vec<ModuleStatsSnapshot>,

    // Radio config fields
    pub module: usize,          // 0 = A, 1 = B
    pub freq_mhz: String,       // editable string
    pub channel: String,
    pub channel_spacing_khz: String,
    pub bw_wide: bool,
    pub mod_type: ModType,
    pub ofdm_mcs: OfdmMcs,
    pub ofdm_opt: OfdmOpt,
    pub qpsk_fchip: QpskFchip,
    pub qpsk_mode: QpskMode,
    pub tx_power: u32,

    // Navigation
    pub focused_field: Field,
    pub editing: bool,          // text-input mode for numeric fields

    // RX log
    pub rx_log: VecDeque<RxEntry>,
    pub rx_log_scroll: usize,

    // Tx counter
    pub tx_count: u64,

    // Compose window (Some = open, None = closed)
    pub compose_text: Option<String>,

    pub should_quit: bool,
}

impl App {
    pub fn new(server_addr: String) -> Self {
        Self {
            server_addr,
            connected: false,
            status_msg: String::from("Connecting…"),
            module_count: 0,
            serial: String::new(),
            mtu: 0,
            version: String::new(),
            stats: Vec::new(),
            module: 0,
            freq_mhz: String::from("869.535"),
            channel: String::from("10"),
            channel_spacing_khz: String::from("200"),
            bw_wide: false,
            mod_type: ModType::Ofdm,
            ofdm_mcs: OfdmMcs::QamC34,
            ofdm_opt: OfdmOpt::Opt1,
            qpsk_fchip: QpskFchip::F100,
            qpsk_mode: QpskMode::Mode0,
            tx_power: 14,
            focused_field: Field::Module,
            editing: false,
            rx_log: VecDeque::new(),
            rx_log_scroll: 0,
            tx_count: 0,
            compose_text: None,
            should_quit: false,
        }
    }

    // ── Visible fields for current modulation ─────────────────────────────

    pub fn visible_fields(&self) -> Vec<Field> {
        let mut fields = vec![
            Field::ServerAddr,
            Field::Module,
            Field::FreqMhz,
            Field::Channel,
            Field::ChannelSpacingKhz,
            Field::BwFilter,
            Field::ModType,
        ];
        match self.mod_type {
            ModType::Ofdm => {
                fields.push(Field::OfdmMcs);
                fields.push(Field::OfdmOpt);
            }
            ModType::Qpsk => {
                fields.push(Field::QpskFchip);
                fields.push(Field::QpskMode);
            }
            _ => {}
        }
        if self.mod_type != ModType::Off && self.mod_type != ModType::Fsk {
            fields.push(Field::TxPower);
        }
        fields
    }

    /// Visible items for rendering — includes section headers.
    pub fn visible_items(&self) -> Vec<FieldItem> {
        let mut items = vec![
            FieldItem::Section("Server"),
            FieldItem::Field(Field::ServerAddr),
            FieldItem::Section("Radio"),
            FieldItem::Field(Field::Module),
            FieldItem::Field(Field::FreqMhz),
            FieldItem::Field(Field::Channel),
            FieldItem::Field(Field::ChannelSpacingKhz),
            FieldItem::Field(Field::BwFilter),
            FieldItem::Section("Modulation"),
            FieldItem::Field(Field::ModType),
        ];
        match self.mod_type {
            ModType::Ofdm => {
                items.push(FieldItem::Field(Field::OfdmMcs));
                items.push(FieldItem::Field(Field::OfdmOpt));
            }
            ModType::Qpsk => {
                items.push(FieldItem::Field(Field::QpskFchip));
                items.push(FieldItem::Field(Field::QpskMode));
            }
            _ => {}
        }
        if self.mod_type != ModType::Off && self.mod_type != ModType::Fsk {
            items.push(FieldItem::Field(Field::TxPower));
        }
        items
    }

    pub fn focused_index(&self) -> usize {
        let fields = self.visible_fields();
        fields
            .iter()
            .position(|f| *f == self.focused_field)
            .unwrap_or(0)
    }

    // ── Field value as display string ─────────────────────────────────────

    pub fn field_value(&self, field: &Field) -> String {
        match field {
            Field::ServerAddr => self.server_addr.clone(),
            Field::Module => {
                if self.module == 0 { "MODULE A".into() } else { "MODULE B".into() }
            }
            Field::FreqMhz => self.freq_mhz.clone(),
            Field::Channel => self.channel.clone(),
            Field::ChannelSpacingKhz => self.channel_spacing_khz.clone(),
            Field::BwFilter => {
                if self.bw_wide { "Wide".into() } else { "Narrow".into() }
            }
            Field::ModType => self.mod_type.label().into(),
            Field::OfdmMcs => self.ofdm_mcs.label().into(),
            Field::OfdmOpt => self.ofdm_opt.label().into(),
            Field::QpskFchip => self.qpsk_fchip.label().into(),
            Field::QpskMode => self.qpsk_mode.label().into(),
            Field::TxPower => format!("{} dBm", self.tx_power),
        }
    }

    // ── Navigation ────────────────────────────────────────────────────────

    pub fn next_field(&mut self) {
        let fields = self.visible_fields();
        let idx = self.focused_index();
        let next = (idx + 1) % fields.len();
        self.focused_field = fields[next];
    }

    pub fn prev_field(&mut self) {
        let fields = self.visible_fields();
        let idx = self.focused_index();
        let prev = if idx == 0 { fields.len() - 1 } else { idx - 1 };
        self.focused_field = fields[prev];
    }

    // ── Cycle-style fields (enum pickers) ─────────────────────────────────

    pub fn cycle_up(&mut self) {
        match self.focused_field {
            Field::Module => {
                if self.module_count > 0 {
                    self.module = (self.module + 1) % self.module_count.max(1);
                }
            }
            Field::BwFilter => self.bw_wide = !self.bw_wide,
            Field::ModType => {
                let idx = ModType::ALL.iter().position(|m| *m == self.mod_type).unwrap_or(0);
                self.mod_type = ModType::ALL[(idx + 1) % ModType::ALL.len()];
                self.clamp_focused_field();
            }
            Field::OfdmMcs => {
                let idx = (self.ofdm_mcs.index() + 1) % OfdmMcs::ALL.len();
                self.ofdm_mcs = OfdmMcs::from_index(idx);
            }
            Field::OfdmOpt => {
                let idx = (self.ofdm_opt.index() + 1) % OfdmOpt::ALL.len();
                self.ofdm_opt = OfdmOpt::from_index(idx);
            }
            Field::QpskFchip => {
                let idx = (self.qpsk_fchip.index() + 1) % QpskFchip::ALL.len();
                self.qpsk_fchip = QpskFchip::from_index(idx);
            }
            Field::QpskMode => {
                let idx = (self.qpsk_mode.index() + 1) % QpskMode::ALL.len();
                self.qpsk_mode = QpskMode::from_index(idx);
            }
            Field::TxPower => {
                if self.tx_power < TX_POWER_MAX { self.tx_power += 1; }
            }
            _ => {}
        }
    }

    pub fn cycle_down(&mut self) {
        match self.focused_field {
            Field::Module => {
                if self.module_count > 0 {
                    let count = self.module_count.max(1);
                    self.module = if self.module == 0 { count - 1 } else { self.module - 1 };
                }
            }
            Field::BwFilter => self.bw_wide = !self.bw_wide,
            Field::ModType => {
                let idx = ModType::ALL.iter().position(|m| *m == self.mod_type).unwrap_or(0);
                let len = ModType::ALL.len();
                self.mod_type = ModType::ALL[(idx + len - 1) % len];
                self.clamp_focused_field();
            }
            Field::OfdmMcs => {
                let len = OfdmMcs::ALL.len();
                let idx = (self.ofdm_mcs.index() + len - 1) % len;
                self.ofdm_mcs = OfdmMcs::from_index(idx);
            }
            Field::OfdmOpt => {
                let len = OfdmOpt::ALL.len();
                let idx = (self.ofdm_opt.index() + len - 1) % len;
                self.ofdm_opt = OfdmOpt::from_index(idx);
            }
            Field::QpskFchip => {
                let len = QpskFchip::ALL.len();
                let idx = (self.qpsk_fchip.index() + len - 1) % len;
                self.qpsk_fchip = QpskFchip::from_index(idx);
            }
            Field::QpskMode => {
                let len = QpskMode::ALL.len();
                let idx = (self.qpsk_mode.index() + len - 1) % len;
                self.qpsk_mode = QpskMode::from_index(idx);
            }
            Field::TxPower => {
                if self.tx_power > TX_POWER_MIN { self.tx_power -= 1; }
            }
            _ => {}
        }
    }

    fn clamp_focused_field(&mut self) {
        let fields = self.visible_fields();
        if !fields.contains(&self.focused_field) {
            self.focused_field = fields[0];
        }
    }

    // ── Text editing ──────────────────────────────────────────────────────

    pub fn edit_push(&mut self, c: char) {
        let s = self.focused_string_mut();
        if let Some(s) = s {
            s.push(c);
        }
    }

    pub fn edit_backspace(&mut self) {
        let s = self.focused_string_mut();
        if let Some(s) = s {
            s.pop();
        }
    }

    fn focused_string_mut(&mut self) -> Option<&mut String> {
        match self.focused_field {
            Field::ServerAddr => Some(&mut self.server_addr),
            Field::FreqMhz => Some(&mut self.freq_mhz),
            Field::Channel => Some(&mut self.channel),
            Field::ChannelSpacingKhz => Some(&mut self.channel_spacing_khz),
            _ => None,
        }
    }

    pub fn is_text_field(&self) -> bool {
        matches!(
            self.focused_field,
            Field::ServerAddr | Field::FreqMhz | Field::Channel | Field::ChannelSpacingKhz
        )
    }

    /// True when the focused text field accepts only numeric input (digits, `.`, `-`).
    pub fn is_numeric_field(&self) -> bool {
        matches!(
            self.focused_field,
            Field::FreqMhz | Field::Channel | Field::ChannelSpacingKhz
        )
    }

    // ── RX log ────────────────────────────────────────────────────────────

    pub fn push_rx(&mut self, entry: RxEntry) {
        if self.rx_log.len() >= MAX_RX_LOG {
            self.rx_log.pop_front();
        }
        self.rx_log.push_back(entry);
        // Auto-scroll to bottom
        self.rx_log_scroll = self.rx_log.len().saturating_sub(1);
    }

    pub fn scroll_rx_up(&mut self) {
        self.rx_log_scroll = self.rx_log_scroll.saturating_sub(1);
    }

    pub fn scroll_rx_down(&mut self) {
        if !self.rx_log.is_empty() {
            self.rx_log_scroll =
                (self.rx_log_scroll + 1).min(self.rx_log.len().saturating_sub(1));
        }
    }
}
