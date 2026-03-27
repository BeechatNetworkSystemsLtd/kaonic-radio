use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap,
    },
};

use crate::app::{App, Field, FieldItem, TX_POWER_MAX, TX_POWER_MIN};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const OK: Color = Color::Green;
const ERR: Color = Color::Red;
const EDIT_FG: Color = Color::Yellow;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Root layout: header / body / footer
    let root = Layout::vertical([
        Constraint::Length(4), // header (2 content lines + 2 borders)
        Constraint::Min(0),    // body
        Constraint::Length(3), // footer
    ])
    .split(area);

    draw_header(frame, app, root[0]);
    draw_body(frame, app, root[1]);
    draw_footer(frame, app, root[2]);

    // Compose popup drawn last so it appears on top
    if app.compose_text.is_some() {
        draw_compose_popup(frame, app, area);
    }
}

// ─── Header ─────────────────────────────────────────────────────────────────

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let (status_text, status_style) = if app.connected {
        (
            format!("● Connected  {}", app.server_addr),
            Style::default().fg(OK).bold(),
        )
    } else {
        (
            format!("○ {}", app.status_msg),
            Style::default().fg(ERR).bold(),
        )
    };

    let serial_str = if app.serial.is_empty() { "–".to_string() } else { app.serial.clone() };
    let version_str = if app.version.is_empty() { "–".to_string() } else { app.version.clone() };
    let mtu_str = if app.mtu == 0 { "–".to_string() } else { app.mtu.to_string() };

    let title = Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled("kaonic-commd-cli  ", Style::default().fg(ACCENT).bold()),
            Span::styled(status_text, status_style),
        ]),
        Line::from(vec![
            Span::styled("SN: ", Style::default().fg(DIM)),
            Span::styled(serial_str, Style::default().fg(Color::White)),
            Span::styled("   v", Style::default().fg(DIM)),
            Span::styled(version_str, Style::default().fg(Color::White)),
            Span::styled("   MTU: ", Style::default().fg(DIM)),
            Span::styled(mtu_str, Style::default().fg(Color::White)),
        ]),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DIM)),
    );

    frame.render_widget(title, area);
}

// ─── Body: left config panel + right RX log ─────────────────────────────────

fn draw_body(frame: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::horizontal([
        Constraint::Percentage(45),
        Constraint::Percentage(55),
    ])
    .split(area);

    draw_config_panel(frame, app, cols[0]);

    // Right column: RX log on top, stats at bottom
    let stats_height = 2 + app.module_count.max(1) as u16 + 2; // border + header line + N module rows + border
    let right_rows = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(stats_height),
    ])
    .split(cols[1]);

    draw_rx_panel(frame, app, right_rows[0]);
    draw_stats_panel(frame, app, right_rows[1]);
}

// ─── Config panel ────────────────────────────────────────────────────────────

