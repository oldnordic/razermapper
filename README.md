# Razermapper

**EXPERIMENTAL** - A Wayland-compatible input device remapper and macro system for Linux.

> **WARNING**: This software is in early experimental stage. It requires root privileges and directly interacts with low-level input systems. Use at your own risk.

## Status

This is a work-in-progress project. Many features are implemented but not fully tested in production environments.

### What Works

- **Device Discovery**: Scans `/proc/bus/input/devices` to find input devices
- **Device Grabbing**: Exclusive device capture via evdev EVIOCGRAB ioctl
- **GUI Client**: Iced-based interface with:
  - Device listing and grab/release controls
  - Macro name input and recording controls
  - Macro list with play/delete buttons
  - Profile save/load functionality
- **IPC Communication**: Unix socket protocol between GUI and daemon
- **Systemd Integration**: Service file and install script included
- **Pacman Packaging**: PKGBUILD for Arch Linux

### What's Partially Implemented

- **Macro Recording**: Backend structure exists but actual key capture during recording needs testing
- **Macro Execution**: Injector code present but macro playback flow incomplete
- **Input Injection**: uinput virtual device creation code exists but needs real-world testing
- **Profile Management**: Save/load requests handled but file persistence needs verification

### What's NOT Implemented

- Actual key remapping (intercepting key A and outputting key B)
- LED/RGB control (only protocol stubs exist)
- Per-device macro restrictions
- GUI key binding configuration interface
- Trigger condition editing
- Hot-reload of configuration
- Token authentication (code exists but disabled)
- Multi-device macro coordination

## Architecture

```
razermapper/
├── razermapper-common/   # Shared types and IPC protocol
├── razermapperd/         # Root daemon (privileged)
├── razermapper-gui/      # User GUI client (unprivileged)
└── tests/                # Integration tests
```

The daemon runs as root to access `/dev/input/*` and `/dev/uinput`. It creates a Unix socket at `/run/razermapper/razermapper.sock` for GUI communication.

## Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test
```

## Installation (Arch Linux)

```bash
# Build package
./build-package.sh

# Install
sudo pacman -U razermapper-0.2.0-1-x86_64.pkg.tar.zst

# Enable and start daemon
sudo systemctl enable --now razermapperd

# Run GUI
razermapper-gui
```

## Manual Usage

```bash
# Start daemon (requires root)
sudo razermapperd

# In another terminal, run GUI
razermapper-gui
```

## Security Considerations

- Daemon MUST run as root for input device access
- Creates uinput virtual devices
- Socket permissions set to 0666 (world-readable/writable)
- No authentication in current version
- Direct hardware access - potential for system instability

## Known Issues

- Macro recording may not capture all key events properly
- No validation of macro trigger conflicts
- GUI doesn't show actual key codes, only counts
- Profile format not documented
- Error handling is basic
- No graceful shutdown on some error conditions

## Dependencies

- Rust 1.70+
- Linux kernel with evdev and uinput support
- iced 0.12 (GUI framework)
- tokio (async runtime)
- serde + bincode (serialization)

## License

This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](LICENSE) file for details.

## Contributing

This is an experimental project. If you want to contribute:

1. Understand that core functionality is incomplete
2. Test thoroughly on non-critical systems
3. Be prepared for breaking changes
4. Focus on the actually-working features first

## Acknowledgments

- Built for Razer devices but should work with any evdev input device
- Inspired by projects like input-remapper and keyd
- Uses the iced GUI framework

---

**Remember**: This is experimental software that requires root access and modifies system input handling. Do not use on production systems.
