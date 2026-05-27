#!/usr/bin/env bash
# Provisions a Proxmox LXC container with a native OSM raster tile server:
# - OSM PBF (local) → planetiler → .mbtiles → tileserver-gl → PNG XYZ tiles
# - uses the FULL tileserver-gl (npm install -g tileserver-gl) with Mesa software rendering for headless GL rasterisation
# - no docker required
# - must be executed as root on the Proxmox host.
#
# usage:
#   bash proxmox_tileserver_lxc_native.sh [OPTIONS]
#
# options (all optional; env vars of the same name are also accepted):
#   --vmid        N          container id (default: 202)
#   --ip          CIDR|dhcp  static ip or dhcp (default: dhcp)
#   --gw          ADDR       gateway; only needed with --ip (default: "")
#   --mem         MB         ram in megabytes (default: 16384)
#   --disk        GB         root disk size in gigabytes (default: 16)
#   --cores       N          vcpu count (default: 4)
#   --bridge      NAME       proxmox bridge (default: vmbr0)
#   --port        N          port tileserver-gl listens on (default: 8080)
#   --storage     NAME       proxmox storage pool (default: local)
#   --pbf-host    PATH       absolute path to pbf on the proxmox host (default: auto-detect from overpass container)
#   --area        NAME       planetiler area shorthand, e.g. germany (default: germany)
#   --java-mem    MB         heap for planetiler in mb (default: 12288)
#   --ctpass      PASS       container root password (default: Car2Xbackend)
#   --style       NAME       style name used in tile urls (default: basic)
#
# tile url after setup:
#   http://<IP>:8080/styles/basic/{z}/{x}/{y}.png


LOG_PREFIX="tileserver-gl"
source "$(dirname "$0")/lib_lxc.sh"

# defaults

VMID="${VMID:-202}"
CT_HOSTNAME="${CT_HOSTNAME:-tileserver}"
MEMORY_MB="${MEMORY_MB:-32768}"
SWAP_MB="${SWAP_MB:-0}"
DISK_GB="${DISK_GB:40}"
CORES="${CORES:-6}"
BRIDGE="${BRIDGE:-vmbr0}"
IP_CIDR="${IP_CIDR:-dhcp}"
GATEWAY="${GATEWAY:-}"
STORAGE="${STORAGE:-local}"
HOST_PORT="${HOST_PORT:-8080}"

PBF_HOST_PATH="${PBF_HOST_PATH:-}"
AREA="${AREA:-germany}"
JAVA_HEAP_MB="${JAVA_HEAP_MB:-28672}"
STYLE_NAME="${STYLE_NAME:-basic}"

INSTALL_DIR="/opt/tileserver"
MBTILES_FILE="${INSTALL_DIR}/data/${AREA}.mbtiles"

PLANETILER_VERSION="0.10.2"
PLANETILER_JAR_URL="https://github.com/onthegomap/planetiler/releases/download/v${PLANETILER_VERSION}/planetiler.jar"

# arg parsing

_handle_extra_arg() {
    case "$1" in
        --pbf-host)  PBF_HOST_PATH="$2"; return 0 ;;
        --area)      AREA="$2";          return 0 ;;
        --java-mem)  JAVA_HEAP_MB="$2";  return 0 ;;
        --style)     STYLE_NAME="$2";    return 0 ;;
        *)           return 1 ;;
    esac
}

parse_args _handle_extra_arg "$@"

# helpers

_find_pbf_on_host() {
    find /var/lib/lxc /rpool /mnt -maxdepth 8 -name "germany*.osm.pbf" \
         -not -path "*/proc/*" 2>/dev/null \
        | head -1 || true
}

# step 1: system dependencies

_deps_installed() {
    [[ "$(ct_sh "dpkg -s openjdk-21-jre-headless nodejs libgles2 &>/dev/null \
        && test -f /usr/local/lib/libvips.so && echo OK" 2>/dev/null)" == "OK" ]]
}

