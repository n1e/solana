#!/bin/bash
here=$(dirname "$0")
# shellcheck source=multinode-demo/common.sh
source "$here"/common.sh

usage() {
  if [[ -n "$1" ]]; then
    echo "$*"
    echo
  fi
  echo "usage: $0 [rsync network path to solana repo on leader machine] [network ip address of leader]"
  exit 1
}

if [[ "$1" = "-h" || -n "$3" ]]; then
  usage
fi

if [[ -d "$SNAP" ]]; then
  # Exit if mode is not yet configured
  # (typically the case after the Snap is first installed)
  [[ -n "$(snapctl get mode)" ]] || exit 0

  # Select leader from the Snap configuration
  leader_address="$(snapctl get leader-address)"
  if [[ -z "$leader_address" ]]; then
    # Assume public testnet by default
    leader_address=35.230.65.68  # testnet.solana.com
  fi
  leader="$leader_address"
else
  if [[ -n "$3" ]]; then
    usage
  fi

  if [[ -z "$1" ]]; then
    leader=${1:-${here}/..}    # Default to local solana repo
    leader_address=${2:-127.0.0.1}  # Default to local leader
  elif [[ -z "$2" ]]; then
    leader="$1"
    leader_address=$(dig +short "$1" | head -n1)
    if [[ -z "$leader_address" ]]; then
      usage "Error: unable to resolve IP address for $leader"
    fi
  else
    leader="$1"
    leader_address="$2"
  fi
fi
leader_port=8001

if [[ -n "$SOLANA_CUDA" ]]; then
  program="$solana_fullnode_cuda"
else
  program="$solana_fullnode"
fi


[[ -f "$SOLANA_CONFIG_DIR"/validator.json ]] || {
  echo "$SOLANA_CONFIG_DIR/validator.json not found, create it by running:"
  echo
  echo "  ${here}/setup.sh -t validator"
  exit 1
}

rsync_leader_url=$(rsync_url "$leader")

set -ex
SOLANA_LEADER_CONFIG_DIR="$SOLANA_CONFIG_DIR"/leader-config
rm -rf "$SOLANA_LEADER_CONFIG_DIR"
$rsync -vPrz "$rsync_leader_url"/config/ "$SOLANA_LEADER_CONFIG_DIR"
ls -lh "$SOLANA_LEADER_CONFIG_DIR"

tune_networking

# shellcheck disable=SC2086 # $program should not be quoted
exec $program \
  -l "$SOLANA_CONFIG_DIR"/validator.json -t "$leader_address:$leader_port" \
  < <(shopt -s nullglob && cat "$SOLANA_LEADER_CONFIG_DIR"/genesis.log \
          "$SOLANA_LEADER_CONFIG_DIR"/tx-*.log)
