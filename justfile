name := 'meeting'
appid := 'com.dangrover.next-meeting-app'

rootdir := ''
prefix := env('HOME') + '/.local'

# Installation paths
base-dir := absolute_path(clean(rootdir / prefix))
cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
appdata-dst := base-dir / 'share' / 'appdata' / appid + '.metainfo.xml'
bin-dst := base-dir / 'bin' / name
desktop-dst := base-dir / 'share' / 'applications' / appid + '.desktop'
icon-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'scalable' / 'apps' / appid + '.svg'

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

# Reload just the meeting applet (panel will respawn it with new binary)
reload-applet:
    #!/usr/bin/env bash
    # Kill our applet processes - panel will auto-respawn them
    if pkill -x meeting 2>/dev/null; then
        sleep 0.5
        if pgrep -x meeting >/dev/null; then
            echo "Applet reloaded (PID: $(pgrep -x meeting | head -1))"
        else
            echo "Applet killed, waiting for panel to respawn..."
            sleep 2
            if pgrep -x meeting >/dev/null; then
                echo "Applet respawned (PID: $(pgrep -x meeting | head -1))"
            else
                echo "Warning: Applet not respawned - panel may need restart"
            fi
        fi
    else
        echo "No running applet found"
    fi

# Restart the cosmic panel (use only if reload-applet doesn't work)
restart-panel:
    #!/usr/bin/env bash
    echo "Warning: This may cause duplicate panels. Consider logout/login instead."
    pkill -x cosmic-panel 2>/dev/null || true
    sleep 2
    nohup cosmic-panel >/dev/null 2>&1 &
    sleep 1
    pgrep -x cosmic-panel && echo "Panel restarted" || echo "Panel may not have started"

# Installs files
install:
    install -Dm0755 {{ cargo-target-dir / 'release' / name }} {{bin-dst}}
    install -Dm0644 resources/app.desktop {{desktop-dst}}
    install -Dm0644 resources/app.metainfo.xml {{appdata-dst}}
    install -Dm0644 resources/icon.svg {{icon-dst}}

# Uninstalls installed files
uninstall:
    rm {{bin-dst}} {{desktop-dst}} {{appdata-dst}} {{icon-dst}}

# Vendor dependencies locally
vendor:
    mkdir -p .cargo
    cargo vendor --sync Cargo.toml | head -n -1 > .cargo/config.toml
    echo 'directory = "vendor"' >> .cargo/config.toml
    echo >> .cargo/config.toml
    rm -rf .cargo vendor

# Extracts vendored dependencies
vendor-extract:
    rm -rf vendor
    tar pxf vendor.tar

# Bump cargo version, create git commit, and create tag
tag version:
    find -type f -name Cargo.toml -exec sed -i '0,/^version/s/^version.*/version = "{{version}}"/' '{}' \; -exec git add '{}' \;
    cargo check
    cargo clean
    git add Cargo.lock
    git commit -m 'release: {{version}}'
    git tag -a {{version}} -m ''

