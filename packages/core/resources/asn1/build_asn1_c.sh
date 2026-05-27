#!/bin/bash
set -e

echo "Ensuring a UPER-capable asn1c compiler is available (mouse07410/asn1c fork)..."

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
ASN_DIR="$SCRIPT_DIR"
OUT_DIR="$CORE_DIR/src/asn1_c"

export GIT_TERMINAL_PROMPT=0

mkdir -p "$ASN_DIR"
mkdir -p "$OUT_DIR"

# EN - European Norm, i.e. release 1 standards with original asn.1 structures and cdd approach
REPOS_V1=(
    "https://forge.etsi.org/rep/ITS/asn1/cam_en302637_2.git"    #CAM - Cooperative Awareness Basic Service messages
    "https://forge.etsi.org/rep/ITS/asn1/denm_en302637_3.git"   #DENM - Decentralized Environmental Notification Basic Service messages
    "https://forge.etsi.org/rep/ITS/asn1/poti_en302890_2.git"   #POTI
    "https://forge.etsi.org/rep/ITS/asn1/saem_en302890_1.git"   #SAEM
)

# TS - Technical Specification, i.e. release 2 standards with updated asn.1 structures and new cdd approach
REPOS_V2=(
    "https://forge.etsi.org/rep/ITS/asn1/avp_ts103882.git"      #AVM - Automated Vehicle Marshalling
    "https://forge.etsi.org/rep/ITS/asn1/cam_ts103900.git"      #CAM - Cooperative Awareness Basic Service messages
    "https://forge.etsi.org/rep/ITS/asn1/cpm_ts103324.git"      #CPM - Collective Perception Basic Service messages
    "https://forge.etsi.org/rep/ITS/asn1/cp_ts104072.git"       #CP - Collective Perception messages
    "https://forge.etsi.org/rep/ITS/asn1/denm_ts103831.git"     #DENM - Decentralized Environmental Notification Basic Service messages
    "https://forge.etsi.org/rep/ITS/asn1/evcsn-ts101556_1.git"  #EVCSN - Electric Vehicle Charging Spot Notification Specification
    "https://forge.etsi.org/rep/ITS/asn1/evrsr_ts101556_3.git"  #EVRSR - Electric Vehicle Recharging Spot Reservation messages
    "https://forge.etsi.org/rep/ITS/asn1/gn_ts103836_4_1.git"   #GN - GeoNetworking Media-Independent protocol
    "https://forge.etsi.org/rep/ITS/asn1/imzm_ts103724.git"     #IMZM - Interference Management Zone Service
    "https://forge.etsi.org/rep/ITS/asn1/is_ts103301.git"       #IS - Multiple Protocols
    "https://forge.etsi.org/rep/ITS/asn1/mrs_ts103759.git"      #MRS - Misbehaviour Reporting Service
    "https://forge.etsi.org/rep/ITS/asn1/pa_ts103916.git"       #PA - Parking Availability Service submodule of ITS POI
    "https://forge.etsi.org/rep/ITS/asn1/pki_ts102941.git"      #PKI - Public Key Infrastructure
    "https://forge.etsi.org/rep/ITS/asn1/poim_ts103916.git"     #POIM
    "https://forge.etsi.org/rep/ITS/asn1/rmo_ts103745.git"      #RMO - Urban Rail ITS/Road ITS shared use of spectrum
    "https://forge.etsi.org/rep/ITS/asn1/saem_ts104091.git"     #SAEM - Services Announcement specification
    "https://forge.etsi.org/rep/ITS/asn1/sdp_ts103601.git"      #SDP - security management messages communication requirements and distribution protocols
    "https://forge.etsi.org/rep/ITS/asn1/sec_ts103097.git"      #SEC - Security
    "https://forge.etsi.org/rep/ITS/asn1/tistpg_ts101556_2.git" #TISTPG - Tyre Information System (TIS) and Tyre Pressure Gauge (TPG) messages
    "https://forge.etsi.org/rep/ITS/asn1/vam-ts103300_3.git"    #VAM - VRU Awareness Basic Service
)
ALL_REPOS=("${REPOS_V1[@]}" "${REPOS_V2[@]}")

