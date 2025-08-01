# Mortise: Auto-tuning Congestion Control to Optimize QoE via Network-Aware Parameter Optimization (USENIX NSDI 2026)

A real-time, network-aware adaptation framework that dynamically and continuously tunes Congestion Control Algorithm parameters to maximize QoE in time-varying network conditions.

> **Note**: Although we are not allowed to publicly release the application modeling, CCA, and adjustment strategies used in the production environment, we have prototyped this repository to demonstrate how Mortise operates and adjusts CCA parameters to optimize QoE with emulation. We provide a file download application that closely resembles the real-world services, along with the corresponding workload traces.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Project Structure](#project-structure)
- [Environment Setup](#environment-setup)
- [Running Basic Experiments](#running-basic-experiments)
- [Reproducing File Download Emulation Experiments](#reproducing-file-download-emulation-experiments)
- [Understanding Results](#understanding-results)

## Prerequisites

- **Host System:** Linux (Ubuntu 20.04+ recommended)
- **Hardware:** At least 16GB RAM, 6 CPU cores, 50GB free disk space
- **Virtualization:** KVM/QEMU support enabled
- **Network:** Internet connection for downloading traces and dependencies

## Project Structure

```
├── scripts/                     # Install and setup scripts (-> /home/vagrant/scripts)
├── mortise/                     # Main experiment directory (-> /home/vagrant/mortise)
│   ├── scripts/                 # Evaluation scripts
│   ├── src/                     # Source code for evaluation and framework
│   ├── workload/                # Workload trace directory
│   ├── result/                  # Log and raw output directory
│   └── traces/                  # Network trace directory
├── algorithm/                   # Congestion control algorithms (-> /home/vagrant/algorithm)
│   ├── kern-mod/                # Kernel module algorithms (mvfst, copa)
│   └── bpf-kern/                # BPF-based algorithms
├── Vagrantfile                  # VM configuration

# VM-only directories:
# /home/vagrant/tools/           # Installed tools and dependencies
```

**Note:** Directories marked with `(-> path)` are automatically synced to the specified VM location.

## Environment Setup

### 1. Host Environment Setup

Run the setup script to install Vagrant, libvirt, and required plugins:

```bash
bash setup.sh
```

This script will:

- Install **Vagrant** and **libvirt**
- Install vagrant plugins: **vagrant-rsync-back** (for VM-to-host syncing)
- Add your user to _libvirt_ and _kvm_ groups

**Important:** After setup completes, you must **logout and re-login** for group changes to take effect.
And please remember to allow traffic from the VM to your host machine in your firewall.

### 2. Virtual Machine Setup

Build and provision the VM:

```bash
vagrant up
```

> **Note**: You can change the VM resources in the `Vagrantfile` if needed, such as CPU cores and memory.
> Currently, it is configured with 6 CPU cores and 16GB RAM.

This automatically runs `scripts/setup vm-new` inside the VM during provisioning.

Access the VM:

```bash
vagrant ssh
```

### 3. BPF Environment Setup

Inside the VM, setup the BPF development environment:

```bash
scripts/setup bpf
```

### 4. Mortise Environment Setup

Inside the VM, setup the Mortise experiment environment:

```bash
scripts/setup mortise
```

**NOTE**: Please logout and re-login after all the setup steps to ensure all environment variables are correctly set and privileges take effect.

### 5. Baseline Algorithm Setup

Our framework supports all congestion control algorithms that can be configured through the `TCP_CONGESTION` interface without additional socket settings.
The baselines involved in the paper can be mainly divided into three categories.

#### Category 1: Kernel Built-in Algorithms

These algorithms are included in the Linux kernel and can be enabled via `sysctl` (e.g., bbr, vegas):

```bash
# Enable BBR
sudo sysctl -w net.ipv4.tcp_congestion_control=bbr

# Enable Vegas
sudo sysctl -w net.ipv4.tcp_congestion_control=vegas

# Check available algorithms
sudo cat /proc/sys/net/ipv4/tcp_available_congestion_control
# Or
# sudo sysctl net.ipv4.tcp_available_congestion_control
```

#### Category 2: Custom Kernel Module Algorithms

These algorithms require compilation and installation as kernel modules (e.g., copa mit, copa mvfst):

```bash
# Navigate to kernel module directory
cd /home/vagrant/algorithm/kern-mod

# Compile and install copa and mvfst
make
sudo make install
```

#### Category 3: Machine Learning-based Algorithms

These algorithms use machine learning models and require specific setup procedures (e.g., Antelope, Orca).
Currently, we refer readers to the steps and guidelines provided by their official code repositories to install and run these CCAs.
After installation, they can also be run in a similar manner to the other two categories.

<!-- ```bash -->
<!-- # Navigate to ML algorithms directory -->
<!-- cd /home/vagrant/algorithm/ml-mod -->
<!-- ``` -->
<!---->
<!-- **Note:** Each ML-based algorithm directory contains detailed README files with specific installation and configuration instructions. -->

## Running Basic Experiments

### Environment Preparation

1. **Setup Python virtual environment** (inside VM `/home/vagrant`):

```bash
sudo apt -qq install -y python3-pip python3-venv
python3 -m venv mortise-venv
source mortise-venv/bin/activate
pip install numpy structlog scipy pandas
```

2. **Build the project** (inside VM `/home/vagrant/mortise`):

```bash
cd /home/vagrant/mortise
cargo build --release --all
```

Executables will be generated in `target/release/`.

### Basic Usage

Although we are not allowed to provide the full set of workloads for evaluating performance in our paper, we offer two workload traces as a demonstration.
The `workload/demo.wk` is used for testing fuckability and debugging, while `workload/app.wk` is an anonymized workload record collected from real services in our production environment.

**Workload Format**: Each workload file contains two columns of numbers representing:

- **Column 1**: Inter-request interval in milliseconds (time gap between consecutive requests)
- **Column 2**: File size in bytes (download size for each request)

Below are the examples of how you can test the basic functionality of the framework using the provided workload traces.
You can directly jump to [next section](#reproducing-file-download-emulation-experiments) to use the encapsulated evaluation scripts to run the experiments.

#### Server-Client Mode to Test General Algorithms

For evaluations of general congestion control algorithms, you can run the server-client mode as follows:

1. **Start server** (in background):

```bash
./target/release/server &
```

2. **Run client** with specific congestion control algorithm:

```bash
./target/release/client --output result.csv --congestion bbr --workload workload/demo.wk
```

This command:

- Uses BBR congestion control algorithm
- Follows the workload specification in `workload/demo.wk`
- Outputs results to `result.csv`

#### Mortise Mode to Test Mortise Framework

For Mortise-specific evaluations, two additional steps are required:

1. **Run the Python preprocessing script** (remember to activate the mortise-venv first):

```bash
python process-report.py
```

2. **Start manager** (requires privileges):

```bash
sudo ./target/release/manager
```

3. Then **run server and client applications** as described above. You should change the `--congestion` argument to `mortise_copa` to use the Mortise framework with the Copa algorithm, for example:

```bash
./target/release/server &
./target/release/client --output result.csv --congestion mortise_copa --workload workload/demo.wk
```

## Reproducing File Download Emulation Experiments

### 1. Download Network Traces

Download cellular network traces from [Cellular-Traces-NYC](https://github.com/Soheil-ab/Cellular-Traces-NYC):

```bash
cd traces/cellular-nyc
bash download.sh
```

### 2. Test Mortise Performance

Run multi-threaded parallel tests across 23 traces with given workloads:

```bash
cd /home/vagrant/mortise
bash run-mortise.sh
```

**Note:** The `run-mortise.sh` script automatically handles the complete Mortise evaluation workflow:

- Activates the Python virtual environment
- Runs `python process-report.py`
- Starts the `manager` process with appropriate privileges
- Executes parallel measurements across all 23 cellular traces
- Manages process coordination and cleanup
- Saves results to the default directory: /home/vagrant/result

### 3. Test Baseline Algorithms

> You have to wait several minutes between each run of `run-mortise.sh` or `run-baseline.sh` for server address to be released

Test baseline algorithms (e.g., Cubic or BBR):

```bash
bash run-baseline.sh bbr
```

This script runs the same workloads using standard kernel congestion control algorithms for comparison.

### 4. Results Analysis

Generate a comprehensive analysis of all results:

```bash
python stat.py -i /path/to/your/result(default: ./result)
```

This script will read all result files from experiments and generate CSV files containing:

- Completion time for each download per algorithm
- Packet loss statistics per session

### 5. Configuring Experiment Parameters

**Quick Results (Default):** The framework is configured for fast evaluation with `iteration = 1` in `exp.toml`.

**More Stable Results:** To achieve more stable and accurate results, readers should conduct the evaluation multiple times as we did in our paper. To do this, modify `exp.toml`:

```toml
iteration = 10
```

**Parallelization Configuration:**

- `task` in `exp.toml`: Controls the number of concurrent connections per experiment setting
- `MAX_CONCURRENT_EXP` in shell scripts: Controls how many different experiment configurations run simultaneously

**Expected Time for Evaluating one CCA:**

- Quick mode (1 iteration): ~30 minutes - 1 hour
- Stable mode (10 iterations): ~5-10 hours

## Understanding Results

### Basic Experiment Output Format

Individual experiment results are saved as CSV files with the following format:

**CSV Header:** `id,size,client_send,server_recv,client_recv,rct`

**Column Descriptions:**

- `id`: File sequence number
- `size`: File size in bytes
- `client_send`: Timestamp when client sends request (ns)
- `server_recv`: Timestamp when server receives request (ns)
- `client_recv`: Timestamp when client completes download (ns)
- `rct`: Request completion time in milliseconds

**Special Last Row:** `0,0,0,[retransmission_count],0,0`

- The fourth column contains the total number of retransmitted packets for the session

**Example:**

```csv
id,size,client_send,server_recv,client_recv,rct
1,1048576,1000,1020,1520,500
2,2097152,2000,2020,3220,1200
0,0,0,315,0,0
```

### Reproduction Experiment Analysis Output

The `stat.py` script generates a comparative analysis with the following CSV format:

**CSV Header:** `cca1_size,cca1_rct,cca1_retrans,cca2_size,cca2_rct,cca2_retrans,cca3_size,cca3_rct,cca3_retrans...`

**Column Pattern:** For each congestion control algorithm:

- `<algorithm>_size`: File size in bytes
- `<algorithm>_rct`: Request completion time in milliseconds
- `<algorithm>_retrans`: Number of retransmissions

**Example:**

```csv
deepcc_size,deepcc_rct,deepcc_retrans,copa_size,copa_rct,copa_retrans,bbr_size,bbr_rct,bbr_retrans,vegas_size,vegas_rct,vegas_retrans
1048576,450,5,1048576,520,8,1048576,480,6,1048576,600,12
2097152,900,10,2097152,1100,15,2097152,950,11,2097152,1300,25
```

This format enables easy comparison of algorithm performance across identical workloads and network conditions.

## FAQs

### What if my VM is crashed or unresponsive?

The easiest way to recover is to destroy and recreate the VM:

```bash
vagrant destroy -f
vagrant up
```

If this still fails, you may need to delete the VM by `virsh` manually:

```bash
# find the name of the VM
sudo virsh list --all
sudo virsh shutdown <the-name-of-the-vm>
sudo virsh undefine <the-name-of-the-vm>
```
