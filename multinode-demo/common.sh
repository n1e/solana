# |source| this file
#
# Disable complaints about unused variables in this file:
# shellcheck disable=2034

rsync=rsync
if [[ -d "$SNAP" ]]; then # Running inside a Linux Snap?
  solana_program() {
    declare program="$1"
    if [[ "$program" = wallet ]]; then
      # TODO: Merge wallet.sh functionality into solana-wallet proper and
      #       remove this special case
      printf "%s/bin/solana-%s" "$SNAP" "$program"
    else
      printf "%s/command-%s.wrapper" "$SNAP" "$program"
    fi
  }
  rsync="$SNAP"/bin/rsync
  SOLANA_METRICS_CONFIG="$(snapctl get metrics-config)"
  SOLANA_CUDA="$(snapctl get enable-cuda)"

elif [[ -n "$USE_SNAP" ]]; then # Use the Linux Snap binaries
  solana_program() {
    declare program="$1"
    printf "solana.%s" "$program"
  }
elif [[ -n "$USE_INSTALL" ]]; then # Assume |cargo install| was run
  solana_program() {
    declare program="$1"
    printf "solana-%s" "$program"
  }
  # CUDA was/wasn't selected at build time, can't affect CUDA state here
  unset SOLANA_CUDA
else
  solana_program() {
    declare program="$1"
    declare features=""
    if [[ "$program" =~ ^(.*)-cuda$ ]]; then
      program=${BASH_REMATCH[1]}
      features="--features=cuda,erasure"
    fi
    if [[ -z "$DEBUG" ]]; then
      maybe_release=--release
    fi
    printf "cargo run $maybe_release --bin solana-%s %s -- " "$program" "$features"
  }
fi

solana_client_demo=$(solana_program client-demo)
solana_wallet=$(solana_program wallet)
solana_drone=$(solana_program drone)
solana_fullnode=$(solana_program fullnode)
solana_fullnode_config=$(solana_program fullnode-config)
solana_fullnode_cuda=$(solana_program fullnode-cuda)
solana_genesis=$(solana_program genesis)
solana_mint=$(solana_program mint)

export RUST_LOG=${RUST_LOG:-solana=info} # if RUST_LOG is unset, default to info
export RUST_BACKTRACE=1


# The SOLANA_METRICS_CONFIG environment variable is formatted as a
# comma-delimited list of parameters. All parameters are optional.
#
# Example:
#   export SOLANA_METRICS_CONFIG="host=<metrics host>,db=<database name>,u=<username>,p=<password>"
#
configure_metrics() {
  [[ -n $SOLANA_METRICS_CONFIG ]] || return

  declare metrics_params
  IFS=',' read -r -a metrics_params <<< "$SOLANA_METRICS_CONFIG"
  for param in "${metrics_params[@]}"; do
    IFS='=' read -r -a pair <<< "$param"
    if [[ "${#pair[@]}" != 2 ]]; then
      echo Error: invalid metrics parameter: "$param"
    else
      declare name="${pair[0]}"
      declare value="${pair[1]}"
      case "$name" in
      host)
        export INFLUX_HOST="$value"
        echo INFLUX_HOST="$INFLUX_HOST"
        ;;
      db)
        export INFLUX_DATABASE="$value"
        echo INFLUX_DATABASE="$INFLUX_DATABASE"
        ;;
      u)
        export INFLUX_USERNAME="$value"
        echo INFLUX_USERNAME="$INFLUX_USERNAME"
        ;;
      p)
        export INFLUX_PASSWORD="$value"
        echo INFLUX_PASSWORD="********"
        ;;
      *)
        echo Error: Unknown metrics parameter name: "$name"
        ;;
      esac
    fi
  done
}
configure_metrics

tune_networking() {
  # Reference: https://medium.com/@CameronSparr/increase-os-udp-buffers-to-improve-performance-51d167bb1360
  [[ $(uname) = Linux ]] && (
    set -x
    # TODO: Check values and warn instead, it's a little rude to set them here.
    sudo sysctl -w net.core.rmem_max=26214400 1>/dev/null 2>/dev/null
    sudo sysctl -w net.core.rmem_default=26214400 1>/dev/null 2>/dev/null
  )
}

SOLANA_CONFIG_DIR=${SNAP_DATA:-$PWD}/config
SOLANA_CONFIG_PRIVATE_DIR=${SNAP_DATA:-$PWD}/config-private
SOLANA_CONFIG_CLIENT_DIR=${SNAP_USER_DATA:-$PWD}/config-client-client

rsync_url() { # adds the 'rsync://` prefix to URLs that need it
  declare url="$1"

  if [[ "$url" =~ ^.*:.*$ ]]; then
    # assume remote-shell transport when colon is present, use $url unmodified
    echo "$url"
    return
  fi

  if [[ -d "$url" ]]; then
    # assume local directory if $url is a valid directory, use $url unmodified
    echo "$url"
    return
  fi

  # Default to rsync:// URL
  echo "rsync://$url"
}
