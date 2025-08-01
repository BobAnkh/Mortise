#!/bin/bash
stationary_traces=("trace-2925703-home1" "trace-3093662-home3" "trace-3219061-home" "trace-3457194-timessquare" "trace-3458374-timessquare")
walking_traces=("trace-3189663-timessquare" "trace-3199718-timessquare" "trace-3201037-timessquare" "trace-3201711-timessquare" "trace-3202253-timessquare" "trace-3203600-timessquare" "trace-3204803-timessquare" "trace-3205967-timessquare" "trace-3207521-timessquare" "trace-3208852-timessquare" "trace-3455408-timessquare" "trace-3453943-timessquare")
driving_traces=("trace-3109898-bus" "trace-3114405-bus" "trace-3552192-bus" "trace-3555076-bus" "trace-2767958-taxi1" "trace-2768760-taxi3")

for file in "${stationary_traces[@]}"; do
	wget -q -O "$file".trace https://raw.githubusercontent.com/Soheil-ab/Cellular-Traces-NYC/master/"$file"
	echo Downloaded "$file"
done

for file in "${walking_traces[@]}"; do
	wget -q -O "$file".trace https://raw.githubusercontent.com/Soheil-ab/Cellular-Traces-NYC/master/"$file"
	echo Downloaded "$file"
done

for file in "${driving_traces[@]}"; do
	wget -q -O "$file".trace https://raw.githubusercontent.com/Soheil-ab/Cellular-Traces-NYC/master/"$file"
	echo Downloaded "$file"
done