fn draw_config_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" ⚙  Radio Configuration ")
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let focused_idx = app.focused_index();
    let fields = app.visible_fields();

    let items: Vec<ListItem> = app.visible_items()
        .into_iter()
        .map(|item| match item {
            // ── Section header ──
            FieldItem::Section(title) => {
                ListItem::new(Line::from(vec![
                    Span::styled("── ", Style::default().fg(DIM)),
                    Span::styled(title, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled(" ──", Style::default().fg(DIM)),
                ]))
            }

            // ── Regular field ──
            FieldItem::Field(field) => {
                let field_pos = fields.iter().position(|f| *f == field).unwrap_or(0);
                let focused = field_pos == focused_idx;

                let label_style = if focused {
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };

                let arrow = if focused { "▶ " } else { "  " };

                if field == Field::TxPower {
                    // ── Slider ──
                    let bar_width: u32 = 20;
                    let range = TX_POWER_MAX - TX_POWER_MIN;
                    let filled = ((app.tx_power - TX_POWER_MIN) * bar_width / range.max(1)) as usize;
                    let empty  = (bar_width as usize).saturating_sub(filled);
                    let bar = format!("{}{}",
                        "█".repeat(filled),
                        "░".repeat(empty),
                    );
                    let bar_style = if focused {
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(DIM)
                    };
                    let val_style = if focused {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(DIM)
                    };
                    ListItem::new(vec![
                        Line::from(vec![
                            Span::styled(arrow, label_style),
                            Span::styled(field.label(), label_style),
                        ]),
                        Line::from(vec![
                            Span::raw("    "),
                            Span::styled("◄ ", if focused { Style::default().fg(ACCENT) } else { Style::default().fg(DIM) }),
                            Span::styled(bar, bar_style),
                            Span::styled(" ►  ", if focused { Style::default().fg(ACCENT) } else { Style::default().fg(DIM) }),
                            Span::styled(format!("{} dBm", app.tx_power), val_style),
                        ]),
                    ])
                } else {
                    // ── Standard field ──
                    let value = app.field_value(&field);
                    let value_style = if focused && app.editing && app.is_text_field() {
                        Style::default().fg(EDIT_FG).add_modifier(Modifier::BOLD)
                    } else if focused {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(DIM)
                    };
                    let cursor = if focused && app.editing && app.is_text_field() { "▌" } else { "" };

                    ListItem::new(vec![
                        Line::from(vec![
                            Span::styled(arrow, label_style),
                            Span::styled(field.label(), label_style),
                        ]),
                        Line::from(vec![
                            Span::raw("    "),
                            Span::styled(format!("{}{}", value, cursor), value_style),
                        ]),
                    ])
                }
            }
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

// ─── RX log panel ────────────────────────────────────────────────────────────

fn draw_rx_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(format!(" 📡  Received Frames ({}) ", app.rx_log.len()))
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.rx_log.is_empty() {
        let placeholder = Paragraph::new("  No frames received yet…")
            .style(Style::default().fg(DIM))
            .wrap(Wrap { trim: false });
        frame.render_widget(placeholder, inner);
        return;
    }

    let height = inner.height as usize;
    let log_len = app.rx_log.len();

    // Window of entries that fit in the panel
    let start = app.rx_log_scroll.saturating_sub(height.saturating_sub(1)).min(log_len.saturating_sub(height));
    let visible: Vec<&crate::app::RxEntry> = app.rx_log.iter().skip(start).take(height).collect();

    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let abs_idx = start + i + 1;
            let mod_label = if entry.module == 0 { "A" } else { "B" };
            let rssi_style = if entry.rssi < -90 {
                Style::default().fg(ERR)
            } else if entry.rssi < -70 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(OK)
            };

            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:>4} ", abs_idx),
                    Style::default().fg(DIM),
                ),
                Span::styled(
                    format!("[{}] ", mod_label),
                    Style::default().fg(ACCENT),
                ),
                Span::styled(
                    format!("{:>4}B ", entry.len),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{:>4}dBm ", entry.rssi),
                    rssi_style,
                ),
                Span::styled(
                    entry.preview.as_str(),
                    Style::default().fg(DIM),
                ),
            ]))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

// ─── Statistics panel ────────────────────────────────────────────────────────

