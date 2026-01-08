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

# Reload applet by restarting panel
reload-applet:
    killall cosmic-panel || true; cosmic-panel &

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
tag version:
    find -type f -name Cargo.toml -exec sed -i '0,/^version/s/^version.*/version = "{{version}}"/' '{}' \; -exec git add '{}' \;
    sed -i 's/^cosmic-next-meeting ([^)]*)/cosmic-next-meeting ({{version}}-1)/' debian/changelog
    git add debian/changelog
    cargo check
    cargo clean
    git add Cargo.lock
    git commit -m 'release: {{version}}'
    git tag -a v{{version}} -m 'Release {{version}}'

# Create and push a release tag (triggers GitHub Actions release build)
release version: (tag version)
    @echo "Release v{{version}} created. Push with:"
    @echo "  git push origin main --tags"

# Build Debian package
build-deb:
    dpkg-buildpackage -us -uc -b

# Generate Flatpak cargo sources and build
flatpak-cargo-sources:
    #!/usr/bin/env bash
    if ! command -v flatpak-cargo-generator.py &> /dev/null; then
        echo "Installing flatpak-cargo-generator..."
        pip install aiohttp toml
        curl -O https://raw.githubusercontent.com/nicokoch/flatpak-cargo-generator/master/flatpak-cargo-generator.py
        chmod +x flatpak-cargo-generator.py
    fi
    python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json

# Build Flatpak (requires flatpak-builder)
build-flatpak: flatpak-cargo-sources
    flatpak-builder --force-clean --user --install-deps-from=flathub --repo=repo builddir {{appid}}.json

# Install Flatpak locally for testing
install-flatpak: build-flatpak
    flatpak --user remote-add --no-gpg-verify --if-not-exists local-repo repo
    flatpak --user install -y local-repo {{appid}}

