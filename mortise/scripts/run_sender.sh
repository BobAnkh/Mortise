#!/bin/bash

POSITIONAL_ARGS=()
PCAP=false
while [[ $# -gt 0 ]]; do
    case $1 in
    --pcap)
        PCAP=true
        PCAP_FILE=$2
        shift # past argument
        shift # past value
        ;;
    *)
        POSITIONAL_ARGS+=("$1") # save positional arg
        shift                   # past argument
        ;;
    esac
done

set -- "${POSITIONAL_ARGS[@]}" # restore positional parameters
if [ "$PCAP" = true ]; then
    mkdir -p "$(dirname "$PCAP_FILE")"
    tcpdump -i ingress -s 96 -w "$PCAP_FILE" &
fi

dump_pid=$!
./target/release/client "$@" --workload ./workload/app.wk --connect $MAHIMAHI_BASE
ret_code=$?
if [ "$PCAP" = true ]; then
    kill $dump_pid
fi
exit $ret_code
