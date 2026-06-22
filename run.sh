#!/bin/sh
#
# Use this script to run your program LOCALLY.
# Note: Changing this script WILL NOT affect how CodeCrafters runs your program.
# Learn more: https://codecrafters.io/program-interface

set -e # Exit early if any commands fail

cd "$(dirname "$0")" # Change to project root (so .env is found)

(
  cargo build --release --target-dir=/tmp/trackerrust --manifest-path Cargo.toml
)
exec /tmp/trackerrust/release/tracker "$@"
