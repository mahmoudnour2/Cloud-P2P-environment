#!/bin/bash

# Number of instances to run
NUM_INSTANCES=10

# Path to your Rust program's executable
EXECUTABLE="./target/debug/client"

# Start multiple instances in parallel
for i in $(seq 1 $NUM_INSTANCES); do
    gnome-terminal -- bash -c "cargo run" &
done

# Wait for all background processes to finish
wait

