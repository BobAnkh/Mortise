import os
import pandas as pd
import numpy as np
import glob
import argparse
import sys
import itertools


def process_cc_data(base_path):
    # Dictionary to store final results
    cc_data = {}  # Store rct data
    cc_retrans = {}  # Store retransmission data
    cc_sizes = {}  # New: Store file size data

    cc_folders = [
        f for f in os.listdir(base_path) if os.path.isdir(os.path.join(base_path, f))
    ]

    print(f"Found CC folders: {cc_folders}")

    for cc in cc_folders:
        cc_path = os.path.join(base_path, cc)
        trace_folders = [
            f for f in os.listdir(cc_path) if os.path.isdir(os.path.join(cc_path, f))
        ]

        print(f"\nProcessing {len(trace_folders)} trace folders in {cc}...")

        all_go_owds = []
        all_retrans = []
        all_sizes = []  # New: Store all file size data for this CC

        # Store network metrics for this CC
        tputs = []
        rtts = []
        losses = []

        for trace in trace_folders:
            trace_path = os.path.join(cc_path, trace)

            # # Process frame folder
            # frames_path = os.path.join(trace_path, 'frame')
            if os.path.exists(trace_path):
                csv_files = glob.glob(os.path.join(trace_path, "*.csv"))
                for csv_file in csv_files:
                    try:
                        df = pd.read_csv(csv_file)
                        if "rct" in df.columns:
                            # Get rct data (excluding last row)
                            rct_values = [
                                max(int(i), 0) for i in df["rct"].dropna().tolist()[:-1]
                            ]

                            # Get file size data
                            size_values = []
                            if "size" in df.columns:
                                size_values = [
                                    int(i) if not pd.isna(i) else 0
                                    for i in df["size"].dropna().tolist()[:-1]
                                ]
                            else:
                                # If no size column, fill with zeros
                                size_values = [0] * len(rct_values)

                            # Get retransmission value
                            retrans_value = 0
                            if len(df) > 0:
                                last_row = df.iloc[-1]
                                if len(last_row) >= 4:  # Ensure fourth column exists
                                    try:
                                        # 38866 is the total packet numbers in one session
                                        retrans_value = (
                                            float(last_row.iloc[3]) / 38866.0 * 100.0
                                        )
                                    except (ValueError, TypeError):
                                        print(
                                            f"Warning: Unable to get valid retrans value from {csv_file}"
                                        )

                            # Create corresponding retrans values for each rct value
                            retrans_values = [retrans_value] * len(rct_values)

                            # Add to total lists
                            all_go_owds.extend(rct_values)
                            all_retrans.extend(retrans_values)
                            all_sizes.extend(size_values)  # Add file size data

                    except Exception as e:
                        print(f"Error: Problem processing {csv_file}: {str(e)}")

        # Store data
        if all_go_owds:
            cc_data[cc] = all_go_owds
            cc_retrans[cc] = all_retrans
            cc_sizes[cc] = all_sizes  # Store file size data

    # Create final DataFrame
    max_length = max(len(data) for data in cc_data.values()) if cc_data else 0
    final_df = pd.DataFrame()

    # Create three columns (rct, retrans, size) for each CC
    for cc in cc_data.keys():
        # Pad size data
        padded_sizes = cc_sizes[cc] + [float("nan")] * (max_length - len(cc_sizes[cc]))
        final_df[f"{cc}_size"] = padded_sizes

        # Pad rct data
        padded_rct = cc_data[cc] + [float("nan")] * (max_length - len(cc_data[cc]))
        final_df[f"{cc}_rct"] = padded_rct

        # Pad retrans data
        padded_retrans = cc_retrans[cc] + [float("nan")] * (
            max_length - len(cc_retrans[cc])
        )
        final_df[f"{cc}_retrans"] = padded_retrans

    return final_df


def main():
    parser = argparse.ArgumentParser(
        description="Process CC data and generate statistics"
    )
    parser.add_argument("-i", "--input", required=True, help="Input path")
    parser.add_argument("-o", "--output", required=False, help="Output path")
    args = parser.parse_args()

    base_path = args.input
    if args.output:
        output_path = args.output
        # Create output directory if it doesn't exist
        os.makedirs(output_path, exist_ok=True)
    else:
        output_path = base_path

    if not os.path.exists(base_path):
        print("Error: Specified path does not exist")
        return

    # Get the last directory name from the path
    path_suffix = os.path.basename(os.path.normpath(base_path))

    # Process data
    result_df = process_cc_data(base_path)

    if result_df.empty:
        print("Error: No data collected")
        return

    # Export options - ensure numbers are not quoted
    csv_options = {
        "index": False,
        "float_format": "%.6f",  # Control float format
        "quoting": 0,  # 0 means QUOTE_MINIMAL, add quotes only when necessary
    }

    # Save results, using the last directory name as file prefix
    result_df.to_csv(os.path.join(output_path, "stat.csv"), **csv_options)

    # Show statistics
    print("\nBasic Statistics:")
    for df, name in [(result_df, "OWD")]:
        print(f"\n{name} Statistics:")
        for column in df.columns:
            print(f"\n{column}:")
            print(f"Data points: {df[column].count()}")
            print(f"Mean: {df[column].mean():.2f}")
            print(f"95th percentile: {np.percentile(df[column].dropna(), 95):.2f}")

    print(
        f"\nProcessing complete! Results saved as {os.path.join(output_path, 'stat.csv')}"
    )


if __name__ == "__main__":
    main()
