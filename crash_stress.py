#!/usr/bin/env python3
"""Crash/WAL stress tester for IronBase.

Default mode runs the controller that alternates between clean commits and
simulated crashes. The worker mode (invoked internally) performs the actual
operations. Run `python crash_stress.py --iterations 10` to execute.
"""

import argparse
import os
import random
import signal
import subprocess
import sys
import time

from ironbase import IronBase

DEFAULT_DB = "stress_crash.mlite"


def cleanup(db_path: str):
    for ext in (".mlite", ".wal"):
        target = db_path.replace(".mlite", ext)
        if os.path.exists(target):
            os.remove(target)


def count_documents(db_path: str) -> int:
    db = IronBase(db_path)
    col = db.collection("stress")
    count = col.count_documents({})
    db.close()
    return count


def run_controller(args):
    db_path = args.db
    cleanup(db_path)
    expected_count = 0
    ops_commit = args.ops_commit
    ops_crash = args.ops_crash

    print("=== Crash/WAL Stress Test ===")
    print(f"Database path : {db_path}")
    print(f"Commit ops    : {ops_commit}")
    print(f"Crash ops     : {ops_crash}")
    print(f"Iterations    : {args.iterations}")
    print()

    for iteration in range(1, args.iterations + 1):
        mode = "commit" if iteration % 2 == 1 else "crash"
        print(f"Iteration {iteration:02d} - mode: {mode.upper()}")

        if mode == "commit":
            start_value = expected_count
            cmd = [
                sys.executable,
                __file__,
                "--worker",
                "commit",
                "--db",
                db_path,
                "--ops",
                str(ops_commit),
                "--start",
                str(start_value),
            ]
            result = subprocess.run(cmd)
            if result.returncode != 0:
                print("  ✗ Commit worker failed")
                break
            expected_count += ops_commit
        else:  # crash mode
            cmd = [
                sys.executable,
                __file__,
                "--worker",
                "crash",
                "--db",
                db_path,
                "--ops",
                str(ops_crash),
            ]
            proc = subprocess.Popen(cmd)
            time.sleep(random.uniform(0.05, 0.2))
            proc.send_signal(signal.SIGKILL)
            proc.wait()
            print("  ⚡ Worker killed to simulate crash")

        time.sleep(0.2)
        actual = count_documents(db_path)
        print(f"  Documents in DB: {actual}, expected: {expected_count}")

        if actual != expected_count:
            print("  ✗ MISMATCH – possible WAL/catalog bug!")
            break
        else:
            print("  ✓ State consistent")

    print("\nCleaning up...")
    cleanup(db_path)


def run_worker(args):
    db = IronBase(args.db)
    col = db.collection("stress")

    if args.worker == "commit":
        base = args.start
        for i in range(args.ops):
            col.insert_one({
                "marker": "commit",
                "seq": base + i,
            })
        db.close()
        sys.exit(0)

    # crash worker: run everything inside a transaction and never commit
    tx_id = db.begin_transaction()
    for _ in range(args.ops):
        db.insert_one_tx(
            "stress",
            {"marker": "crash", "value": random.randint(0, 1_000_000)},
            tx_id,
        )
        time.sleep(0.01)
    print("  [crash-worker] inserted ops inside tx, waiting for kill...")
    while True:
        time.sleep(1)


def parse_args():
    parser = argparse.ArgumentParser(description="IronBase crash/WAL stress tester")
    parser.add_argument("--db", default=DEFAULT_DB, help="Database path (default: %(default)s)")
    parser.add_argument("--iterations", type=int, default=20, help="Total iterations (default: %(default)s)")
    parser.add_argument("--ops-commit", type=int, default=200, help="Insert operations per commit iteration")
    parser.add_argument("--ops-crash", type=int, default=50, help="Insert operations per crash iteration")
    parser.add_argument("--worker", choices=["commit", "crash"], help=argparse.SUPPRESS)
    parser.add_argument("--start", type=int, default=0, help=argparse.SUPPRESS)
    parser.add_argument("--ops", type=int, default=0, help=argparse.SUPPRESS)
    return parser.parse_args()


def main():
    args = parse_args()
    if args.worker:
        run_worker(args)
    else:
        run_controller(args)


if __name__ == "__main__":
    main()
