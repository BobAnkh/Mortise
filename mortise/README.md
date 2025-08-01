# Mortise

## Operation Design

Operations are divided into two categories: one is `ManagerOperation` which controls the manager,
and the other is `FlowOperation` which controls per flow configuration.

## Usage

First, run the python script `process-report.py` and then run the rust `manager`(in privilege) and `server`. After that, run the `client` or `executor`.