_install_deps() {
    log "installing system deps (java, node.js, mesa gl)..."
    ct apt-get update -qq
    ct apt-get install -y --no-install-recommends \
        curl ca-certificates gnupg unzip wget build-essential\
        openjdk-21-jre-headless \
        libgles2 libgbm1 libegl1 libgl1-mesa-dri \
        libgl1 libglx0 libopengl0 \
        libwebp7 \
        libxi6 libxrandr2 libx11-xcb1 libx11-6 \
        libfontconfig1 libfreetype6 \
        python3 python3-setuptools \
        pkg-config libpng-dev libpng16-16 \
        libglib2.0-dev libexpat1-dev libjpeg-dev libwebp-dev libtiff-dev liborc-0.4-dev \
        meson ninja-build \
        libcairo2-dev libpango1.0-dev libgif-dev librsvg2-dev

    # node.js 22 via nodesource
    push_file /tmp/setup_node.sh <<'NODESCRIPT'
#!/bin/bash
set -euo pipefail
if ! node --version 2>/dev/null | grep -qE '^v22\.'; then
    curl -fsSL https://deb.nodesource.com/setup_22.x | bash -
    apt-get install -y --no-install-recommends nodejs
fi
NODESCRIPT
    ct bash /tmp/setup_node.sh

    # ubuntu 24.04 ships libvips 8.15.2, but sharp requires >= 8.16.0. build 8.17.3 from source
    log "building libvips 8.17.3 from source (ubuntu 24.04 ships 8.15.2)..."
    push_file /tmp/build_libvips.sh <<'VIPSSCRIPT'
#!/bin/bash
set -euo pipefail
VER=8.17.3
wget -q "https://github.com/libvips/libvips/releases/download/v${VER}/vips-${VER}.tar.xz" \
    -O /tmp/vips.tar.xz
tar xf /tmp/vips.tar.xz -C /tmp
cd /tmp/vips-${VER}
meson setup build --prefix=/usr/local --buildtype=release \
    -Dintrospection=disabled
meson compile -C build -j"$(nproc)"
meson install -C build
ldconfig
rm -rf /tmp/vips-${VER} /tmp/vips.tar.xz
VIPSSCRIPT
    ct bash /tmp/build_libvips.sh
    log "done."
}

# step 2: tileserver-gl (full, with gl raster rendering)

_tileserver_installed() {
    [[ "$(ct_sh "command -v tileserver-gl &>/dev/null && echo OK" 2>/dev/null)" == "OK" ]]
}

_install_tileserver() {
    log "installing tileserver-gl globally via npm (includes native gl bindings). this may take 5-10 minutes on first run."
    push_file /tmp/install_tileserver.sh <<'TSSCRIPT'
#!/bin/bash
set -euo pipefail
# increase npm timeout for large native module downloads
npm config set fetch-timeout 300000
# PKG_CONFIG_PATH lets sharp find the system libvips 8.17.3 built in the deps step. sharp then downloads a prebuilt that dynamically links against system libvips/libpng (1.6.43), matching @maplibre/maplibre-gl-native's prebuilt binary expectation
PKG_CONFIG_PATH=/usr/local/lib/pkgconfig npm install -g tileserver-gl 2>&1
TSSCRIPT
    ct bash /tmp/install_tileserver.sh

    # canvas bundles an entire ubuntu 22.04-era library suite in build/Release/ with an RPATH that loads them before system libraries -> this causes two crashes:
    #   1. libpng version mismatch (1.6.37 vs system 1.6.43 headers) → mbgl.node abort
    #   2. missing GLib symbol g_once_init_enter_pointer (added in 2.76) → libvips abort
    # fix: rebuild canvas from source against ubuntu 24.04 system libraries so no bundled .so files are shipped at all
    log "rebuilding canvas from source against system libraries (GLib 2.80, libpng 1.6.43)..."
    push_file /tmp/rebuild_canvas.sh <<'CANVASSCRIPT'
#!/bin/bash
set -euo pipefail
CANVAS_DIR=$(find /usr/lib/node_modules/tileserver-gl/node_modules/canvas \
    -maxdepth 0 -type d 2>/dev/null | head -1)
[[ -n "$CANVAS_DIR" ]] || { echo "canvas module not found, skipping rebuild"; exit 0; }
cd "$CANVAS_DIR"
# Force node-gyp rebuild — uses system cairo/pango/glib/libpng discovered via pkg-config.
npm rebuild canvas --build-from-source 2>&1
echo "canvas rebuild complete."
CANVASSCRIPT
    ct bash /tmp/rebuild_canvas.sh
    log "tileserver-gl installed at /usr/local/bin/tileserver-gl."
}

# step 3: planetiler jar

_planetiler_downloaded() {
    [[ "$(ct_sh "test -f '${INSTALL_DIR}/planetiler.jar' && echo OK" 2>/dev/null)" == "OK" ]]
}

_download_planetiler() {
    log "getting planetiler v${PLANETILER_VERSION}..."
    ct_sh "mkdir -p '${INSTALL_DIR}/data'"
    ct_sh "curl -fsSL '${PLANETILER_JAR_URL}' -o '${INSTALL_DIR}/planetiler.jar'"
    log "jar ready."
}

# step 4: OSM pbf