# fetch repos
for REPO_URL in "${ALL_REPOS[@]}"; do
    REPO_NAME="$(basename "$REPO_URL" .git)"
    TARGET_DIR="$ASN_DIR/$REPO_NAME"
    
    if [ -d "$TARGET_DIR/.git" ]; then
        echo "Updating $REPO_NAME..."
        git -C "$TARGET_DIR" fetch --all 2>/dev/null || true
        DEFAULT_BRANCH=$(git -C "$TARGET_DIR" symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "master")
        git -C "$TARGET_DIR" reset --hard "origin/$DEFAULT_BRANCH" 2>/dev/null || echo "Update failed, continuing."
        
        # remove stale git locks to prevent submodule sync/update failures
        find "$TARGET_DIR/.git" -name "*.lock" -type f -delete 2>/dev/null || true
        
        # init submodules, then update them to avoid a single failing submodule
        git -C "$TARGET_DIR" submodule init || true
        git -C "$TARGET_DIR" submodule sync || true
        git -C "$TARGET_DIR" submodule foreach 'git fetch --all 2>/dev/null || true' || true
        git -C "$TARGET_DIR" submodule update --force || true
    else
        echo "Cloning $REPO_NAME..."
        git clone "$REPO_URL" "$TARGET_DIR" || echo "Clone failed for $REPO_NAME"
        DEFAULT_BRANCH=$(git -C "$TARGET_DIR" symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "master")
        git -C "$TARGET_DIR" reset --hard "origin/$DEFAULT_BRANCH" 2>/dev/null || true
        
        # remove stale git locks to prevent submodule sync/update failures
        find "$TARGET_DIR/.git" -name "*.lock" -type f -delete 2>/dev/null || true
        
        git -C "$TARGET_DIR" submodule init || true
        git -C "$TARGET_DIR" submodule update --force || true
    fi
done

# fetch missing iso dependencies
echo "Fetching additional ISO dependencies..."
ISO_DIR="$ASN_DIR/iso19321"
mkdir -p "$ISO_DIR"
curl -sS "https://standards.iso.org/iso/ts/19321/ed-3/en/ISO19321IVI-IS.asn" -o "$ISO_DIR/ISO19321IVI-IS.asn"
curl -sS "https://standards.iso.org/iso/ts/19321/ed-3/en/ISO19321IVIv3.1.asn" -o "$ISO_DIR/ISO19321IVIv3.1.asn"
curl -sS "https://standards.iso.org/iso/14823/-1/ed-1/en/ISO_14823-1%20ed1_AnnexE.asn" -o "$ISO_DIR/ISO14823-1_ed1_AnnexE.asn"
curl -sS "https://standards.iso.org/iso/17573/-3/ed-1/en/ISO17573-3(2023)EfcDataDictionaryV1.3.asn" -o "$ISO_DIR/EfcDataDictionaryV1.3.asn"

# clone specialized cdd repository for iso 19321 bindings
if [ ! -d "$ISO_DIR/cdd_ts102894_2" ]; then
    git clone "https://forge.etsi.org/rep/ITS/asn1/cdd_ts102894_2.git" "$ISO_DIR/cdd_ts102894_2" || echo "Failed to clone cdd_ts102894_2.git"
else
    git -C "$ISO_DIR/cdd_ts102894_2" fetch origin || true
    DEFAULT_CDD_BRANCH=$(git -C "$ISO_DIR/cdd_ts102894_2" symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "main")
    git -C "$ISO_DIR/cdd_ts102894_2" reset --hard "origin/$DEFAULT_CDD_BRANCH" || true
fi

TMP_COMP="$(mktemp -d)"
# ensure temp files are deleted
trap 'rm -rf "$TMP_COMP"' EXIT

