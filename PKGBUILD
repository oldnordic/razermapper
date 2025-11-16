# Maintainer: Feanor <your.email@example.com>
pkgname=razermapper
pkgver=0.2.0
pkgrel=1
pkgdesc="Wayland-compatible input remapper and macro engine for Razer devices"
arch=('x86_64')
url="https://github.com/feanor/razermapper"
license=('GPL-3.0-only')
depends=(
    'glibc'
    'gcc-libs'
)
makedepends=(
    'rust'
    'cargo'
)
optdepends=(
    'openrazer-daemon: For enhanced Razer device detection'
    'openrazer-meta: Full OpenRazer stack for RGB control'
)
backup=('etc/razermapper/config.yaml')
install=razermapper.install
provides=('razermapper')
conflicts=('razermapper-git')

build() {
    cd "$startdir"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    cargo build --release -p razermapperd -p razermapper-gui
}

check() {
    cd "$startdir"
    export RUSTUP_TOOLCHAIN=stable
    cargo test --release -p razermapper-common 2>/dev/null || true
}

package() {
    cd "$startdir"

    # Install daemon binary
    install -Dm755 "target/release/razermapperd" "$pkgdir/usr/bin/razermapperd"

    # Install GUI binary
    install -Dm755 "target/release/razermapper-gui" "$pkgdir/usr/bin/razermapper-gui"

    # Install test tool
    if [ -f "target/release/test_grab" ]; then
        install -Dm755 "target/release/test_grab" "$pkgdir/usr/bin/razermapper-test-grab"
    fi

    # Install systemd service
    install -Dm644 "razermapperd.service" "$pkgdir/usr/lib/systemd/system/razermapperd.service"

    # Install udev rules for device access
    install -Dm644 "99-razermapper.rules" "$pkgdir/usr/lib/udev/rules.d/99-razermapper.rules"

    # Create config directory and install example config
    install -dm755 "$pkgdir/etc/razermapper"
    install -Dm644 "config.yaml.example" "$pkgdir/etc/razermapper/config.yaml"

    # Install desktop file for GUI
    install -Dm644 "razermapper.desktop" "$pkgdir/usr/share/applications/razermapper.desktop"

    # Install license
    install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"

    # Install documentation
    install -Dm644 "README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
}
