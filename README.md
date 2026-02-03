# Kaonic Radio

A modular RF communication system for embedded radio operations with dual-band support (sub-GHz and 2.4 GHz).

## Overview

Kaonic Radio is a complete radio communication stack built around the AT86RF215 dual-band transceiver chip. The system provides a gRPC-based service architecture for radio control, network protocols with forward error correction, adaptive QoS mechanisms, and performance testing tools.

**Key Features:**
- Dual-band operation: sub-GHz (389.5 - 1020 MHz) and 2.4 GHz (2400 - 2483.5 MHz)
- Multiple modulation schemes: OFDM, QPSK, FSK
- Adaptive QoS with channel assessment and power control
- LDPC forward error correction
- gRPC-based control interface
- Desktop GUI for monitoring and configuration
- Network performance testing tools

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                    Application Layer                         │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐      │
│  │  kaonic-gui  │   │ kaonic-iperf │   │ Your App     │      │
│  └──────┬───────┘   └──────┬───────┘   └──────┬───────┘      │
│         │                  │                  │              │
│         └──────────────────┴──────────────────┘              │
│                            │ gRPC (port 8080)                │
└────────────────────────────┼─────────────────────────────────┘
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                    Service Layer                            │
│  ┌────────────────────────────────────────────────────────┐ │
│  │              kaonic-commd (gRPC Server)                │ │
│  │    Main communication daemon & radio orchestration     │ │
│  └─────────────────────────┬──────────────────────────────┘ │
└────────────────────────────┼────────────────────────────────┘
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                    Protocol Stack                           │
│  ┌──────────────┐   ┌──────────────┐  ┌──────────────┐      │
│  │ kaonic-net   │   │ kaonic-qos   │  │ kaonic-fpga  │      │
│  │ (LDPC FEC,   │   │ (CCA, Adapt. │  │ (FPGA I/O)   │      │
│  │  Framing)    │   │  Modulation) │  │              │      │
│  └──────┬───────┘   └──────┬───────┘  └──────┬───────┘      │
│         └──────────────────┴─────────────────┘              │
└────────────────────────────┼────────────────────────────────┘
                             ▼
┌──────────────────────────────────────────────────────────────┐
│                    Hardware Layer                            │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              kaonic-radio (Platform HAL)                │ │
│  │         Linux GPIO + SPI | Dummy Implementation         │ │
│  └─────────────────────────┬───────────────────────────────┘ │
│                            ▼                                 │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │           radio-rf215 (AT86RF215 Driver)                │ │
│  │              SPI-based Transceiver Control              │ │
│  └─────────────────────────┬───────────────────────────────┘ │
└────────────────────────────┼─────────────────────────────────┘
                             ▼
                         Hardware
```

## Components

### Core Libraries

#### **radio-rf215**
Low-level driver for the AT86RF215 dual-band transceiver chip.
- Direct SPI register control
- Modulation configuration (OFDM/QPSK/FSK)
- Dual radio module support (Module A & B)
- RSSI and energy detection

#### **kaonic-radio**
Platform abstraction layer for radio hardware.
- **Linux:** Uses `libgpiod` for GPIO control and `linux-embedded-hal` for SPI
- **Other platforms:** Dummy implementation for development
- Frame transmission and reception
- Platform-specific variants: `kaonic1s`, standard Linux, RF215

#### **kaonic-net**
Network layer with error correction and packet handling.
- LDPC forward error correction (Labrador codec)
- Packet encoding/decoding with CRC validation
- Frame multiplexing and demultiplexing
- Maximum payload: 2047 bytes

#### **kaonic-qos**
Quality-of-Service and adaptive transmission control.
- Clear Channel Assessment (CCA) based on energy detection
- Adaptive modulation selection
- Adaptive transmit power control
- Interference detection via EDV (Energy Detection Values)

#### **kaonic-fpga**
FPGA register abstraction using memory-mapped I/O.
- Register read/write via `memmap2`
- Hardware integration for FPGA-based platforms

### Services & Daemons

#### **kaonic-commd**
Main communication daemon providing gRPC control interface.
- Listens on port 8080 (default: `http://127.0.0.1:8080`)
- Protobuf-based API for radio configuration
- Multi-threaded async runtime (Tokio)
- Services: Device info, radio configuration, transmit/receive, network operations

**gRPC Services:**
- `Device`: System information and statistics
- `Radio`: Radio module configuration and frame operations
- `Network`: Network-layer transmit/receive with FEC

#### **kaonic-factory**
Factory testing and provisioning service.
- gRPC interface for manufacturing tests
- Device provisioning workflows
- Hardware validation

### Applications

#### **kaonic-gui**
Desktop GUI application for radio monitoring and control.
- Built with ImGui + OpenGL (glow backend)
- Real-time RSSI visualization and waterfall display
- Radio configuration interface
- OTA firmware update support
- iPerf integration for performance testing

**Platform Support:** Windows, Linux, macOS

#### **kaonic-iperf**
Network performance measurement tool (similar to iperf).
- Client/Server mode for RTT and throughput testing
- Configurable via TOML config file
- Supports both radio modules
- CRC32 packet validation
- Command-line interface

#### **kaonic-test**
Test utilities and validation tools for the radio stack.

## Modulation Support

### OFDM (Orthogonal Frequency-Division Multiplexing)
- **MCS Levels:** 0-6 (configurable coding/modulation schemes)
- **Options:** 0-3 (bandwidth and interleaving configurations)
- **Use Case:** High data rate, robust against multipath fading
- **Bands:** Both sub-GHz and 2.4 GHz

### QPSK (Quadrature Phase-Shift Keying)
- **Chip Frequencies:** Configurable spreading
- **Rate Modes:** Multiple data rates
- **Use Case:** Moderate data rate with good range
- **Bands:** Both sub-GHz and 2.4 GHz

### FSK (Frequency-Shift Keying)
- **Configurable Parameters:** Symbol rate, modulation index, preamble length
- **FEC Support:** Convolutional coding available
- **Use Case:** Long-range, low data rate applications
- **Bands:** Primarily sub-GHz

## Frequency Bands

### Sub-GHz Band
- **Range:** 389.5 - 1020 MHz (389500 - 1020000 kHz)
- **Channel Spacing:** 25-400 kHz (configurable)
- **Typical Use:** Long-range IoT, sub-GHz ISM bands (433 MHz, 868 MHz, 915 MHz)

### 2.4 GHz Band
- **Range:** 2400 - 2483.5 MHz (2400000 - 2483500 kHz)
- **Channel Spacing:** 200-2000 kHz (configurable)
- **Typical Use:** WiFi-adjacent ISM band, higher throughput

## Supported Platforms

### Production Platforms
- **Linux** (primary target)
  - Requires: libgpiod, Linux kernel with SPI support
  - Tested on: ARM (32-bit, 64-bit), x86_64
  - Hardware: Custom boards with AT86RF215 via SPI

### Development Platforms
- **macOS** (aarch64-apple-darwin, x86_64-apple-darwin)
  - Dummy radio implementation for application development
  - GUI and testing tools fully functional
  
- **Windows**
  - Dummy radio implementation
  - GUI support via OpenGL

## License

See [LICENSE](LICENSE) file for details.

## Repository

https://github.com/BeechatNetworkSystemsLtd/kaonic-radio

---

**Note:** This system is designed for embedded radio communication applications. Ensure compliance with local RF regulations when operating radio hardware.
