#!/bin/bash
cargo clean
cargo +nightly miri test
