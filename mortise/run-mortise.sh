#!/bin/bash

setup() {
	traces=("trace-2925703-home1" "trace-3093662-home3" "trace-3219061-home" "trace-3457194-timessquare" "trace-3458374-timessquare"
		"trace-3189663-timessquare" "trace-3199718-timessquare" "trace-3201037-timessquare" "trace-3201711-timessquare" "trace-3202253-timessquare" "trace-3203600-timessquare" "trace-3204803-timessquare" "trace-3205967-timessquare" "trace-3207521-timessquare" "trace-3208852-timessquare" "trace-3455408-timessquare" "trace-3453943-timessquare"
		"trace-3109898-bus" "trace-3114405-bus" "trace-3552192-bus" "trace-3555076-bus" "trace-2767958-taxi1" "trace-2768760-taxi3")
	RESULT_DIR="./result"
	TRACE_DIR="./traces/cellular-nyc"
	cca="mortise_copa"
	source /home/vagrant/mortise-venv/bin/activate
	LOG_LEVEL=warn python3 process-report.py &
	sleep 3s
	RUST_LOG=warn sudo -E ./target/release/manager &
	sleep 5s
	./target/release/server &
}

cleanup() {
	sudo killall manager
	sudo killall executor
	sudo killall -9 server
	sudo killall -9 client
	sudo killall python3
}

run_single_exp() {
	CONFIG_FILE=$1
	BASE_PORT=$2
	./target/release/executor --config "$CONFIG_FILE" --base-port "$BASE_PORT"
	rm "$CONFIG_FILE"
}

run_all_traces() {
	pids=()
	results_to_parse=()
	traces_to_parse=()
	trace_idx=0
	trace_finish=0
	# multitask
	MAX_CONCURRENT_EXP=12
	cur_concurrent_exp=0
	base_port=10001
	while [ "$trace_finish" -lt "${#traces[@]}" ]; do
		if [ "$trace_idx" -lt "${#traces[@]}" ] && [ "$cur_concurrent_exp" -lt "$MAX_CONCURRENT_EXP" ]; then
			trace="${traces[$trace_idx]}"
			CONFIG_FILE="./rank-$trace.toml"
			TRACE_PATH="$TRACE_DIR"/"$trace".trace
			RESULT_PATH="$RESULT_DIR/$cca/$trace"
			mkdir -p "$RESULT_PATH"
			cp exp.toml "$CONFIG_FILE"
			sed "/tcp_ca /c\tcp_ca = \"$cca\"" "$CONFIG_FILE" -i
			sed "/trace =/c\trace = \"""$TRACE_PATH""\"" "$CONFIG_FILE" -i
			sed "/result_directory/c\result_directory = \"$RESULT_PATH\"" "$CONFIG_FILE" -i
			run_single_exp "$CONFIG_FILE" "$base_port" &
			pids[cur_concurrent_exp]=$!
			trace_idx=$((trace_idx + 1))
			cur_concurrent_exp=$((cur_concurrent_exp + 1))
			base_port=$((base_port + 1000))
			results_to_parse+=("$RESULT_PATH")
			traces_to_parse+=("$trace")
		else
			for pid in "${pids[@]}"; do
				wait "$pid"
				trace_finish=$((trace_finish + 1))
			done
			cur_concurrent_exp=0
			pids=()
			results_to_parse=()
			traces_to_parse=()
		fi
	done
	echo 'Exp end'
}

echo "Run mortise file download experiments..."
cleanup
setup
run_all_traces
cleanup
echo "Run mortise test finish" && exit 1