# add homebrew bins to path for macOS compatibility with older bison
export PATH="/opt/homebrew/opt/bison/bin:/opt/homebrew/opt/flex/bin:$PATH"
if ! command -v asn1c &> /dev/null || ! asn1c -v 2>&1 | grep -q "mouse07410"; then
    echo "asn1c not found or not the correct VLM/mouse07410 fork. Building locally in $TMP_COMP..."
    
    cd "$TMP_COMP"
    git clone https://github.com/mouse07410/asn1c.git .
    test -f configure || autoreconf -iv
    ./configure
    make -j$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)
    ASN1C_EXEC="$TMP_COMP/asn1c/asn1c"
    ASN1C_SHARE="$TMP_COMP/skeletons"
else
    # system asn1c is assumed to be correct (unlikely due to v0.9.28 on homebrew)
    ASN1C_EXEC="asn1c"
    echo "Using system asn1c (warning: check if it handles ETSI schemas)."
fi

mkdir -p "$OUT_DIR"
cd "$OUT_DIR"

echo "Compiling ASN.1 schemas per module..."

compile_module() {
    local mod_name=$1
    shift
    local target_dir="$OUT_DIR/$mod_name"
    mkdir -p "$target_dir"
    echo " -> Compiling $mod_name"
    
    cd "$target_dir"
    "$ASN1C_EXEC" -gen-UPER -fcompound-names -fwide-types -no-gen-example -pdu=all "$@"
}

# V1 / release 1
compile_module "cam_v1" \
    "$ASN_DIR/cam_en302637_2/cdd/ITS-Container.asn" \
    "$ASN_DIR/cam_en302637_2/CAM-PDU-Descriptions.asn"

compile_module "denm_v1" \
    "$ASN_DIR/denm_en302637_3/cdd/ITS-Container.asn" \
    "$ASN_DIR/denm_en302637_3/DENM-PDU-Descriptions.asn"

# V2 / release 2
compile_module "cam_v2" \
    "$ASN_DIR/iso19321/cdd_ts102894_2/ETSI-ITS-CDD.asn" \
    "$ASN_DIR/cam_ts103900/CAM-PDU-Descriptions.asn"

compile_module "denm_v2" \
    "$ASN_DIR/iso19321/cdd_ts102894_2/ETSI-ITS-CDD.asn" \
    "$ASN_DIR/denm_ts103831/DENM-PDU-Descriptions.asn"

compile_module "cpm_v2" \
    "$ASN_DIR/cpm_ts103324/asn/cdd/ETSI-ITS-CDD.asn" \
    "$ASN_DIR/cpm_ts103324/asn/CPM-PDU-Descriptions.asn" \
    "$ASN_DIR/cpm_ts103324/asn/CPM-OriginatingStationContainers.asn" \
    "$ASN_DIR/cpm_ts103324/asn/CPM-PerceivedObjectContainer.asn" \
    "$ASN_DIR/cpm_ts103324/asn/CPM-PerceptionRegionContainer.asn" \
    "$ASN_DIR/cpm_ts103324/asn/CPM-SensorInformationContainer.asn"

compile_module "is_v2" \
    "$ASN_DIR/iso19321/cdd_ts102894_2/ETSI-ITS-CDD.asn" \
    "$ASN_DIR/iso19321/EfcDataDictionaryV1.3.asn" \
    "$ASN_DIR/iso19321/ISO14823-1_ed1_AnnexE.asn" \
    "$ASN_DIR/iso19321/ISO19321IVIv3.1.asn" \
    "$ASN_DIR/iso19321/ISO19321IVI-IS.asn" \
    "$ASN_DIR/is_ts103301/DSRC.asn" \
    "$ASN_DIR/is_ts103301/DSRC-addgrp-C.asn" \
    "$ASN_DIR/is_ts103301/DSRC-region.asn" \
    "$ASN_DIR/is_ts103301/IVIM-PDU-Descriptions.asn" \
    "$ASN_DIR/is_ts103301/MAPEM-PDU-Descriptions.asn" \
    "$ASN_DIR/is_ts103301/RTCMEM-PDU-Descriptions.asn" \
    "$ASN_DIR/is_ts103301/SPATEM-PDU-Descriptions.asn" \
    "$ASN_DIR/is_ts103301/SREM-PDU-Descriptions.asn" \
    "$ASN_DIR/is_ts103301/SSEM-PDU-Descriptions.asn"

cd "$OUT_DIR"

