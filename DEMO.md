# CityHall Sync: Replication Demo

This document provides a step-by-step guide to demonstrating the leader-replica replication feature of CityHall.

## 1. Prerequisites

-   Rust and Cargo are installed.
-   The project has been built with `cargo build --release`.
-   You have a terminal with at least three separate tabs or windows.

## 2. Demo Script

### Step 1: Prepare Directories and Terminals

First, let's set up clean data directories for our leader and replica nodes.

```bash
# Clean up previous demo runs
rm -rf /tmp/leader /tmp/replica1

# Create new directories
mkdir -p /tmp/leader /tmp/replica1
```

You will need three terminals:
-   **Terminal 1:** For the Leader node.
-   **Terminal 2:** For the Replica node.
-   **Terminal 3:** For the Client CLI to send commands.

### Step 2: Start the Leader

In **Terminal 1**, start the leader node. It will listen for client connections on port `7878` and for replica connections on port `7879`.

```bash
./target/release/cityhall leader --port 7878 --replication-port 7879 --data-dir /tmp/leader
```

You should see output indicating the leader has started.

### Step 3: Start the Replica

In **Terminal 2**, start the replica node. It will connect to the leader at `127.0.0.1:7879` and store its own data in `/tmp/replica1`.

```bash
./target/release/cityhall replica --leader 127.0.0.1:7879 --data-dir /tmp/replica1
```

The replica will start and immediately try to sync with the leader. Since the leader has no data yet, it will report that it is caught up.

### Step 4: Write Data to the Leader

In **Terminal 3**, use the client to write a key-value pair to the leader.

```bash
# Write 'city' = 'Nairobi' to the leader
./target/release/cityhall client --addr 127.0.0.1:7878 put city Nairobi
```

### Step 5: Verify Replication

Now, let's verify that the data was replicated to the replica node.

In **Terminal 2**, you should see the replica's logs indicating that it has synced a new segment from the leader.

In **Terminal 3**, try to read the key from the replica. Note that the replica's client port is not running a server, so we must inspect its state directly. We can do this by checking the replica's status.

```bash
# Check the replica's status
./target/release/cityhall replica status --data-dir /tmp/replica1
```

You should see that the `Last Synced Segment` and `Total Entries` have increased.

### Step 6: Demonstrate Offline Catch-Up

This is the most important part of the demo. We'll show that the replica can recover and catch up after being offline.

1.  **Stop the replica:** In **Terminal 2**, press `Ctrl+C` to stop the replica process.

2.  **Write more data to the leader:** In **Terminal 3**, write several more keys to the leader while the replica is offline.

    ```bash
    ./target/release/cityhall client --addr 127.0.0.1:7878 put country Kenya
    ./target/release/cityhall client --addr 127.0.0.1:7878 put event RustAfrica
    ```

3.  **Restart the replica:** In **Terminal 2**, start the replica again with the same command.

    ```bash
    ./target/release/cityhall replica --leader 127.0.0.1:7879 --data-dir /tmp/replica1
    ```

    Observe the logs in Terminal 2. The replica will load its previous state, realize it is behind the leader, and immediately start syncing all the segments it missed while it was offline.

### Step 7: Final Verification

Once the replica reports that it is caught up, check its status again.

In **Terminal 3**:

```bash
# Check the replica's status one last time
./target/release/cityhall replica status --data-dir /tmp/replica1
```

You will see that the `Total Entries` and `Last Synced Segment` now reflect all the writes that were made to the leader, proving that the catch-up mechanism worked correctly.

This completes the demo of CityHall's replication feature!
