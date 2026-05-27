#!/bin/bash
set -e

# =========================================================================
# NATIVE DEPLOYMENT SCRIPT FOR NODE (RASPBERRY PI)
# =========================================================================
# Compiles the node binary using Docker (locally or on a remote host),
# extracts it, and deploys it natively as a systemd service.
#
# Flags:
#   --force-asn   Delete and regenerate ASN.1 C files before building
# =========================================================================

# build host configuration
USE_REMOTE_DOCKER=true
DOCKER_BUILD_HOST="192.168.1.100"   # ip or hostname of the docker build host
DOCKER_BUILD_USER="root"

FORCE_ASN=false
for arg in "$@"; do
    [ "$arg" = "--force-asn" ] && FORCE_ASN=true
done

PI_USER="user"
PI_HOST="node3.example.com"

# node name derivation
# hostname (e.g. node3.example.com): first label -> "CITES-node3"
# raw ip address -> interactive prompt
if [[ "$PI_HOST" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    read -rp "IP address given — enter node name (e.g. node3): " NODE_LABEL
    [[ -z "$NODE_LABEL" ]] && { echo "Node name must not be empty."; exit 1; }
else
    NODE_LABEL="${PI_HOST%%.*}"   # "node3.example.com" -> "node3"
fi
NODE_NAME="CITES-${NODE_LABEL}"
echo "Node name: $NODE_NAME"

SERVICE_NAME="cites-node"
SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"
PI_BIN_DIR="/usr/local/bin"
PI_CFG_DIR="/etc/cites"

BUILDER_IMG="cites-node-builder"
EXTRACT_CTR="cites-node-extractor"
BIN_OUT="./cites-node-bin"

ASN1_BASE="packages/core/src/asn1_c"
LINUX_TAR="$ASN1_BASE/linux.tar"

# remote docker setup 
DOCKER_ENV=""
if [ "$USE_REMOTE_DOCKER" = true ]; then
    LOCAL_KEY="$HOME/.ssh/id_ed25519"

    if [[ ! -f "$LOCAL_KEY" ]]; then
        echo "No SSH key found – generating $LOCAL_KEY ..."
        ssh-keygen -t ed25519 -N "" -f "$LOCAL_KEY"
    fi

    if ! ssh-keygen -F "$DOCKER_BUILD_HOST" &>/dev/null; then
        echo "Remote host '$DOCKER_BUILD_HOST' is not yet in known_hosts."
        read -rp "Add host key now? [y/N] " REPLY
        [[ "$REPLY" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 1; }
        ssh-keyscan -H "$DOCKER_BUILD_HOST" >> ~/.ssh/known_hosts
        echo "Host key added."
    fi

    if ! ssh -i "$LOCAL_KEY" -o BatchMode=yes -o ConnectTimeout=5 \
            "${DOCKER_BUILD_USER}@${DOCKER_BUILD_HOST}" exit 2>/dev/null; then
        echo "Key auth failed – copying SSH key to remote host..."
        ssh-copy-id -i  "${LOCAL_KEY}.pub" "${DOCKER_BUILD_USER}@${DOCKER_BUILD_HOST}"
        echo "SSH key copied."
    fi

    DOCKER_ENV="DOCKER_HOST=ssh://${DOCKER_BUILD_USER}@${DOCKER_BUILD_HOST}"
    echo "Using remote Docker host: ${DOCKER_BUILD_USER}@${DOCKER_BUILD_HOST}"
else
    echo "Using local Docker."
fi


# asn.1 c-files (linux.tar)
# both local and remote builds use the same linux.tar so asn1c output is
# identical regardless of the remote host's asn1c version, and macOS APFS
# case-collapsing (ActionId.h == ActionID.h) is avoided

if [ "$FORCE_ASN" = true ]; then
    echo "--force-asn: removing existing Linux ASN.1 tar..."
    rm -f "$LINUX_TAR"
fi

if [ -f "$LINUX_TAR" ]; then
    echo "Linux ASN.1 cache found ($LINUX_TAR) – skipping generation."
else
    echo "Generating Linux ASN.1 C files via container (bind-mounted repos)..."
    chmod +x packages/core/resources/asn1/build_asn1_c.sh
    mkdir -p "$ASN1_BASE"
    docker run --rm \
        --platform linux/amd64 \
        -v "$(pwd):/workspace" \
        -w /workspace \
        rust:bookworm \
        bash -c '
            apt-get update -qq
            apt-get install -y -qq autoconf automake libtool bison byacc flex clang libclang-dev git
            export YACC="bison -y"
            export LEX=flex
            bash packages/core/resources/asn1/build_asn1_c.sh

            # asn1c generates Id.h filenames but .c files include ID.h (same
            # pattern as the sed patches in build_asn1_c.sh, applied to filenames).
            for mod in cam_v2 denm_v2 cpm_v2 is_v2; do
                d=packages/core/src/asn1_c/$mod
                [ -f "$d/ActionId.h" ]  && cp "$d/ActionId.h"  "$d/ActionID.h"
                [ -f "$d/StationId.h" ] && cp "$d/StationId.h" "$d/StationID.h"
            done
            d=packages/core/src/asn1_c/is_v2
            [ -f "$d/IntersectionReferenceId.h" ] && cp "$d/IntersectionReferenceId.h" "$d/IntersectionReferenceID.h"
            [ -f "$d/RoadSegmentReferenceId.h" ]  && cp "$d/RoadSegmentReferenceId.h"  "$d/RoadSegmentReferenceID.h"

            tar -cf packages/core/src/asn1_c/linux.tar \
                -C packages/core/src/asn1_c \
                cam_v1 denm_v1 cam_v2 denm_v2 cpm_v2 is_v2
        '
    echo "Linux ASN.1 C files archived to $LINUX_TAR."
fi

# temporary .dockerignore: include linux.tar
cp .dockerignore .dockerignore.bak
printf 'target/\n.git/\npackages/core/resources/asn1/\npackages/core/src/asn1_c/cam_v1/\npackages/core/src/asn1_c/denm_v1/\npackages/core/src/asn1_c/cam_v2/\npackages/core/src/asn1_c/denm_v2/\npackages/core/src/asn1_c/cpm_v2/\npackages/core/src/asn1_c/is_v2/\n' > .dockerignore
trap 'mv .dockerignore.bak .dockerignore' EXIT

# docker build
echo "Building native ARM64 binary using Docker builder..."
build_failed=false
if [ "$USE_REMOTE_DOCKER" = true ]; then
    # cross-compilation on x86: builds natively, no qemu emulation needed
    env $DOCKER_ENV docker buildx build \
        --build-arg  SKIP_ASN1_BUILD=true \
        -t $BUILDER_IMG -f Dockerfile.node.cross . || build_failed=true
else
    # local mac: --platform linux/arm64 runs natively on Apple Silicon 
    docker buildx build --platform linux/arm64 \
        --build-arg SKIP_ASN1_BUILD=true \
        -t $BUILDER_IMG -f Dockerfile.node . || build_failed=true
fi

if [ "$build_failed" = true ]; then
    echo ""
    echo "ERROR: Docker build failed."
    echo ""
    echo "If the error is related to ASN.1 C files (missing headers, compile errors),"
    echo "the cached files may be stale or incorrectly generated. Options:"
    echo ""
    echo "  Force regeneration:  ./deploy_node.sh --force-asn"
    echo "  Manual deletion:     rm -rf $LINUX_TAR"
    echo "                       then re-run ./deploy_node.sh"
    echo ""
    exit 1
fi

# extract binary
echo "Extracting binary..."
env $DOCKER_ENV docker create --name $EXTRACT_CTR $BUILDER_IMG
env $DOCKER_ENV docker cp $EXTRACT_CTR:/node $BIN_OUT
env $DOCKER_ENV docker rm $EXTRACT_CTR

echo "Transferring binary to $PI_USER@$PI_HOST..."
scp "$BIN_OUT" "$PI_USER@$PI_HOST:/tmp/$SERVICE_NAME"

echo "Deploying config for node '$NODE_NAME' from packages/node/config.toml..."
TEMP_CFG=$(mktemp /tmp/cites-node-XXXXXX.toml)
sed "s/^name = .*/name = \"$NODE_NAME\"/" packages/node/config.toml > "$TEMP_CFG"
scp "$TEMP_CFG" "$PI_USER@$PI_HOST:/tmp/config.toml"
rm -f "$TEMP_CFG"

echo "Configuring systemd service natively on target..."
ssh -t $PI_USER@$PI_HOST << SSH_EOF
    set -e

    echo "Stopping existing service..."
    sudo systemctl stop $SERVICE_NAME 2>/dev/null || true

    echo "Installing binary natively..."
    sudo mv /tmp/$SERVICE_NAME $PI_BIN_DIR/$SERVICE_NAME
    sudo chmod +x $PI_BIN_DIR/$SERVICE_NAME

    echo "Configuirng application directory..."
    sudo mkdir -p $PI_CFG_DIR
    sudo mv /tmp/config.toml $PI_CFG_DIR/config.toml

    echo "Creating systemd service file..."
    sudo tee $SERVICE_FILE > /dev/null << SYSTEMD_EOF
[Unit]
Description=CITES Node Backend 
After=network.target bluetooth.target car2x-ocb-setup.service

[Service]
Type=simple
ExecStart=$PI_BIN_DIR/$SERVICE_NAME
WorkingDirectory=$PI_CFG_DIR
Restart=always
RestartSec=5
User=root
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
SYSTEMD_EOF

    echo "Reloading daemon and starting service..."
    sudo systemctl daemon-reload
    sudo systemctl enable $SERVICE_NAME
    #sudo systemctl stop car2x-* 2>/dev/null || true #stage 1 capture services
    sudo systemctl start $SERVICE_NAME
SSH_EOF

echo "Cleaning up local files..."
rm -f  $BIN_OUT

echo "Native deployment complete."