if [ -d "$TMP_COMP" ]; then
    rm -rf "$TMP_COMP"
fi

echo "Patching generated C files to resolve C macro conflicts..."
if [ "$(uname)" = "Darwin" ]; then
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i '' -e 's/NULL_t[[:space:]][[:space:]]*NULL;/NULL_t null_value;/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i '' -e 's/choice\.NULL/choice.null_value/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i '' -e 's/choice, NULL/choice, null_value/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i '' -e 's/StationId_t/StationID_t/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i '' -e 's/StationId_t/StationID_t/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i '' -e 's/asn_DEF_StationId/asn_DEF_StationID/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i '' -e 's/asn_DEF_StationId/asn_DEF_StationID/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i '' -e 's/ActionId_t/ActionID_t/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i '' -e 's/ActionId_t/ActionID_t/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i '' -e 's/asn_DEF_ActionId/asn_DEF_ActionID/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i '' -e 's/asn_DEF_ActionId/asn_DEF_ActionID/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i '' -e 's/asn_DEF_RoadSegmentReferenceId/asn_DEF_RoadSegmentReferenceID/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i '' -e 's/asn_DEF_RoadSegmentReferenceId/asn_DEF_RoadSegmentReferenceID/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i '' -e 's/asn_DEF_IntersectionReferenceId/asn_DEF_IntersectionReferenceID/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i '' -e 's/asn_DEF_IntersectionReferenceId/asn_DEF_IntersectionReferenceID/g' {} +
else
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i -e 's/NULL_t[[:space:]][[:space:]]*NULL;/NULL_t null_value;/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i -e 's/choice\.NULL/choice.null_value/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i -e 's/choice, NULL/choice, null_value/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i -e 's/StationId_t/StationID_t/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i -e 's/StationId_t/StationID_t/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i -e 's/asn_DEF_StationId/asn_DEF_StationID/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i -e 's/asn_DEF_StationId/asn_DEF_StationID/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i -e 's/ActionId_t/ActionID_t/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i -e 's/ActionId_t/ActionID_t/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i -e 's/asn_DEF_ActionId/asn_DEF_ActionID/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i -e 's/asn_DEF_ActionId/asn_DEF_ActionID/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i -e 's/asn_DEF_RoadSegmentReferenceId/asn_DEF_RoadSegmentReferenceID/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i -e 's/asn_DEF_RoadSegmentReferenceId/asn_DEF_RoadSegmentReferenceID/g' {} +
    find "$OUT_DIR" -type f -name "*.h" -exec sed -i -e 's/asn_DEF_IntersectionReferenceId/asn_DEF_IntersectionReferenceID/g' {} +
    find "$OUT_DIR" -type f -name "*.c" -exec sed -i -e 's/asn_DEF_IntersectionReferenceId/asn_DEF_IntersectionReferenceID/g' {} +
fi

echo "Generating wrapper headers for each module..."
cat << 'HEADER_EOF' > "$OUT_DIR/cam_v1/etsi_cam_v1_wrapper.h"
#include "CAM.h"
HEADER_EOF

cat << 'HEADER_EOF' > "$OUT_DIR/denm_v1/etsi_denm_v1_wrapper.h"
#include "DENM.h"
HEADER_EOF

cat << 'HEADER_EOF' > "$OUT_DIR/cam_v2/etsi_cam_v2_wrapper.h"
#include "CAM.h"
HEADER_EOF

cat << 'HEADER_EOF' > "$OUT_DIR/denm_v2/etsi_denm_v2_wrapper.h"
#include "DENM.h"
HEADER_EOF

cat << 'HEADER_EOF' > "$OUT_DIR/cpm_v2/etsi_cpm_v2_wrapper.h"
#include "CollectivePerceptionMessage.h"
HEADER_EOF

cat << 'HEADER_EOF' > "$OUT_DIR/is_v2/etsi_is_v2_wrapper.h"
#include "IVIM.h"
#include "MAPEM.h"
#include "RTCMEM.h"
#include "SPATEM.h"
#include "SREM.h"
#include "SSEM.h"
HEADER_EOF

echo "Done generating modular C sources."
