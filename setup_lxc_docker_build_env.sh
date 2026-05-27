#!/bin/bash
# =============================================================================
# PROXMOX LXC SETUP – CITES Docker Build Environment
# =============================================================================
# Run this script ON THE PROXMOX HOST (PVE shell).
#
# Creates an unprivileged Debian 12 LXC container with Docker CE installed, configured as a remote Docker build host for CITES ARM64 cross-compilation.
#
# Usage: bash setup_lxc.sh
#
# After setup, build from your machine with: DOCKER_BUILD_HOST=ssh://user@host ./deploy_node.sh
# =============================================================================

set -e

# configuration (edit as needed)
CT_ID="${CT_ID:-200}"
CT_HOSTNAME="${CT_HOSTNAME:-cites-builder}"
CT_PASSWORD="${CT_PASSWORD:-}"          # leave empty to be prompted
CT_MEMORY="${CT_MEMORY:-8192}"          # MB
CT_SWAP="${CT_SWAP:-0}"                 # MB
CT_DISK="${CT_DISK:-80}"                # GB  (Rust builds are large)
CT_CORES="${CT_CORES:-6}"
CT_BRIDGE="${CT_BRIDGE:-vmbr0}"
CT_IP="${CT_IP:-dhcp}"                  # e.g. "192.168.1.50/24" for static
CT_GW="${CT_GW:-}"                      # gateway, only needed for static IP
STORAGE="${STORAGE:-local}"             # proxmox storage target
TEMPLATE_STORAGE="${TEMPLATE_STORAGE:-local}"

SSH_PUB_KEY="${SSH_PUB_KEY:-$HOME/.ssh/id_rsa.pub}"  # injected into container

# helpers
info()  { echo -e "[INFO] $*"; }
warn()  { echo -e "[WARN] $*"; }
error() { echo -e "[ERROR] $*"; exit 1; }

[[ $EUID -ne 0 ]] && error "Run as root on the Proxmox host."
command -v pct  &>/dev/null || error "'pct' not found – is this a Proxmox host?"
command -v pvesh &>/dev/null || error "'pvesh' not found – is this a Proxmox host?"

if pct status "$CT_ID" &>/dev/null; then
    error "Container $CT_ID already exists. Choose a different CT_ID."
fi

if [[ -z "$CT_PASSWORD" ]]; then
    read -rsp "Root password for the container: " CT_PASSWORD; echo
    [[ -z "$CT_PASSWORD" ]] && error "Password cannot be empty."
fi

# download template if missing
info "Updating template list..."
pveam update

TEMPLATE=$(pveam available --section system 2>/dev/null \
    | awk '{print $2}' \
    | grep '^debian-12-standard' \
    | sort -V | tail -1)

[[ -z "$TEMPLATE" ]] && error "No debian-12-standard template found. Run 'pveam available --section system' to inspect."
info "Using template: $TEMPLATE"

TEMPLATE_PATH="/var/lib/vz/template/cache/$TEMPLATE"
if [[ ! -f "$TEMPLATE_PATH" ]]; then
    info "Downloading $TEMPLATE..."
    pveam download "$TEMPLATE_STORAGE" "$TEMPLATE" || \
        error "Template download failed."
fi

# build network config string
if [[ "$CT_IP" == "dhcp" ]]; then
    NET_CONFIG="name=eth0,bridge=${CT_BRIDGE},ip=dhcp"
else
    [[ -z "$CT_GW" ]] && error "CT_GW must be set when using a static IP."
    NET_CONFIG="name=eth0,bridge=${CT_BRIDGE},ip=${CT_IP},gw=${CT_GW}"
fi

# create container
info "Creating LXC container $CT_ID ($CT_HOSTNAME)..."

pct create "$CT_ID" "${TEMPLATE_STORAGE}:vztmpl/${TEMPLATE}" \
    --hostname      "$CT_HOSTNAME" \
    --password      "$CT_PASSWORD" \
    --memory        "$CT_MEMORY" \
    --swap          "$CT_SWAP" \
    --cores         "$CT_CORES" \
    --rootfs        "${STORAGE}:${CT_DISK}" \
    --net0          "$NET_CONFIG" \
    --ostype        debian \
    --unprivileged  1 \
    --features      "keyctl=1,nesting=1" \
    --onboot        1
    
# additional lxc config for docker (apparmor + cgroup passthrough)
info "Applying Docker-specific LXC configuration..."
LXC_CONF="/etc/pve/lxc/${CT_ID}.conf"
cat >> "$LXC_CONF" <<'EOF'
lxc.apparmor.profile: unconfined
lxc.cgroup2.devices.allow: a
lxc.cap.drop:
EOF

info "Starting container..."
pct start "$CT_ID"
sleep 5 

# allow root ssh login with password
info "Configuring SSH: PermitRootLogin yes..."
pct exec "$CT_ID" -- bash -c '
    sed -i "s/^#\?PermitRootLogin.*/PermitRootLogin yes/" /etc/ssh/sshd_config
    grep -q "^PermitRootLogin" /etc/ssh/sshd_config || echo "PermitRootLogin yes" >> /etc/ssh/sshd_config
    systemctl restart ssh
'

# inject ssh public key
if [[ -f "$SSH_PUB_KEY" ]]; then
    info "Injecting SSH public key from $SSH_PUB_KEY..."
    pct exec "$CT_ID" -- bash -c 'mkdir -p /root/.ssh && chmod 700 /root/.ssh'
    pct push "$CT_ID" "$SSH_PUB_KEY" /root/.ssh/authorized_keys --perms 600
else
    warn "SSH public key not found at $SSH_PUB_KEY – skipping. Add it manually later."
fi

info "Installing Docker CE inside the container..."

pct exec "$CT_ID" -- bash -c '
set -e

apt-get update -qq
apt-get install -y -qq \
    ca-certificates curl gnupg lsb-release

# Add Docker GPG key and repository
install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/debian/gpg \
    -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc

echo \
  "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] \
  https://download.docker.com/linux/debian \
  $(. /etc/os-release && echo "$VERSION_CODENAME") stable" \
  > /etc/apt/sources.list.d/docker.list

apt-get update -qq
apt-get install -y -qq \
    docker-ce docker-ce-cli containerd.io \
    docker-buildx-plugin docker-compose-plugin

systemctl enable docker
systemctl start docker

# Verify
docker --version
docker buildx version
echo "Docker installed successfully."
'

# install qemu for arm64 cross-compilation
info "Installing QEMU binfmt (ARM64 cross-build support)..."
pct exec "$CT_ID" -- bash -c '
    apt-get install -y -qq qemu-user-static binfmt-support
    update-binfmts --enable
    echo "QEMU binfmt installed and enabled."
'

CT_ACTUAL_IP=$(pct exec "$CT_ID" -- bash -c "hostname -I | awk '{print \$1}'" 2>/dev/null || echo "<check with: pct exec $CT_ID -- hostname -I>")

echo ""
echo -e "================================="
echo -e "  LXC container $CT_ID ready."
echo -e "  IP:  $CT_ACTUAL_IP"
echo -e "================================="
echo ""
echo "  Test SSH access: ssh root@${CT_ACTUAL_IP}"
echo ""
echo "  Build CITES node binary remotely: DOCKER_BUILD_HOST=ssh://root@${CT_ACTUAL_IP} ./deploy_node.sh"
echo ""