_pbf_available_in_container() {
    [[ "$(ct_sh "test -f '${INSTALL_DIR}/data/${AREA}.osm.pbf' && echo OK" 2>/dev/null)" == "OK" ]]
}

_provide_pbf() {
    local pbf_path="$PBF_HOST_PATH"
    local dest="${INSTALL_DIR}/data/${AREA}.osm.pbf"

    [[ -n "$pbf_path" ]] || pbf_path=$(_find_pbf_on_host)

    if [[ -n "$pbf_path" && -f "$pbf_path" ]]; then
        log "copying pbf from host: ${pbf_path} (~4 gb)..."
        pct push "$VMID" "$pbf_path" "$dest"
        log "pbf pushed into container."
    else
        log "no host pbf found; downloading germany pbf (~4 gb)..."
        push_file /tmp/dl_pbf.sh <<EOF
#!/bin/bash
set -euo pipefail
wget --progress=dot:giga -O '${dest}' 'https://download.geofabrik.de/europe/germany-latest.osm.pbf'
size=\$(stat -c%s '${dest}')
[[ \$size -gt 500000000 ]] || { echo "download too small (\${size} bytes)"; rm '${dest}'; exit 1; }
EOF
        ct bash /tmp/dl_pbf.sh
        log "pbf done."
    fi
}

# step 5: mbtiles generation

_mbtiles_generated() {
    [[ "$(ct_sh "test -f '${MBTILES_FILE}' && echo OK" 2>/dev/null)" == "OK" ]]
}

_generate_mbtiles() {
    log "generating mbtiles with planetiler (20-60 min for germany)..."
    push_file /tmp/gen_tiles.sh <<EOF
#!/bin/bash
set -euo pipefail
TMP='${INSTALL_DIR}/data/tmp'
mkdir -p "\$TMP"

# pre-download auxiliary datasets with wget (better retry/resume than planetiler's http client). planetiler skips downloads when the file already exists in --download-dir
_wget() {
    local url="\$1" dest="\$TMP/\$(basename "\$1")"
    [[ -s "\$dest" ]] && return 0
    wget --tries=10 --retry-connrefused --waitretry=15 \
         --continue --progress=dot:giga -O "\$dest" "\$url" || {
        rm -f "\$dest"; return 1
    }
}

_wget "https://osmdata.openstreetmap.de/download/water-polygons-split-3857.zip"
_wget "https://dev.maptiler.download/geodata/omt/natural_earth_vector.sqlite.zip"

java -Xmx${JAVA_HEAP_MB}m -XX:+UseParallelGC \
    -jar '${INSTALL_DIR}/planetiler.jar' \
    --osm-path='${INSTALL_DIR}/data/${AREA}.osm.pbf' \
    --output='${MBTILES_FILE}' \
    --download-dir="\$TMP" \
    --download \
    --force
rm -rf "\$TMP"
EOF
    ct bash /tmp/gen_tiles.sh
    log "mbtiles written to ${MBTILES_FILE}."
}

# step 6a: style json (OpenMapTiles schema, no text labels → no fonts needed)

_style_written() {
    [[ "$(ct_sh "test -f '${INSTALL_DIR}/styles/${STYLE_NAME}.json' && echo OK" 2>/dev/null)" == "OK" ]]
}

_write_style() {
    log "writing maplibre gl style (${STYLE_NAME})..."
    ct_sh "mkdir -p '${INSTALL_DIR}/styles' '${INSTALL_DIR}/fonts'"

    # clone repo to temp dir, copy required files, then remove repo
    repo_dir="$(mktemp -d)"
    git clone --depth 1 https://github.com/klokantech/tileserver-gl-styles.git "$repo_dir" >/dev/null 2>&1

    # copy and adapt style: use basic-preview/style.json and set mbtiles source to ${AREA}
    src_style="$repo_dir/styles/basic-preview/style.json"
    dest_style="${INSTALL_DIR}/styles/${STYLE_NAME}.json"
    if [ -f "$src_style" ]; then
        perl -0777 -pe 's/"url"\s*:\s*"mbtiles:\/\/[^"]*"/"url": "mbtiles://'${AREA}'"/g' "$src_style" > "$dest_style"
        log "style written: ${dest_style}"
    else
        log "source style not found: ${src_style}"
    fi

    # copy Noto Sans Regular fonts folder
    src_fonts_dir="$repo_dir/fonts/Noto Sans Regular"
    dest_fonts_dir="${INSTALL_DIR}/fonts/Noto Sans Regular"
    if [ -d "$src_fonts_dir" ]; then
        ct_sh "mkdir -p '${dest_fonts_dir}'"
        ct_sh "cp -a '${src_fonts_dir}/.' '${dest_fonts_dir}/'"
        log "fonts copied: ${dest_fonts_dir}"
    else
        log "source fonts not found: ${src_fonts_dir}"
    fi

    rm -rf "$repo_dir"
    log "temp repo removed"
}


