# Use cases

MSRV: 1.67.1 (for get rid of uninlined_format_args warning)

You should first generate bindings for your kernel headers. This can be done by running:

```bash
cargo xtask codegen
```

This will generate `bindings.rs` under `mortise-common/src`.

Do remember to generate the bindings for different kernel versions, especially when on customized kernel versions.

## Operation Design

Operations are divided into two categories: one is `ManagerOperation` which controls the manager,
and the other is `FlowOperation` which controls per flow configuration.

## Usage

First, run the python script `process-report.py` and then run the rust `receiver` and `manager`(in privilege). After that, run the `sender` or `executor`.