fn draw_stats_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" 📊  Statistics ")
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Header row
    let header = Line::from(vec![
        Span::styled(format!("{:<4}", "Mod"), Style::default().fg(DIM).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {:>8}", "RX pkts"), Style::default().fg(DIM)),
        Span::styled(format!(" {:>8}", "TX pkts"), Style::default().fg(DIM)),
        Span::styled(format!(" {:>9}", "RX bytes"), Style::default().fg(DIM)),
        Span::styled(format!(" {:>9}", "TX bytes"), Style::default().fg(DIM)),
        Span::styled(format!(" {:>6}", "RX err"), Style::default().fg(DIM)),
        Span::styled(format!(" {:>6}", "TX err"), Style::default().fg(DIM)),
    ]);

    let mut lines = vec![header];

    for (i, snap) in app.stats.iter().enumerate() {
        let mod_label = if i == 0 { "A" } else { "B" };
        let err_style = if snap.rx_errors > 0 || snap.tx_errors > 0 {
            Style::default().fg(ERR)
        } else {
            Style::default().fg(OK)
        };

        lines.push(Line::from(vec![
            Span::styled(format!("[{:<2}]", mod_label), Style::default().fg(ACCENT)),
            Span::styled(format!(" {:>8}", snap.rx_packets), Style::default().fg(Color::White)),
            Span::styled(format!(" {:>8}", snap.tx_packets), Style::default().fg(Color::White)),
            Span::styled(format!(" {:>9}", fmt_bytes(snap.rx_bytes)), Style::default().fg(Color::White)),
            Span::styled(format!(" {:>9}", fmt_bytes(snap.tx_bytes)), Style::default().fg(Color::White)),
            Span::styled(format!(" {:>6}", snap.rx_errors), err_style),
            Span::styled(format!(" {:>6}", snap.tx_errors), err_style),
        ]));
    }

    if app.stats.is_empty() {
        lines.push(Line::from(Span::styled("  No data yet…", Style::default().fg(DIM))));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn fmt_bytes(b: u64) -> String {
    if b >= 1_000_000 {
        format!("{:.1}MB", b as f64 / 1_000_000.0)
    } else if b >= 1_000 {
        format!("{:.1}KB", b as f64 / 1_000.0)
    } else {
        format!("{}B", b)
    }
}

// ─── Compose popup ───────────────────────────────────────────────────────────

fn draw_compose_popup(frame: &mut Frame, app: &App, area: Rect) {
    let popup_w = area.width.min(70).max(40);
    let popup_h = 5u16;
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup_area = Rect { x, y, width: popup_w, height: popup_h };

    let mod_label = if app.module == 0 { "A" } else { "B" };
    let title = format!(" ✉  Transmit on Module {} ", mod_label);

    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(ACCENT));

    let text = app.compose_text.as_deref().unwrap_or("");

    // Inner area split: hint line + input line
    let inner = block.inner(popup_area);
    let rows = Layout::vertical([
        Constraint::Length(1), // hint
        Constraint::Length(1), // input
    ])
    .split(inner);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(block, popup_area);

    // Hint
    let hint = Paragraph::new(Line::from(vec![
        Span::styled(" Enter", Style::default().fg(Color::Black).bg(ACCENT).bold()),
        Span::styled(" send  ", Style::default().fg(Color::Gray)),
        Span::styled(" Esc", Style::default().fg(Color::Black).bg(ACCENT).bold()),
        Span::styled(" cancel", Style::default().fg(Color::Gray)),
    ]));
    frame.render_widget(hint, rows[0]);

    // Input line
    let max_visible = (inner.width as usize).saturating_sub(3);
    let display_text = if text.len() > max_visible {
        &text[text.len() - max_visible..]
    } else {
        text
    };
    let input = Paragraph::new(Line::from(vec![
        Span::styled("> ", Style::default().fg(ACCENT).bold()),
        Span::styled(display_text, Style::default().fg(Color::White)),
        Span::styled("▌", Style::default().fg(ACCENT)),
    ]));
    frame.render_widget(input, rows[1]);
}

// ─── Footer / key hints ──────────────────────────────────────────────────────

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let mut hints: Vec<(&str, &str)> = vec![
        (" ↑↓ ", "navigate"),
        (" ←→ ", "cycle"),
        (" Enter ", "edit"),
        (" c ", "configure"),
        (" t ", "transmit"),
        (" q ", "quit"),
    ];

    if app.compose_text.is_some() {
        hints = vec![
            (" Enter ", "send"),
            (" Esc ", "cancel"),
        ];
    } else if app.editing {
        if app.focused_field == Field::ServerAddr {
            hints = vec![
                (" Enter ", "reconnect"),
                (" Esc ", "cancel"),
            ];
        } else {
            hints = vec![
                (" Enter ", "confirm"),
                (" Esc ", "cancel"),
                (" 0-9 . ", "input"),
            ];
        }
    }

    let spans: Vec<Span> = hints
        .iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(*key, Style::default().fg(Color::Black).bg(ACCENT).bold()),
                Span::styled(format!(" {} ", desc), Style::default().fg(Color::Gray)),
            ]
        })
        .collect();

    let status_right = if !app.status_msg.is_empty() {
        Span::styled(
            format!(" {} ", app.status_msg),
            Style::default().fg(if app.status_msg.starts_with("Configure OK") || app.status_msg.contains("TX") {
                OK
            } else {
                ERR
            }),
        )
    } else {
        Span::raw("")
    };

    let left_line = Line::from(spans);
    let right_line = Line::from(vec![status_right]);

    // Split footer into left (hints) and right (status)
    let cols = Layout::horizontal([Constraint::Min(0), Constraint::Length(40)]).split(area);

    let left = Paragraph::new(left_line).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DIM)),
    );
    let right = Paragraph::new(right_line)
        .alignment(Alignment::Right)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(DIM)),
        );

    frame.render_widget(left, cols[0]);
    frame.render_widget(right, cols[1]);
}
