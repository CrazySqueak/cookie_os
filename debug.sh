#!/bin/bash
export CARGOFLAGS=">/dev/null 2>&1"
export INCLUDE_DEBUG_SYMBOLS=1
export RELEASE_BUILD=0
cd "$(dirname "$0")"
make debug
