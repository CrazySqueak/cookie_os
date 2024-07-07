#!/bin/bash
export INCLUDE_DEBUG_SYMBOLS=0
export RELEASE_BUILD=1
cd "$(dirname "$0")"
make "$@"
