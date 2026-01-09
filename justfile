name := 'cosmic-next-meeting'
appid := 'com.dangrover.next-meeting-app'

rootdir := ''
prefix := env('HOME') + '/.local'

# Installation paths
base-dir := absolute_path(clean(rootdir / prefix))
cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
metainfo-dst := base-dir / 'share' / 'metainfo' / appid + '.metainfo.xml'
bin-dst := base-dir / 'bin' / name
desktop-dst := base-dir / 'share' / 'applications' / appid + '.desktop'
icon-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'scalable' / 'apps' / appid + '.svg'
icon-symbolic-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'symbolic' / 'apps' / appid + '-symbolic.svg'

# Default recipe which runs `just build-release`
default: build-release

# Runs `cargo clean`
clean:
    cargo clean

# Removes vendored dependencies
clean-vendor:
    rm -rf .cargo vendor vendor.tar

# `cargo clean` and removes vendored dependencies
clean-dist: clean clean-vendor

# Compiles with debug profile
build-debug *args:
    cargo build {{args}}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Compiles release profile with vendored dependencies
build-vendored *args: vendor-extract (build-release '--frozen --offline' args)

# Runs a clippy check
check *args:
    cargo clippy --all-features {{args}} -- -W clippy::pedantic

# Runs a clippy check with JSON message format
check-json: (check '--message-format=json')

# Run the application for testing purposes
run *args:
    env RUST_BACKTRACE=full cargo run --release {{args}}

# Build, install, and reload applet for quick dev iteration
dev: build-release install reload-applet

# Reload applet by restarting panel (system auto-restarts it)
reload-applet:
    #!/usr/bin/env bash
    echo "Restarting panel..."
    pkill -TERM cosmic-panel 2>/dev/null || true
    # Wait for panel to restart (cosmic-session should restart it)
    for i in {1..10}; do
        sleep 0.5
        if pgrep -x cosmic-panel >/dev/null 2>&1; then
            echo "Panel restarted successfully."
            exit 0
        fi
    done
    echo "Panel did not restart within 5 seconds."
    echo "Try: Log out and back in, or run 'cosmic-panel &' manually."

# Installs files
install:
    install -Dm0755 {{ cargo-target-dir / 'release' / name }} {{bin-dst}}
    install -Dm0644 resources/app.desktop {{desktop-dst}}
    install -Dm0644 resources/app.metainfo.xml {{metainfo-dst}}
    install -Dm0644 resources/icon.svg {{icon-dst}}
    install -Dm0644 resources/icon-symbolic.svg {{icon-symbolic-dst}}

# Uninstalls installed files
uninstall:
    rm {{bin-dst}} {{desktop-dst}} {{metainfo-dst}} {{icon-dst}} {{icon-symbolic-dst}}

# Vendor dependencies locally
vendor:
    mkdir -p .cargo
    cargo vendor --sync Cargo.toml | head -n -1 > .cargo/config.toml
    echo 'directory = "vendor"' >> .cargo/config.toml
    echo >> .cargo/config.toml
    tar pcf vendor.tar vendor
    rm -rf vendor

# Extracts vendored dependencies
vendor-extract:
    rm -rf vendor
    tar pxf vendor.tar

# Bump cargo version, create git commit, and create tag
tag version: update-flatpak-sources
    find -type f -name Cargo.toml -exec sed -i '0,/^version/s/^version.*/version = "{{version}}"/' '{}' \; -exec git add '{}' \;
    sed -i 's/^cosmic-next-meeting ([^)]*)/cosmic-next-meeting ({{version}}-1)/' debian/changelog
    git add debian/changelog
    sed -i '/<releases>/a\    <release version="{{version}}" date="'"$(date +%Y-%m-%d)"'">\n      <description>\n        <p>TODO: Add release notes<\/p>\n      <\/description>\n    <\/release>' resources/app.metainfo.xml
    git add resources/app.metainfo.xml
    cargo check
    cargo clean
    git add Cargo.lock cargo-sources.json
    git commit -m 'release: {{version}}'
    git tag -a v{{version}} -m 'Release {{version}}'

# Create and push a release tag (triggers GitHub Actions release build)
release version: (tag version)
    @echo "Release v{{version}} created. Push with:"
    @echo "  git push origin main --tags"

# Install build dependencies for packaging
install-build-deps:
    #!/usr/bin/env bash
    echo "Installing build dependencies..."
    sudo apt-get update
    sudo apt-get install -y debhelper devscripts flatpak-builder
    flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
    pip install aiohttp tomlkit

# Build Debian package (requires: sudo apt install debhelper devscripts)
build-deb:
    dpkg-buildpackage -us -uc -b

# Regenerate Flatpak cargo sources (run after updating Cargo.lock)
update-flatpak-sources:
    #!/usr/bin/env bash
    if [ ! -f flatpak-cargo-generator.py ]; then
        echo "Downloading flatpak-cargo-generator..."
        pip install aiohttp tomlkit
        curl -L -o flatpak-cargo-generator.py https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
        chmod +x flatpak-cargo-generator.py
    fi
    python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json
    echo "cargo-sources.json updated. Don't forget to commit it!"

# Build Flatpak (requires flatpak-builder)
build-flatpak:
    flatpak-builder --force-clean --user --install-deps-from=flathub --repo=repo builddir {{appid}}.json

# Install Flatpak locally for testing
install-flatpak: build-flatpak
    flatpak --user remote-add --no-gpg-verify --if-not-exists local-repo repo
    flatpak --user install -y local-repo {{appid}}