# step 6b: tileserver-gl config

_config_written() {
    [[ "$(ct_sh "test -f '${INSTALL_DIR}/config.json' && echo OK" 2>/dev/null)" == "OK" ]]
}

_write_config() {
    log "writing tileserver-gl config..."

    push_file "${INSTALL_DIR}/config.json" <<EOF
{
  "options": {
    "paths": {
      "styles":  "${INSTALL_DIR}/styles",
      "fonts":   "${INSTALL_DIR}/fonts",
      "mbtiles": "${INSTALL_DIR}/data"
    }
  },
  "styles": {
    "${STYLE_NAME}": {
      "style": "${STYLE_NAME}.json"
    }
  },
  "data": {
    "${AREA}": {
      "mbtiles": "${AREA}.mbtiles"
    }
  }
}
EOF
    ct_sh "mkdir -p '${INSTALL_DIR}/fonts'"  # must exist even when empty (no text labels in style)
    log "done."
}

# step 7: systemd service

_service_running() {
    [[ "$(ct_sh "systemctl is-active tileserver &>/dev/null && echo OK" 2>/dev/null)" == "OK" ]]
}

_setup_service() {
    log "setting up systemd service (port ${HOST_PORT})..."

    local ts_bin
    ts_bin=$(ct_sh "command -v tileserver-gl" 2>/dev/null | tr -d '[:space:]')
    [[ -n "$ts_bin" ]] || die "tileserver-gl binary not found in container PATH"

    push_file /etc/systemd/system/tileserver.service <<EOF
[Unit]
Description=tileserver-gl OSM Raster Tile Server
After=network.target

[Service]
Type=simple
WorkingDirectory=${INSTALL_DIR}
ExecStart=${ts_bin} --config ${INSTALL_DIR}/config.json --port ${HOST_PORT}
# mesa software rendering — no gpu required in lxc container
Environment=LIBGL_ALWAYS_SOFTWARE=1
Environment=GALLIUM_DRIVER=llvmpipe
Environment=NODE_ENV=production
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

    ct systemctl daemon-reload
    ct systemctl enable --now tileserver
    log "service started."
}

# readiness poll

_wait_for_tileserver() {
    local url="http://localhost:${HOST_PORT}/health"
    log "polling tileserver-gl on port ${HOST_PORT} (timeout 120s)..."
    local elapsed=0
    while true; do
        local code
        code=$(ct_sh "curl -s -o /dev/null -w '%{http_code}' '${url}'" 2>/dev/null || echo "000")
        [[ "$code" == "200" ]] && { log "tileserver-gl is ready."; return 0; }
        sleep 5; elapsed=$(( elapsed + 5 ))
        if [[ $elapsed -ge 120 ]]; then
            log "WARNING: tileserver-gl did not respond within 120s."
            log "inspect: pct exec ${VMID} -- journalctl -u tileserver -n 50"
            return 1
        fi
    done
}

# result output

_print_tile_result() {
    local ip
    ip=$(get_container_ip)
    local raster_url="http://${ip}:${HOST_PORT}/styles/${STYLE_NAME}/{z}/{x}/{y}.png"
    local vector_url="http://${ip}:${HOST_PORT}/data/${AREA}/{z}/{x}/{y}.pbf"
    local preview_url="http://${ip}:${HOST_PORT}/"

    echo ""
    echo "====================================================="
    echo "OSM tile server ready"
    echo "  container ip   : ${ip}"
    echo "  preview ui     : ${preview_url}"
    echo ""
    echo "  raster png url : ${raster_url}"
    echo "  vector pbf url : ${vector_url}"
    echo "====================================================="
}

# main

main() {
    require_root

    local tmpl
    tmpl=$(ensure_template)

    ensure_container_running "$tmpl" "nesting=1"

    run_step "system dependencies"  _deps_installed             _install_deps
    run_step "tileserver-gl"        _tileserver_installed       _install_tileserver
    run_step "planetiler JAR"       _planetiler_downloaded      _download_planetiler
    run_step "OSM PBF"              _pbf_available_in_container _provide_pbf
    run_step "MBTiles generation"   _mbtiles_generated          _generate_mbtiles
    run_step "style JSON"           _style_written              _write_style
    run_step "tileserver config"    _config_written             _write_config
    run_step "tileserver service"   _service_running            _setup_service

    _wait_for_tileserver
    _print_tile_result
}

main "$@"
